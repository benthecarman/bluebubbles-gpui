//! Turns a chat's messages into what the transcript draws: timestamp
//! separators, group events, bubble runs with tails, sender names in
//! groups, the receipt under your latest message, and each bubble's
//! photos and files, link preview, tapbacks and quoted reply.
//!
//! Pure data, so the layout rules are unit tested without a window.

use super::time;
use bb_core::rich_link::{self, URL_BALLOON};
use bb_core::{Names, Reaction, tapback};
use bb_store::{AttachmentRow, MessageRow};
use chrono::{DateTime, TimeZone};
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

/// A gap this long starts a new timestamp separator.
const SEPARATOR_GAP: i64 = 30 * 60_000;
/// A run of bubbles breaks (and gets its tail) after this long.
const RUN_GAP: i64 = 60_000;
/// Up to this many emoji alone show large without a bubble.
const JUMBO_MAX: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    IMessage,
    Sms,
    Received,
}

/// Stable identity for list diffing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Separator(i64),
    Event(i64),
    Message(i64),
    Typing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Receipt {
    /// "Delivered", "Read 9:41 AM", "Sent as Text Message".
    Status(String),
    /// The message didn't go out; clicking retries.
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bubble {
    pub row: MessageRow,
    pub kind: Kind,
    /// Last of its run: draws the tail (and the avatar in groups).
    pub tail: bool,
    /// First of its run: more space above, and the sender's name in
    /// groups.
    pub first: bool,
    pub sender_name: Option<String>,
    /// Group chats leave room for avatars beside incoming bubbles.
    pub avatar_slot: bool,
    /// The text, or `None` when there is none to show.
    pub text: Option<String>,
    /// Only emoji, few enough to draw large without a bubble.
    pub jumbo: bool,
    pub receipt: Option<Receipt>,
    /// Photos, videos, audio and files, in order.
    pub media: Vec<Media>,
    pub link: Option<Link>,
    /// Standing tapbacks, oldest first.
    pub reactions: Vec<Reaction>,
    /// The message this one replies to.
    pub quote: Option<Quote>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub kind: MediaKind,
    pub attachment: AttachmentRow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub url: String,
    pub title: Option<String>,
    /// "example.com".
    pub host: String,
    /// The preview image the sender's device attached.
    pub image: Option<AttachmentRow>,
    /// The sender's device is still fetching the page.
    pub loading: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    /// "You", or the sender's short name.
    pub sender: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Separator { day: String, time: String },
    Event(String),
    Bubble(Box<Bubble>),
    Typing { avatar_slot: bool },
}

pub struct Transcript {
    pub items: Vec<Item>,
    pub keys: Vec<Key>,
}

fn same_sender(a: &MessageRow, b: &MessageRow) -> bool {
    a.is_from_me == b.is_from_me && (a.is_from_me || a.sender == b.sender)
}

fn is_event(m: &MessageRow) -> bool {
    m.item_type != 0 || m.date_retracted.is_some()
}

fn kind(m: &MessageRow, chat_service: &str) -> Kind {
    if !m.is_from_me {
        return Kind::Received;
    }
    let service = m.service.as_deref().unwrap_or(chat_service);
    if service.eq_ignore_ascii_case("SMS") || service.eq_ignore_ascii_case("RCS") {
        Kind::Sms
    } else {
        Kind::IMessage
    }
}

/// Text without the object-replacement characters attachments leave.
fn visible_text(m: &MessageRow) -> Option<String> {
    let t: String = m
        .text
        .as_deref()?
        .chars()
        .filter(|&c| c != '\u{fffc}')
        .collect();
    let t = t.trim_matches(|c: char| c.is_whitespace());
    (!t.is_empty()).then(|| t.to_owned())
}

fn is_emoji_grapheme(g: &str) -> bool {
    let Some(first) = g.chars().next() else {
        return false;
    };
    let c = first as u32;
    let pictographic = matches!(c,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF | 0x2B00..=0x2BFF
        | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x2190..=0x21FF | 0x2934 | 0x2935);
    let keycap = g.contains('\u{20E3}');
    let presentation =
        g.contains('\u{FE0F}') && (c == 0xA9 || c == 0xAE || c == 0x203C || c == 0x2049);
    pictographic || keycap || presentation
}

/// Whether text is only emoji, and few enough to draw large.
pub fn jumbo(text: &str) -> bool {
    let graphemes: Vec<&str> = text
        .graphemes(true)
        .filter(|g| !g.trim().is_empty())
        .collect();
    !graphemes.is_empty()
        && graphemes.len() <= JUMBO_MAX
        && !text
            .chars()
            .any(|c| c.is_ascii_alphanumeric() && !text.contains('\u{20E3}'))
        && graphemes.iter().all(|g| is_emoji_grapheme(g))
}

pub struct Context<'a, Tz: TimeZone> {
    pub names: &'a Names,
    pub is_group: bool,
    /// `iMessage`, `SMS`, `RCS`; `any` on macOS 26.
    pub chat_service: &'a str,
    pub now: DateTime<Tz>,
    pub to_local: &'a dyn Fn(i64) -> DateTime<Tz>,
    pub typing: bool,
    /// Tapbacks and stickers, by the GUID of the message they're on.
    pub associated: &'a HashMap<String, Vec<MessageRow>>,
    /// Messages replied to, by GUID (they may be outside the page).
    pub originals: &'a HashMap<String, MessageRow>,
}

