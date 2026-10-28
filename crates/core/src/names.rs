//! What people and chats are called, and one-line message previews for
//! the sidebar and notifications.

use crate::address::{self, Region};
use crate::tapback::{self, Association};
use bb_store::{ChatRow, ContactRow, MessageRow, Store};
use std::collections::HashMap;

/// Contact names for a set of addresses, loaded in one query.
#[derive(Debug, Clone, Default)]
pub struct Names {
    region: Option<Region>,
    by_key: HashMap<String, ContactRow>,
}

impl Names {
    pub fn load<'a>(
        store: &Store,
        region: Region,
        addresses: impl IntoIterator<Item = &'a str>,
    ) -> bb_store::Result<Self> {
        let mut keys: Vec<String> = addresses
            .into_iter()
            .map(|a| address::normalize(a, region))
            .collect();
        keys.sort();
        keys.dedup();
        Ok(Self {
            region: Some(region),
            by_key: store.contacts_for(&keys)?,
        })
    }

    /// Every address a chat list or transcript mentions.
    pub fn for_chats(store: &Store, region: Region, chats: &[ChatRow]) -> bb_store::Result<Self> {
        let addresses = chats.iter().flat_map(|c| {
            c.participants
                .iter()
                .map(String::as_str)
                .chain(c.last_message.as_ref().and_then(|m| m.sender.as_deref()))
        });
        Self::load(store, region, addresses)
    }

    fn region(&self) -> Region {
        self.region.unwrap_or(Region::new("US"))
    }

    pub fn contact(&self, address: &str) -> Option<&ContactRow> {
        self.by_key.get(&address::normalize(address, self.region()))
    }

    /// "Alex Kim", or the formatted number or email.
    pub fn full(&self, address: &str) -> String {
        match self.contact(address) {
            Some(c) => full_name(c).unwrap_or_else(|| address::display(address, self.region())),
            None => address::display(address, self.region()),
        }
    }

    /// "Alex", for group titles and group transcripts.
    pub fn short(&self, address: &str) -> String {
        match self.contact(address) {
            Some(c) => c
                .nickname
                .clone()
                .filter(|n| !n.trim().is_empty())
                .or_else(|| c.first_name.clone().filter(|n| !n.trim().is_empty()))
                .or_else(|| full_name(c))
                .unwrap_or_else(|| address::display(address, self.region())),
            None => address::display(address, self.region()),
        }
    }

    /// Initials for an avatar, or `None` when there is no name to take them
    /// from (the avatar shows a person glyph instead).
    pub fn initials(&self, address: &str) -> Option<String> {
        let c = self.contact(address)?;
        let first = c
            .first_name
            .as_deref()
            .and_then(|n| n.trim().chars().next());
        let last = c.last_name.as_deref().and_then(|n| n.trim().chars().next());
        let s: String = match (first, last) {
            (Some(f), Some(l)) => [f, l].iter().collect(),
            (Some(f), None) => f.to_string(),
            _ => full_name(c)?
                .split_whitespace()
                .filter_map(|w| w.chars().next())
                .take(2)
                .collect(),
        };
        let s = s.to_uppercase();
        (!s.is_empty() && s.chars().all(char::is_alphabetic)).then_some(s)
    }

    /// A chat's title: its name, the other person, or "Alex, Sam & Jordan".
    pub fn chat_title(&self, chat: &ChatRow) -> String {
        if let Some(name) = chat
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            return name.to_owned();
        }
        match chat.participants.as_slice() {
            [] => chat
                .chat_identifier
                .as_deref()
                .map(|id| address::display(id, self.region()))
                .unwrap_or_default(),
            [one] => self.full(one),
            many => {
                let names: Vec<String> = many.iter().map(|a| self.short(a)).collect();
                join_names(&names)
            }
        }
    }

    /// Who did something, for previews and group events.
    fn actor(&self, m: &MessageRow) -> String {
        if m.is_from_me {
            "You".into()
        } else {
            m.sender
                .as_deref()
                .map(|s| self.short(s))
                .unwrap_or_else(|| "Someone".into())
        }
    }

    /// The one-line summary of a message.
    pub fn preview(&self, m: &MessageRow) -> String {
        if m.date_retracted.is_some() {
            return if m.is_from_me {
                "You unsent a message".into()
            } else {
                format!("{} unsent a message", self.actor(m))
            };
        }
        if let Some(event) = self.group_event(m) {
            return event;
        }
        if let Some(kind) = m.associated_type.as_deref() {
            match tapback::parse(kind, m.text.as_deref()) {
                Association::Sticker => return "Sticker".into(),
                Association::Remove(_) => return clean(m.text.as_deref()).unwrap_or_default(),
                _ => {}
            }
        }
        if let Some(text) = clean(m.text.as_deref()) {
            return text;
        }
        if let Some(subject) = clean(m.subject.as_deref()) {
            return subject;
        }
        attachment_summary(m)
    }

    /// The line a group event shows in the transcript, or `None` for an
    /// ordinary message.
    pub fn group_event(&self, m: &MessageRow) -> Option<String> {
        let actor = self.actor(m);
        let other = || {
            m.other_address
                .as_deref()
                .map(|a| self.full(a))
                .unwrap_or_else(|| "someone".into())
        };
        Some(match (m.item_type, m.group_action_type) {
            (1, 0) => format!("{actor} added {} to the conversation", other()),
            (1, 1) => format!("{actor} removed {} from the conversation", other()),
            (2, _) => match m.group_title.as_deref().filter(|t| !t.is_empty()) {
                Some(title) => format!("{actor} named the conversation “{title}”"),
                None => format!("{actor} removed the name from the conversation"),
            },
            (3, 0) => format!("{actor} left the conversation"),
            (3, 1) => format!("{actor} changed the group photo"),
            (3, 2) => format!("{actor} removed the group photo"),
            (0, _) => return None,
            _ => return None,
        })
    }
}

