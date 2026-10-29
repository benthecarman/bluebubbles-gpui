//! Emoji shortcodes in the composer, as in Slack or GitHub: `:joy:` turns
//! into 😂 when its closing colon is typed, and `:jo` offers the emoji
//! whose shortcodes match. Shortcodes are GitHub's (gemoji).

use emojis::{Emoji, Group, UnicodeVersion};
use std::ops::Range;

/// Letters typed after the colon before suggestions show, so `:)` and
/// `:D` stay emoticons.
const MIN_QUERY: usize = 2;
/// The newest emoji the bundled Noto Color Emoji draws.
const NEWEST: UnicodeVersion = UnicodeVersion::new(17, 0);

/// One suggestion: the emoji and the shortcode that matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Suggestion {
    pub emoji: &'static str,
    pub shortcode: &'static str,
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// A colon starts a shortcode at the start of the text or after a space
/// or bracket, so `10:30`, `http://` and `Re:` don't.
fn opens_shortcode(text: &str, colon: usize) -> bool {
    text[..colon]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\''))
}

/// The `:name` being typed just before `caret`: its range (colon
/// included) and the name, once it's long enough to look up.
pub fn query_at(text: &str, caret: usize) -> Option<(Range<usize>, &str)> {
    let before = text.get(..caret)?;
    let colon = before.rfind(':')?;
    let name = &before[colon + 1..];
    (name.len() >= MIN_QUERY && name.chars().all(name_char) && opens_shortcode(text, colon))
        .then_some((colon..caret, name))
}

/// A whole `:name:` ending at `caret`, if `name` is a shortcode: its range
/// and the emoji to put there.
pub fn completed_at(text: &str, caret: usize) -> Option<(Range<usize>, &'static str)> {
    let before = text.get(..caret)?.strip_suffix(':')?;
    let colon = before.rfind(':')?;
    let name = &before[colon + 1..];
    if name.is_empty() || !name.chars().all(name_char) || !opens_shortcode(text, colon) {
        return None;
    }
    let emoji = emojis::get_by_shortcode(&name.to_ascii_lowercase()).filter(|e| drawable(e))?;
    Some((colon..caret, emoji.as_str()))
}

fn drawable(emoji: &Emoji) -> bool {
    emoji.unicode_version() <= NEWEST
}

/// Emoji whose shortcodes match `query`: whole matches first, then ones
/// that start with it, shortest first (`:hea` gives `heart` before
/// `heartbeat`), then ones with a later word that does (`:cat` finds
/// `joy_cat`). Flags come after the rest of their group, so `:jo` gives
/// 😂 and 😹 before 🇯🇴. Ties go in the usual emoji order. One shortcode
/// per emoji.
pub fn suggestions(query: &str, limit: usize) -> Vec<Suggestion> {
    let query = query.to_ascii_lowercase();
    let mut found: Vec<((u8, bool, usize), usize, Suggestion)> = emojis::iter()
        .filter(|e| drawable(e))
        .enumerate()
        .filter_map(|(order, emoji)| {
            emoji
                .shortcodes()
                .filter_map(|code| rank(code, &query).map(|r| (r, code)))
                .min_by_key(|&(r, _)| r)
                .map(|((group, len), code)| ((group, emoji.group() == Group::Flags, len), code))
                .map(|(r, shortcode)| {
                    let emoji = emoji.as_str();
                    (r, order, Suggestion { emoji, shortcode })
                })
        })
        .collect();
    found.sort_by_key(|&(r, order, _)| (r, order));
    found.into_iter().take(limit).map(|(_, _, s)| s).collect()
}

fn rank(code: &str, query: &str) -> Option<(u8, usize)> {
    if code == query {
        Some((0, 0))
    } else if code.starts_with(query) {
        Some((1, code.len()))
    } else if code.split(['_', '-']).skip(1).any(|w| w.starts_with(query)) {
        Some((2, 0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closing_colon_makes_the_emoji() {
        let text = "that's great :joy:";
        assert_eq!(completed_at(text, text.len()), Some((13..text.len(), "😂")));
        assert_eq!(completed_at(":+1:", 4), Some((0..4, "👍")));
        assert_eq!(completed_at("(:Heart:", 8), Some((1..8, "❤️")));
        // Not a shortcode, not closed, or not at a word start.
        assert_eq!(completed_at(":notanemoji:", 12), None);
        assert_eq!(completed_at(":joy", 4), None);
        assert_eq!(completed_at("x:joy:", 6), None);
        assert_eq!(completed_at("::", 2), None);
    }

    #[test]
    fn suggests_while_a_shortcode_is_typed() {
        assert_eq!(query_at("ok :jo", 6), Some((3..6, "jo")));
        assert_eq!(query_at(":thumbs", 7), Some((0..7, "thumbs")));
        // Too short, finished, or not a shortcode at all.
        assert_eq!(query_at("ok :j", 5), None);
        assert_eq!(query_at(":)", 2), None);
        assert_eq!(query_at("at 10:30", 8), None);
        assert_eq!(query_at("https://example.com", 19), None);
        assert_eq!(query_at(":jo ", 4), None);
        // Only the text before the caret counts.
        assert_eq!(query_at(":joy later", 4), Some((0..4, "joy")));
    }

    #[test]
    fn whole_and_prefix_matches_come_first() {
        let codes = |q| {
            suggestions(q, 6)
                .into_iter()
                .map(|s| s.shortcode)
                .collect::<Vec<_>>()
        };
        assert_eq!(codes("joy")[0], "joy");
        assert!(codes("joy").contains(&"joy_cat"));
        assert_eq!(suggestions("heart", 3)[0].emoji, "❤️");
        assert_eq!(codes("hea")[0], "heart");
        assert_eq!(codes("jo")[..2], ["joy", "joy_cat"]);
        // A later word in a shortcode matches too.
        assert!(codes("cat").iter().any(|c| c.ends_with("_cat")));
        assert!(suggestions("zzzz", 6).is_empty());
        assert_eq!(suggestions("smile", 4).len(), 4);
    }
}
