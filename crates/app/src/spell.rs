//! Spellchecking for the composer, with Hunspell dictionaries read by
//! spellbook. The system's dictionary for the user's language is used when
//! there is one (`hunspell-en-gb` and the like), and the bundled en_US one
//! otherwise. Words added to the dictionary are kept in
//! `~/.config/bluebubbles-gpui/dictionary.txt`, one per line; ignored
//! words last until the app quits, as Ignore Spelling does on the Mac.

use crate::config;
use gpui_kit::{App, AppContext};
use spellbook::Dictionary;
use std::collections::HashSet;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};
use unicode_segmentation::UnicodeSegmentation;

const BUNDLED_AFF: &str = include_str!("../../../assets/dictionaries/en_US.aff");
const BUNDLED_DIC: &str = include_str!("../../../assets/dictionaries/en_US.dic");

/// Where distributions put Hunspell dictionaries.
const SYSTEM_DIRS: &[&str] = &[
    "/usr/share/hunspell",
    "/usr/share/myspell",
    "/usr/share/myspell/dicts",
    "/run/current-system/sw/share/hunspell",
];

/// Shorthand people text that the dictionary doesn't have.
const CHAT_WORDS: &[&str] = &[
    "lol", "lmao", "lmk", "omg", "btw", "idk", "imo", "imho", "tbh", "smh", "fyi", "brb", "ttyl",
    "thx", "ty", "np", "nvm", "jk", "irl", "ok", "okay", "haha", "hahaha", "hehe", "ya", "yep",
    "yup", "nope", "gonna", "wanna", "gotta", "kinda", "sorta", "ugh", "hmm", "hmmm", "omw", "rn",
    "tho", "pls", "plz", "bday", "selfie", "emoji", "emojis", "iMessage",
];

/// Most suggestions offered, as on the Mac.
const MAX_SUGGESTIONS: usize = 5;

pub struct Speller {
    dict: Dictionary,
    ignored: HashSet<String>,
    /// Where learned words are saved; `None` in tests.
    personal: Option<PathBuf>,
}

static SPELLER: OnceLock<RwLock<Speller>> = OnceLock::new();
/// Bumped when the set of accepted words changes, so views recheck.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Loads the dictionary off the UI thread; checking starts once it's in.
pub fn init(cx: &mut App) {
    let loaded =
        cx.background_spawn(async { load(Some(config::config_dir()), language().as_deref()) });
    cx.spawn(async move |cx| {
        if let Some(speller) = loaded.await {
            let _ = SPELLER.set(RwLock::new(speller));
            GENERATION.fetch_add(1, Ordering::Relaxed);
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

/// Changes whenever a word is learned or ignored.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn with<T>(f: impl FnOnce(&Speller) -> T) -> Option<T> {
    let lock = SPELLER.get()?;
    Some(f(&lock.read().unwrap_or_else(|e| e.into_inner())))
}

fn with_mut<T>(f: impl FnOnce(&mut Speller) -> T) -> Option<T> {
    let lock = SPELLER.get()?;
    let out = f(&mut lock.write().unwrap_or_else(|e| e.into_inner()));
    GENERATION.fetch_add(1, Ordering::Relaxed);
    Some(out)
}

/// The misspelled words of `text`; empty until the dictionary is loaded.
pub fn misspelled(text: &str, typing_at: Option<usize>) -> Vec<Range<usize>> {
    with(|s| s.misspelled(text, typing_at)).unwrap_or_default()
}

pub fn suggestions(word: &str) -> Vec<String> {
    with(|s| s.suggestions(word)).unwrap_or_default()
}

pub fn ignore(word: &str) {
    with_mut(|s| s.ignore(word));
}

pub fn learn(word: &str) {
    with_mut(|s| s.learn(word));
}

/// `lang` picks a system dictionary; en_US and `None` use the bundled one.
fn load(config_dir: Option<PathBuf>, lang: Option<&str>) -> Option<Speller> {
    let started = std::time::Instant::now();
    let system = lang.filter(|l| *l != "en_US").and_then(system_dictionary);
    let (dict, source) = match system {
        Some((path, aff, dic)) => match Dictionary::new(&aff, &dic) {
            Ok(dict) => (dict, path.display().to_string()),
            Err(e) => {
                tracing::warn!("can't read the dictionary at {}: {e}", path.display());
                (bundled()?, "the bundled en_US dictionary".to_owned())
            }
        },
        None => (bundled()?, "the bundled en_US dictionary".to_owned()),
    };
    let personal = config_dir.map(|d| d.join("dictionary.txt"));
    let mut speller = Speller {
        dict,
        ignored: HashSet::new(),
        personal: None,
    };
    for word in CHAT_WORDS {
        let _ = speller.dict.add(word);
    }
    if let Some(path) = &personal
        && let Ok(words) = std::fs::read_to_string(path)
    {
        for word in words.lines().map(str::trim).filter(|w| !w.is_empty()) {
            let _ = speller.dict.add(word);
        }
    }
    speller.personal = personal;
    tracing::info!(
        "spellcheck: {source}, loaded in {} ms",
        started.elapsed().as_millis()
    );
    Some(speller)
}

fn bundled() -> Option<Dictionary> {
    Dictionary::new(BUNDLED_AFF, BUNDLED_DIC)
        .inspect_err(|e| tracing::error!("the bundled dictionary doesn't parse: {e}"))
        .ok()
}

/// `en_GB` from `LC_ALL`, `LC_MESSAGES` or `LANG` (`en_GB.UTF-8`).
fn language() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .and_then(|v| {
            let lang = v.split(['.', '@']).next()?.to_owned();
            (lang != "C" && lang != "POSIX").then_some(lang)
        })
}

fn system_dictionary(lang: &str) -> Option<(PathBuf, String, String)> {
    SYSTEM_DIRS.iter().find_map(|dir| {
        let base = Path::new(dir).join(lang);
        let aff = std::fs::read_to_string(base.with_extension("aff")).ok()?;
        let dic = std::fs::read_to_string(base.with_extension("dic")).ok()?;
        Some((base.with_extension("dic"), aff, dic))
    })
}

impl Speller {
    /// The byte ranges of misspelled words. The word touching `typing_at`
    /// (the caret) is skipped, so a word isn't marked while it's typed.
    pub fn misspelled(&self, text: &str, typing_at: Option<usize>) -> Vec<Range<usize>> {
        let skip = skipped_spans(text);
        text.unicode_word_indices()
            .map(|(start, word)| (start..start + word.len(), word))
            .filter(|(range, _)| !typing_at.is_some_and(|c| range.start <= c && c <= range.end))
            .filter(|(range, _)| {
                !skip
                    .iter()
                    .any(|s| s.start <= range.start && range.end <= s.end)
            })
            .filter(|(_, word)| checkable(word))
            .filter(|(_, word)| !self.accepts(word))
            .map(|(range, _)| range)
            .collect()
    }

    fn accepts(&self, word: &str) -> bool {
        let word = normalize(word);
        self.ignored.contains(&word) || self.dict.check(&word)
    }

    pub fn suggestions(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.dict.suggest(&normalize(word), &mut out);
        out.truncate(MAX_SUGGESTIONS);
        out
    }

    pub fn ignore(&mut self, word: &str) {
        self.ignored.insert(normalize(word));
    }

    /// Accepts `word` from now on and saves it for later runs.
    pub fn learn(&mut self, word: &str) {
        let word = normalize(word);
        if let Err(e) = self.dict.add(&word) {
            tracing::warn!("can't add {word:?} to the dictionary: {e}");
            return;
        }
        if let Some(path) = &self.personal {
            let saved = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                })
                .and_then(|mut f| writeln!(f, "{word}"));
            if let Err(e) = saved {
                tracing::warn!("saving {}: {e}", path.display());
            }
        }
    }
}