fn full_name(c: &ContactRow) -> Option<String> {
    let display = c
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    if let Some(d) = display {
        return Some(d.to_owned());
    }
    let parts: Vec<&str> = [c.first_name.as_deref(), c.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// "A", "A & B", "A, B & C", "A, B & 3 others".
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} & {b}"),
        [a, b, c] => format!("{a}, {b} & {c}"),
        [a, b, rest @ ..] => format!("{a}, {b} & {} others", rest.len()),
    }
}

/// Text without the object-replacement characters that stand in for
/// attachments, or `None` if nothing is left.
fn clean(text: Option<&str>) -> Option<String> {
    let t: String = text?
        .chars()
        .filter(|&c| c != '\u{fffc}' && c != '\u{fffd}')
        .collect();
    let t = t.trim();
    (!t.is_empty()).then(|| t.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn attachment_summary(m: &MessageRow) -> String {
    let visible: Vec<_> = m.attachments.iter().filter(|a| !a.hidden).collect();
    if visible.is_empty() {
        return String::new();
    }
    if m.is_audio {
        return "Audio Message".into();
    }
    let kind = |a: &&bb_store::AttachmentRow| {
        let mime = a.mime_type.as_deref().unwrap_or_default();
        if a.is_sticker {
            "Sticker"
        } else if mime.starts_with("image/") {
            "Image"
        } else if mime.starts_with("video/") {
            "Video"
        } else if mime.starts_with("audio/") {
            "Audio"
        } else {
            "Attachment"
        }
    };
    let first = kind(&visible[0]);
    if visible.iter().all(|a| kind(a) == first) {
        if visible.len() == 1 {
            first.to_owned()
        } else {
            format!("{} {first}s", visible.len())
        }
    } else {
        format!("{} Attachments", visible.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_like_messages() {
        let n = |v: &[&str]| join_names(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(n(&["A", "B"]), "A & B");
        assert_eq!(n(&["A", "B", "C"]), "A, B & C");
        assert_eq!(n(&["A", "B", "C", "D", "E"]), "A, B & 3 others");
    }

    #[test]
    fn previews() {
        let names = Names::default();
        let mut m = MessageRow {
            text: Some("\u{fffc}".into()),
            attachments: vec![
                bb_store::AttachmentRow {
                    mime_type: Some("image/jpeg".into()),
                    ..Default::default()
                };
                2
            ],
            ..Default::default()
        };
        assert_eq!(names.preview(&m), "2 Images");
        m.text = Some("look  at\nthis".into());
        assert_eq!(names.preview(&m), "look at this");
        m.is_from_me = true;
        m.date_retracted = Some(1);
        assert_eq!(names.preview(&m), "You unsent a message");
        let event = MessageRow {
            item_type: 2,
            is_from_me: true,
            group_title: Some("Trip".into()),
            ..Default::default()
        };
        assert_eq!(names.preview(&event), "You named the conversation “Trip”");
    }
}