/// What an attachment is, from its MIME type, else its UTI or name.
pub fn media_kind(a: &AttachmentRow) -> MediaKind {
    let mime = a.mime_type.as_deref().unwrap_or_default();
    let uti = a.uti.as_deref().unwrap_or_default();
    let ext = a
        .transfer_name
        .as_deref()
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if mime.starts_with("image/")
        || matches!(
            uti,
            "public.jpeg" | "public.png" | "public.heic" | "com.compuserve.gif"
        )
        || matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "heic" | "heif" | "gif" | "webp" | "tif" | "tiff"
        )
    {
        MediaKind::Image
    } else if mime.starts_with("video/")
        || matches!(uti, "com.apple.quicktime-movie" | "public.mpeg-4")
        || matches!(ext.as_str(), "mov" | "mp4" | "m4v")
    {
        MediaKind::Video
    } else if mime.starts_with("audio/")
        || uti == "com.apple.coreaudio-format"
        || matches!(ext.as_str(), "caf" | "mp3" | "m4a" | "wav" | "aac" | "amr")
    {
        MediaKind::Audio
    } else {
        MediaKind::File
    }
}

/// Data a Messages extension (a link preview, an app) sent along; not
/// something to show as a file.
fn is_plugin_payload(a: &AttachmentRow) -> bool {
    a.transfer_name
        .as_deref()
        .is_some_and(|n| n.contains("pluginPayloadAttachment"))
}

