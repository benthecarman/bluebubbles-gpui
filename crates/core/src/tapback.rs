//! Tapbacks: the server's `associatedMessageType` values and the current
//! reactions on a message.

use bb_store::MessageRow;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Tapback {
    Love,
    Like,
    Dislike,
    Laugh,
    Emphasize,
    Question,
    /// Any emoji (iOS 18 / macOS 15).
    Emoji(String),
}

impl Tapback {
    /// The six classic tapbacks, in picker order.
    pub const CLASSIC: [Tapback; 6] = [
        Tapback::Love,
        Tapback::Like,
        Tapback::Dislike,
        Tapback::Laugh,
        Tapback::Emphasize,
        Tapback::Question,
    ];

    /// The `reaction` value for `POST /message/react`.
    pub fn api_name(&self) -> String {
        match self {
            Tapback::Love => "love".into(),
            Tapback::Like => "like".into(),
            Tapback::Dislike => "dislike".into(),
            Tapback::Laugh => "laugh".into(),
            Tapback::Emphasize => "emphasize".into(),
            Tapback::Question => "question".into(),
            Tapback::Emoji(e) => e.clone(),
        }
    }

    /// Past tense for previews: "Loved", "Laughed at".
    pub fn verb(&self) -> String {
        match self {
            Tapback::Love => "Loved".into(),
            Tapback::Like => "Liked".into(),
            Tapback::Dislike => "Disliked".into(),
            Tapback::Laugh => "Laughed at".into(),
            Tapback::Emphasize => "Emphasized".into(),
            Tapback::Question => "Questioned".into(),
            Tapback::Emoji(e) => format!("Reacted {e} to"),
        }
    }
}

/// What an associated message does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Association {
    Add(Tapback),
    Remove(Option<Tapback>),
    Sticker,
    /// A type this client doesn't know.
    Other,
}

/// Parses `associatedMessageType`; `text` is needed for emoji tapbacks,
/// whose emoji only appears in "Reacted 😂 to “…”".
pub fn parse(kind: &str, text: Option<&str>) -> Association {
    let (removed, name) = match kind.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, kind),
    };
    let classic = match name {
        "love" | "2000" | "3000" => Some(Tapback::Love),
        "like" | "2001" | "3001" => Some(Tapback::Like),
        "dislike" | "2002" | "3002" => Some(Tapback::Dislike),
        "laugh" | "2003" | "3003" => Some(Tapback::Laugh),
        "emphasize" | "2004" | "3004" => Some(Tapback::Emphasize),
        "question" | "2005" | "3005" => Some(Tapback::Question),
        _ => None,
    };
    let removed = removed || name.starts_with('3');
    if let Some(t) = classic {
        return if removed {
            Association::Remove(Some(t))
        } else {
            Association::Add(t)
        };
    }
    match name {
        "2006" => match text.and_then(emoji_of) {
            Some(e) => Association::Add(Tapback::Emoji(e)),
            None => Association::Other,
        },
        "3006" => Association::Remove(text.and_then(emoji_of).map(Tapback::Emoji)),
        "sticker" | "1000" | "2007" => Association::Sticker,
        "3007" => Association::Remove(None),
        _ => Association::Other,
    }
}

/// The emoji in "Reacted 😂 to …" or "Removed a 😂 from …".
fn emoji_of(text: &str) -> Option<String> {
    let rest = text
        .strip_prefix("Reacted ")
        .or_else(|| text.strip_prefix("Removed a "))
        .or_else(|| text.strip_prefix("Removed "))?;
    let end = rest.find(" to ").or_else(|| rest.find(" from "))?;
    let emoji = rest[..end].trim();
    (!emoji.is_empty()).then(|| emoji.to_owned())
}

/// One person's current tapback on a message part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub tapback: Tapback,
    pub from_me: bool,
    /// The sender's address, `None` for me.
    pub sender: Option<String>,
    pub date: i64,
    /// The tapback's own message GUID.
    pub guid: String,
}

/// The tapbacks standing on `part` of a message, given its associated
/// messages in any order: each person's latest wins, and a removal clears
/// theirs. Oldest first.
pub fn reactions(associated: &[MessageRow], part: i64) -> Vec<Reaction> {
    let mut rows: Vec<&MessageRow> = associated
        .iter()
        .filter(|m| m.associated_part.unwrap_or(0) == part)
        .collect();
    rows.sort_by_key(|m| (m.date_created, m.id));
    let mut out: Vec<Reaction> = Vec::new();
    for m in rows {
        let Some(kind) = m.associated_type.as_deref() else {
            continue;
        };
        let sender = if m.is_from_me { None } else { m.sender.clone() };
        let association = parse(kind, m.text.as_deref());
        if matches!(association, Association::Sticker | Association::Other) {
            continue;
        }
        out.retain(|r| !(r.from_me == m.is_from_me && r.sender == sender));
        if let Association::Add(tapback) = association {
            out.push(Reaction {
                tapback,
                from_me: m.is_from_me,
                sender,
                date: m.date_created,
                guid: m.guid.clone(),
            });
        }
    }
    out
}

/// The tapback I have on a part, to toggle it from the picker.
pub fn mine(reactions: &[Reaction]) -> Option<&Tapback> {
    reactions.iter().find(|r| r.from_me).map(|r| &r.tapback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_values() {
        assert_eq!(parse("love", None), Association::Add(Tapback::Love));
        assert_eq!(
            parse("-laugh", None),
            Association::Remove(Some(Tapback::Laugh))
        );
        assert_eq!(parse("2001", None), Association::Add(Tapback::Like));
        assert_eq!(
            parse("3005", None),
            Association::Remove(Some(Tapback::Question))
        );
        assert_eq!(
            parse("2006", Some("Reacted 🔥 to “nice”")),
            Association::Add(Tapback::Emoji("🔥".into()))
        );
        assert_eq!(
            parse("3006", Some("Removed a 🔥 from “nice”")),
            Association::Remove(Some(Tapback::Emoji("🔥".into())))
        );
        assert_eq!(parse("sticker", None), Association::Sticker);
        assert_eq!(parse("2006", None), Association::Other);
        assert_eq!(parse("4000", None), Association::Other);
    }

    fn row(id: i64, from_me: bool, sender: &str, kind: &str, date: i64) -> MessageRow {
        MessageRow {
            id,
            guid: format!("g{id}"),
            sender: (!from_me).then(|| sender.to_owned()),
            is_from_me: from_me,
            date_created: date,
            associated_guid: Some("p:0/T".into()),
            associated_type: Some(kind.into()),
            associated_part: Some(0),
            ..Default::default()
        }
    }

    #[test]
    fn latest_per_person_wins() {
        let rows = vec![
            row(3, false, "a", "laugh", 30),
            row(1, false, "a", "love", 10),
            row(2, false, "b", "like", 20),
            row(4, true, "", "love", 40),
            row(5, false, "b", "-like", 50),
        ];
        let r = reactions(&rows, 0);
        assert_eq!(
            r.iter()
                .map(|r| (&r.tapback, r.from_me))
                .collect::<Vec<_>>(),
            [(&Tapback::Laugh, false), (&Tapback::Love, true)]
        );
        assert_eq!(mine(&r), Some(&Tapback::Love));
        assert!(reactions(&rows, 1).is_empty());
    }
}