/// Curly apostrophes, which iPhones type, as the dictionary spells them.
fn normalize(word: &str) -> String {
    word.replace('\u{2019}', "'")
}

/// Worth checking: two or more letters and no digits.
fn checkable(word: &str) -> bool {
    word.chars().filter(|c| c.is_alphabetic()).count() >= 2
        && !word.chars().any(|c| c.is_numeric() || c == '_')
}

/// Links, email addresses, @mentions, #tags and :emoji: shortcodes,
/// which aren't words.
fn skipped_spans(text: &str) -> Vec<Range<usize>> {
    text.split_whitespace()
        .filter(|t| {
            t.contains("://")
                || t.starts_with("www.")
                || t.contains('@')
                || t.starts_with('#')
                || t.starts_with(':')
                || (t.contains('.') && t.contains('/'))
        })
        .map(|t| {
            let start = t.as_ptr() as usize - text.as_ptr() as usize;
            start..start + t.len()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speller() -> Speller {
        load(None, None).expect("the bundled dictionary loads")
    }

    fn marked<'a>(s: &Speller, text: &'a str, typing_at: Option<usize>) -> Vec<&'a str> {
        s.misspelled(text, typing_at)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn marks_misspelled_words_only() {
        let s = speller();
        assert_eq!(
            marked(&s, "I recieved teh package, didn't you?", None),
            ["recieved", "teh"]
        );
        // Curly apostrophes, capitals and chat shorthand are fine.
        assert!(marked(&s, "Don\u{2019}t worry, lol. See you in Boston", None).is_empty());
    }

    #[test]
    fn leaves_links_numbers_and_mentions_alone() {
        let s = speller();
        let text = "see https://exmaple.com/foo or www.qwzx.org, mail qwzx@exmaple.com, \
                    @qwzxy #qwzxtag :qwzx_face: :qwzx at 5pm in 3rd";
        assert!(
            marked(&s, text, None).is_empty(),
            "{:?}",
            marked(&s, text, None)
        );
    }

    #[test]
    fn the_word_being_typed_isnt_marked() {
        let s = speller();
        let text = "teh recieve";
        assert_eq!(marked(&s, text, Some(text.len())), ["teh"]);
        assert_eq!(marked(&s, text, Some(2)), ["recieve"]);
        assert_eq!(marked(&s, text, None), ["teh", "recieve"]);
    }

    #[test]
    fn suggests_the_likely_word_first() {
        let s = speller();
        let suggestions = s.suggestions("recieve");
        assert_eq!(suggestions.first().map(String::as_str), Some("receive"));
        assert!(suggestions.len() <= MAX_SUGGESTIONS);
    }

    #[test]
    fn ignored_and_learned_words_are_accepted() {
        let dir = std::env::temp_dir().join(format!("bb-spell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = load(Some(dir.clone()), None).unwrap();
        assert_eq!(marked(&s, "qwzx and flurbo", None), ["qwzx", "flurbo"]);
        s.ignore("qwzx");
        s.learn("flurbo");
        assert!(marked(&s, "qwzx and flurbo", None).is_empty());

        // Learned words come back on the next run; ignored ones don't.
        let again = load(Some(dir.clone()), None).unwrap();
        assert_eq!(marked(&again, "qwzx and flurbo", None), ["qwzx"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