/// The links in a message's text, as byte ranges and the URL each
/// opens: words starting with `http://`, `https://` or `www.`, without
/// the punctuation that ends a sentence.
pub fn links(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                let word = text[s..i]
                    .trim_end_matches(['.', ',', '!', '?', ')', ']', '\'', '"', '”', ':', ';']);
                let lower = word.to_ascii_lowercase();
                let url = if lower.starts_with("http://") || lower.starts_with("https://") {
                    Some(word.to_owned())
                } else if lower.starts_with("www.") && word.len() > 4 {
                    Some(format!("https://{word}"))
                } else {
                    None
                };
                if let Some(url) = url.filter(|u| u.contains('.')) {
                    out.push((s..s + word.len(), url));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn looks_like_url(text: &str) -> bool {
    let t = text.trim();
    !t.contains(char::is_whitespace)
        && (t.starts_with("http://") || t.starts_with("https://") || t.contains('.'))
}

fn link(m: &MessageRow, text: Option<&str>) -> Option<Link> {
    if m.balloon_bundle_id.as_deref() != Some(URL_BALLOON) {
        return None;
    }
    let rich = m.payload_data.as_ref().and_then(rich_link::parse);
    let url = rich
        .as_ref()
        .and_then(|r| r.url.clone())
        .or_else(|| text.filter(|t| looks_like_url(t)).map(str::to_owned))?;
    let image = m
        .attachments
        .iter()
        .filter(|a| is_plugin_payload(a))
        .max_by_key(|a| a.total_bytes.unwrap_or(0))
        .cloned();
    let host = rich_link::display_host(&url);
    Some(Link {
        title: rich.as_ref().and_then(|r| r.title.clone()),
        loading: rich.as_ref().is_some_and(|r| r.placeholder),
        image,
        host,
        url,
    })
}

fn reactions_on(m: &MessageRow, associated: &HashMap<String, Vec<MessageRow>>) -> Vec<Reaction> {
    let Some(on) = associated.get(&m.guid) else {
        return Vec::new();
    };
    let mut parts: Vec<i64> = on.iter().map(|a| a.associated_part.unwrap_or(0)).collect();
    parts.sort_unstable();
    parts.dedup();
    parts
        .into_iter()
        .flat_map(|part| tapback::reactions(on, part))
        .collect()
}

pub fn build<Tz: TimeZone>(rows: &[MessageRow], cx: &Context<'_, Tz>) -> Transcript
where
    Tz::Offset: std::fmt::Display,
{
    let mut items = Vec::new();
    let mut keys = Vec::new();
    let receipt_owner = rows
        .iter()
        .rev()
        .find(|m| m.is_from_me && !is_event(m) && !m.is_failed())
        .map(|m| m.id);
    let mut prev: Option<&MessageRow> = None;
    for (i, m) in rows.iter().enumerate() {
        let new_section = prev.is_none_or(|p| m.date_created - p.date_created > SEPARATOR_GAP);
        if new_section {
            let (day, time) = time::separator(&(cx.to_local)(m.date_created), &cx.now);
            keys.push(Key::Separator(m.id));
            items.push(Item::Separator { day, time });
        }
        if is_event(m) {
            let text = cx
                .names
                .group_event(m)
                .unwrap_or_else(|| cx.names.preview(m));
            if !text.is_empty() {
                keys.push(Key::Event(m.id));
                items.push(Item::Event(text));
            }
            prev = Some(m);
            continue;
        }
        let continues = !new_section
            && prev.is_some_and(|p| {
                !is_event(p) && same_sender(p, m) && m.date_created - p.date_created <= RUN_GAP
            });
        let next = rows.get(i + 1);
        let next_continues = next.is_some_and(|n| {
            !is_event(n) && same_sender(m, n) && n.date_created - m.date_created <= RUN_GAP
        });
        let mut text = visible_text(m);
        let link = link(m, text.as_deref());
        // The card shows the link; a bare URL needs no bubble as well.
        if link.is_some() && text.as_deref().is_some_and(looks_like_url) {
            text = None;
        }
        let media: Vec<Media> = m
            .attachments
            .iter()
            .filter(|a| !a.hidden && !is_plugin_payload(a))
            .map(|a| Media {
                kind: media_kind(a),
                attachment: a.clone(),
            })
            .collect();
        let jumbo = text.as_deref().is_some_and(jumbo) && media.is_empty();
        let quote = m
            .thread_originator_guid
            .as_ref()
            .and_then(|g| cx.originals.get(g))
            .map(|o| Quote {
                sender: match &o.sender {
                    _ if o.is_from_me => "You".into(),
                    Some(s) => cx.names.short(s),
                    None => String::new(),
                },
                text: cx.names.preview(o),
            });
        let receipt = if m.is_from_me && m.is_failed() {
            Some(Receipt::Failed)
        } else if Some(m.id) == receipt_owner && !m.is_pending() {
            status(m, cx)
        } else {
            None
        };
        // A receipt or a failure ends the run visually, so it keeps its tail.
        let tail = !next_continues || receipt.is_some();
        let sender_name = (cx.is_group && !m.is_from_me && !continues)
            .then(|| m.sender.as_deref().map(|s| cx.names.short(s)))
            .flatten();
        keys.push(Key::Message(m.id));
        items.push(Item::Bubble(Box::new(Bubble {
            kind: kind(m, cx.chat_service),
            tail,
            first: !continues,
            sender_name,
            avatar_slot: cx.is_group && !m.is_from_me,
            text,
            jumbo,
            receipt,
            media,
            link,
            reactions: reactions_on(m, cx.associated),
            quote,
            row: m.clone(),
        })));
        prev = Some(m);
    }
    if cx.typing {
        keys.push(Key::Typing);
        items.push(Item::Typing {
            avatar_slot: cx.is_group,
        });
    }
    Transcript { items, keys }
}

/// The status under your latest message. Read receipts exist only in
/// iMessage DMs; groups and SMS say Delivered at most.
fn status<Tz: TimeZone>(m: &MessageRow, cx: &Context<'_, Tz>) -> Option<Receipt>
where
    Tz::Offset: std::fmt::Display,
{
    let kind = kind(m, cx.chat_service);
    if let Some(read) = m
        .date_read
        .filter(|_| !cx.is_group && kind == Kind::IMessage)
    {
        return Some(Receipt::Status(time::receipt(
            "Read",
            &(cx.to_local)(read),
            &cx.now,
        )));
    }
    if m.date_delivered.is_some() {
        return Some(Receipt::Status("Delivered".into()));
    }
    if kind == Kind::Sms {
        return Some(Receipt::Status("Sent as Text Message".into()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    const MIN: i64 = 60_000;

    fn msg(id: i64, from: Option<&str>, text: &str, date: i64) -> MessageRow {
        MessageRow {
            id,
            guid: format!("g{id}"),
            sender: from.map(str::to_owned),
            is_from_me: from.is_none(),
            text: Some(text.into()),
            date_created: date,
            service: Some("iMessage".into()),
            ..Default::default()
        }
    }

    fn build_with(rows: &[MessageRow], is_group: bool, typing: bool) -> Transcript {
        build_full(rows, is_group, typing, &HashMap::new(), &HashMap::new())
    }

    fn build_full(
        rows: &[MessageRow],
        is_group: bool,
        typing: bool,
        associated: &HashMap<String, Vec<MessageRow>>,
        originals: &HashMap<String, MessageRow>,
    ) -> Transcript {
        let names = Names::default();
        let to_local = |ms: i64| Utc.timestamp_millis_opt(ms).unwrap();
        let cx = Context {
            names: &names,
            is_group,
            chat_service: "iMessage",
            now: Utc.timestamp_millis_opt(1_800_000_000_000).unwrap(),
            to_local: &to_local,
            typing,
            associated,
            originals,
        };
        build(rows, &cx)
    }

    fn bubbles(t: &Transcript) -> Vec<&Bubble> {
        t.items
            .iter()
            .filter_map(|i| match i {
                Item::Bubble(b) => Some(b.as_ref()),
                _ => None,
            })
            .collect()
    }

    const T0: i64 = 1_799_990_000_000;

    #[test]
    fn runs_tails_and_separators() {
        let rows = vec![
            msg(1, Some("a"), "hi", T0),
            msg(2, Some("a"), "there", T0 + 10_000),
            msg(3, None, "hey", T0 + 20_000),
            msg(4, None, "later", T0 + 5 * MIN),
            msg(5, None, "much later", T0 + 60 * MIN),
        ];
        let t = build_with(&rows, false, false);
        let seps = t
            .items
            .iter()
            .filter(|i| matches!(i, Item::Separator { .. }))
            .count();
        assert_eq!(seps, 2);
        let b = bubbles(&t);
        assert_eq!(
            b.iter().map(|b| (b.first, b.tail)).collect::<Vec<_>>(),
            [
                (true, false),
                (false, true),
                (true, true),
                (true, true),
                (true, true)
            ]
        );
        assert_eq!(b[0].kind, Kind::Received);
        assert_eq!(b[2].kind, Kind::IMessage);
    }

    #[test]
    fn group_names_and_avatars() {
        let rows = vec![
            msg(1, Some("+15550000001"), "one", T0),
            msg(2, Some("+15550000001"), "two", T0 + 1000),
            msg(3, Some("+15550000002"), "three", T0 + 2000),
        ];
        let t = build_with(&rows, true, true);
        let b = bubbles(&t);
        assert!(b[0].sender_name.is_some());
        assert!(b[1].sender_name.is_none());
        assert!(b[2].sender_name.is_some());
        assert!(b.iter().all(|b| b.avatar_slot));
        assert_eq!(t.keys.last(), Some(&Key::Typing));
    }

    #[test]
    fn receipt_under_the_latest_only() {
        let mut a = msg(1, None, "one", T0);
        a.date_delivered = Some(T0 + 1000);
        a.date_read = Some(T0 + 2000);
        let mut b = msg(2, None, "two", T0 + 3000);
        b.date_delivered = Some(T0 + 4000);
        let t = build_with(&[a.clone(), b.clone()], false, false);
        let bs = bubbles(&t);
        assert_eq!(bs[0].receipt, None);
        assert_eq!(bs[1].receipt, Some(Receipt::Status("Delivered".into())));
        assert!(bs[1].tail);

        b.date_read = Some(T0 + 5000);
        let t = build_with(&[a, b.clone()], false, false);
        let Some(Receipt::Status(s)) = &bubbles(&t)[1].receipt else {
            panic!()
        };
        assert!(s.starts_with("Read "), "{s}");

        // Groups don't show Read.
        let t = build_with(&[b], true, false);
        assert_eq!(
            bubbles(&t)[0].receipt,
            Some(Receipt::Status("Delivered".into()))
        );
    }

    #[test]
    fn failed_sends_say_so() {
        let mut m = msg(1, None, "nope", T0);
        m.send_state = bb_store::SEND_FAILED;
        let t = build_with(&[m], false, false);
        assert_eq!(bubbles(&t)[0].receipt, Some(Receipt::Failed));
    }

    #[test]
    fn jumbo_emoji() {
        assert!(jumbo("😂"));
        assert!(jumbo("❤️🔥"));
        assert!(jumbo("👍🏽"));
        assert!(jumbo("👨‍👩‍👧"));
        assert!(!jumbo("😂😂😂😂"));
        assert!(!jumbo("ok 👍"));
        assert!(!jumbo("1"));
    }

    fn attachment(guid: &str, name: &str, mime: Option<&str>) -> AttachmentRow {
        AttachmentRow {
            guid: guid.into(),
            transfer_name: Some(name.into()),
            mime_type: mime.map(Into::into),
            total_bytes: Some(100),
            ..Default::default()
        }
    }

    #[test]
    fn attachments_become_media_and_plugin_data_is_hidden() {
        let mut m = msg(1, Some("a"), "\u{fffc}\u{fffc}look", T0);
        m.attachments = vec![
            attachment("p", "IMG_1.HEIC", None),
            attachment("v", "clip.mov", Some("video/quicktime")),
            attachment("x", "x.pluginPayloadAttachment", None),
            attachment("f", "notes.pdf", Some("application/pdf")),
        ];
        m.attachments[3].hidden = false;
        let t = build_with(&[m], false, false);
        let b = bubbles(&t)[0];
        assert_eq!(
            b.media.iter().map(|m| m.kind).collect::<Vec<_>>(),
            [MediaKind::Image, MediaKind::Video, MediaKind::File]
        );
        assert_eq!(b.text.as_deref(), Some("look"));
        assert!(!b.jumbo);
    }

    #[test]
    fn a_url_balloon_is_a_card_without_its_bare_url() {
        let mut m = msg(1, Some("a"), "https://example.com/trailer", T0);
        m.balloon_bundle_id = Some(URL_BALLOON.into());
        m.attachments = vec![
            attachment("icon", "a.pluginPayloadAttachment", None),
            AttachmentRow {
                total_bytes: Some(5000),
                ..attachment("image", "b.pluginPayloadAttachment", None)
            },
        ];
        let t = build_with(&[m], false, false);
        let b = bubbles(&t)[0];
        let link = b.link.as_ref().unwrap();
        assert_eq!(link.host, "example.com");
        assert_eq!(link.image.as_ref().unwrap().guid, "image");
        assert_eq!(b.text, None);
        assert!(b.media.is_empty());
    }

    #[test]
    fn tapbacks_and_quotes_attach_to_their_message() {
        let original = msg(1, Some("a"), "dinner at 8?", T0);
        let mut reply = msg(2, None, "works for me", T0 + 1000);
        reply.thread_originator_guid = Some(original.guid.clone());
        let mut love = msg(3, Some("a"), "Loved “works for me”", T0 + 2000);
        love.associated_guid = Some(reply.guid.clone());
        love.associated_type = Some("love".into());
        let associated = HashMap::from([(reply.guid.clone(), vec![love])]);
        let originals = HashMap::from([(original.guid.clone(), original.clone())]);
        let t = build_full(&[original, reply], false, false, &associated, &originals);
        let b = bubbles(&t);
        assert!(b[0].reactions.is_empty());
        assert_eq!(b[1].reactions.len(), 1);
        assert_eq!(b[1].reactions[0].tapback, bb_core::Tapback::Love);
        let quote = b[1].quote.as_ref().unwrap();
        assert_eq!(quote.text, "dinner at 8?");
    }

    #[test]
    fn finds_links_in_text() {
        let text = "See https://example.com/a?b=1. Or www.example.org, not e.g. this";
        let found = links(text);
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].0.clone()], "https://example.com/a?b=1");
        assert_eq!(found[1].1, "https://www.example.org");
        assert!(links("no links here").is_empty());
        assert!(links("https://").is_empty());
    }
}
