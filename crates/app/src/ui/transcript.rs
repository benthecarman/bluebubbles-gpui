//! Turns a chat's messages into what the transcript draws: timestamp
//! separators, group events, bubble runs with tails, sender names in
//! groups, and the receipt under your latest message.
//!
//! Pure data, so the layout rules are unit tested without a window.

use super::time;
use bb_core::Names;
use bb_store::MessageRow;
use chrono::{DateTime, TimeZone};
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
        let text = visible_text(m);
        let jumbo = text.as_deref().is_some_and(jumbo) && m.attachments.is_empty();
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
        let names = Names::default();
        let to_local = |ms: i64| Utc.timestamp_millis_opt(ms).unwrap();
        let cx = Context {
            names: &names,
            is_group,
            chat_service: "iMessage",
            now: Utc.timestamp_millis_opt(1_800_000_000_000).unwrap(),
            to_local: &to_local,
            typing,
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
}
