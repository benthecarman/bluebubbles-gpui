use crate::{Result, Store, chat_service};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatRow {
    pub id: i64,
    pub guid: String,
    pub style: Option<i64>,
    pub chat_identifier: Option<String>,
    pub display_name: Option<String>,
    pub is_archived: bool,
    pub complete: bool,
    /// Participant addresses, not including you.
    pub participants: Vec<String>,
    pub last_date: i64,
    pub unread: bool,
    pub muted: bool,
    pub pinned: Option<i64>,
    pub draft: Option<String>,
    pub history_done: bool,
    pub last_message: Option<MessageRow>,
}

impl ChatRow {
    pub fn is_group(&self) -> bool {
        self.style == Some(bb_api::STYLE_GROUP)
            || self.participants.len() > 1
            || self.guid.contains(";+;")
    }

    pub fn service(&self) -> &str {
        chat_service(&self.guid)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttachmentRow {
    pub guid: String,
    pub message_id: i64,
    pub position: i64,
    pub uti: Option<String>,
    pub mime_type: Option<String>,
    pub transfer_name: Option<String>,
    pub total_bytes: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub is_sticker: bool,
    pub has_live_photo: bool,
    pub hidden: bool,
    pub transfer_state: Option<i64>,
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageRow {
    pub id: i64,
    pub guid: String,
    pub temp_guid: Option<String>,
    pub chat_id: i64,
    pub server_rowid: Option<i64>,
    /// The sender's address; `None` for messages from you.
    pub sender: Option<String>,
    pub service: Option<String>,
    pub is_from_me: bool,
    pub text: Option<String>,
    pub subject: Option<String>,
    pub date_created: i64,
    pub date_delivered: Option<i64>,
    pub date_read: Option<i64>,
    pub date_edited: Option<i64>,
    pub date_retracted: Option<i64>,
    pub date_played: Option<i64>,
    pub is_delivered: bool,
    pub error: i64,
    pub item_type: i64,
    pub group_action_type: i64,
    pub group_title: Option<String>,
    /// For group events: the address of the person added or removed.
    pub other_address: Option<String>,
    pub balloon_bundle_id: Option<String>,
    pub associated_guid: Option<String>,
    pub associated_type: Option<String>,
    pub associated_part: Option<i64>,
    pub thread_originator_guid: Option<String>,
    pub thread_originator_part: Option<String>,
    pub effect: Option<String>,
    pub attributed_body: Option<Value>,
    pub summary_info: Option<Value>,
    pub payload_data: Option<Value>,
    pub part_count: Option<i64>,
    pub delivered_quietly: bool,
    pub did_notify_recipient: bool,
    pub is_audio: bool,
    pub send_state: i64,
    pub send_error: Option<String>,
    pub attachments: Vec<AttachmentRow>,
}

impl MessageRow {
    pub fn is_pending(&self) -> bool {
        self.send_state == crate::write::SEND_PENDING
    }

    pub fn is_failed(&self) -> bool {
        self.send_state == crate::write::SEND_FAILED || (self.is_from_me && self.error > 0)
    }

    /// Tapbacks and stickers placed on another message.
    pub fn is_associated(&self) -> bool {
        self.associated_guid.is_some()
    }
}

/// A position in a chat's history: messages sort by date, then row id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCursor {
    pub date: i64,
    pub id: i64,
}

impl MessageRow {
    pub fn cursor(&self) -> PageCursor {
        PageCursor {
            date: self.date_created,
            id: self.id,
        }
    }
}

const MESSAGE_COLUMNS: &str = "m.id, m.guid, m.temp_guid, m.chat_id, m.server_rowid, m.sender,
    m.service, m.is_from_me, m.text, m.subject, m.date_created, m.date_delivered, m.date_read,
    m.date_edited, m.date_retracted, m.date_played, m.is_delivered, m.error, m.item_type,
    m.group_action_type, m.group_title,
    (SELECT address FROM handles WHERE server_rowid = m.other_handle LIMIT 1),
    m.balloon_bundle_id, m.associated_guid, m.associated_type, m.associated_part,
    m.thread_originator_guid, m.thread_originator_part, m.effect, m.attributed_body,
    m.summary_info, m.payload_data, m.part_count, m.delivered_quietly, m.did_notify_recipient,
    m.is_audio, m.send_state, m.send_error";

fn parse_json(raw: Option<String>) -> Option<Value> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
}

fn message_row(r: &Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: r.get(0)?,
        guid: r.get(1)?,
        temp_guid: r.get(2)?,
        chat_id: r.get(3)?,
        server_rowid: r.get(4)?,
        sender: r.get(5)?,
        service: r.get(6)?,
        is_from_me: r.get(7)?,
        text: r.get(8)?,
        subject: r.get(9)?,
        date_created: r.get(10)?,
        date_delivered: r.get(11)?,
        date_read: r.get(12)?,
        date_edited: r.get(13)?,
        date_retracted: r.get(14)?,
        date_played: r.get(15)?,
        is_delivered: r.get::<_, Option<bool>>(16)?.unwrap_or(false),
        error: r.get(17)?,
        item_type: r.get(18)?,
        group_action_type: r.get(19)?,
        group_title: r.get(20)?,
        other_address: r.get(21)?,
        balloon_bundle_id: r.get(22)?,
        associated_guid: r.get(23)?,
        associated_type: r.get(24)?,
        associated_part: r.get(25)?,
        thread_originator_guid: r.get(26)?,
        thread_originator_part: r.get(27)?,
        effect: r.get(28)?,
        attributed_body: parse_json(r.get(29)?),
        summary_info: parse_json(r.get(30)?),
        payload_data: parse_json(r.get(31)?),
        part_count: r.get(32)?,
        delivered_quietly: r.get::<_, Option<bool>>(33)?.unwrap_or(false),
        did_notify_recipient: r.get::<_, Option<bool>>(34)?.unwrap_or(false),
        is_audio: r.get::<_, Option<bool>>(35)?.unwrap_or(false),
        send_state: r.get(36)?,
        send_error: r.get(37)?,
        attachments: Vec::new(),
    })
}

fn attachment_row(r: &Row) -> rusqlite::Result<AttachmentRow> {
    Ok(AttachmentRow {
        guid: r.get(0)?,
        message_id: r.get(1)?,
        position: r.get(2)?,
        uti: r.get(3)?,
        mime_type: r.get(4)?,
        transfer_name: r.get(5)?,
        total_bytes: r.get(6)?,
        width: r.get(7)?,
        height: r.get(8)?,
        is_sticker: r.get(9)?,
        has_live_photo: r.get(10)?,
        hidden: r.get(11)?,
        transfer_state: r.get(12)?,
        local_path: r.get(13)?,
    })
}

fn with_attachments(conn: &Connection, mut rows: Vec<MessageRow>) -> Result<Vec<MessageRow>> {
    if rows.is_empty() {
        return Ok(rows);
    }
    let ids: Vec<i64> = rows.iter().map(|m| m.id).collect();
    let placeholders = vec!["?"; ids.len()].join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT guid, message_id, position, uti, mime_type, transfer_name, total_bytes, width,
                height, is_sticker, has_live_photo, hidden, transfer_state, local_path
         FROM attachments WHERE message_id IN ({placeholders}) ORDER BY message_id, position"
    ))?;
    let mut by_message: HashMap<i64, Vec<AttachmentRow>> = HashMap::new();
    for a in stmt.query_map(params_from_iter(ids.iter()), attachment_row)? {
        let a = a?;
        by_message.entry(a.message_id).or_default().push(a);
    }
    for m in &mut rows {
        m.attachments = by_message.remove(&m.id).unwrap_or_default();
    }
    Ok(rows)
}

fn query_messages(
    conn: &Connection,
    filter: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<MessageRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages m {filter}"
    ))?;
    let rows = stmt
        .query_map(args, message_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    with_attachments(conn, rows)
}

const CHAT_COLUMNS: &str = "id, guid, style, chat_identifier, display_name, is_archived, complete,
    last_date, unread, muted, pinned, draft, history_done, last_message_id";

impl Store {
    /// Chats for the sidebar: pinned first, then most recent.
    pub fn chats(&self, include_archived: bool) -> Result<Vec<ChatRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CHAT_COLUMNS} FROM chats
             WHERE (?1 OR is_archived = 0) AND (complete = 1 OR last_message_id IS NOT NULL)
             ORDER BY pinned IS NULL, pinned, last_date DESC, id DESC"
        ))?;
        let rows = stmt
            .query_map([include_archived], |r| {
                Ok((chat_row(r)?, r.get::<_, Option<i64>>(13)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        finish_chats(&conn, rows)
    }

    pub fn chat(&self, id: i64) -> Result<Option<ChatRow>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                &format!("SELECT {CHAT_COLUMNS} FROM chats WHERE id = ?1"),
                [id],
                |r| Ok((chat_row(r)?, r.get::<_, Option<i64>>(13)?)),
            )
            .optional()?;
        Ok(finish_chats(&conn, row.into_iter().collect())?.pop())
    }

    pub fn chat_by_guid(&self, guid: &str) -> Result<Option<ChatRow>> {
        let id = {
            let mut conn = self.lock();
            let tx = conn.transaction()?;
            crate::write::find_chat(&tx, guid)?
        };
        match id {
            Some(id) => self.chat(id),
            None => Ok(None),
        }
    }

    /// Up to `limit` messages before `before` (or the newest), oldest
    /// first, without tapbacks (see [`Store::associated`]).
    pub fn messages_before(
        &self,
        chat_id: i64,
        before: Option<PageCursor>,
        limit: u32,
    ) -> Result<Vec<MessageRow>> {
        let conn = self.lock();
        let (date, id) = before.map_or((i64::MAX, i64::MAX), |c| (c.date, c.id));
        let mut rows = query_messages(
            &conn,
            "WHERE m.chat_id = ?1 AND m.associated_guid IS NULL
               AND (m.date_created < ?2 OR (m.date_created = ?2 AND m.id < ?3))
             ORDER BY m.date_created DESC, m.id DESC LIMIT ?4",
            &[&chat_id, &date, &id, &limit],
        )?;
        rows.reverse();
        Ok(rows)
    }

    /// Messages after `after`, oldest first, without tapbacks.
    pub fn messages_after(
        &self,
        chat_id: i64,
        after: PageCursor,
        limit: u32,
    ) -> Result<Vec<MessageRow>> {
        let conn = self.lock();
        query_messages(
            &conn,
            "WHERE m.chat_id = ?1 AND m.associated_guid IS NULL
               AND (m.date_created > ?2 OR (m.date_created = ?2 AND m.id > ?3))
             ORDER BY m.date_created, m.id LIMIT ?4",
            &[&chat_id, &after.date, &after.id, &limit],
        )
    }

    pub fn messages_by_id(&self, ids: &[i64]) -> Result<Vec<MessageRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m WHERE m.id IN ({placeholders})
             ORDER BY m.date_created, m.id"
        ))?;
        let rows = stmt
            .query_map(params_from_iter(ids.iter()), message_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        with_attachments(&conn, rows)
    }

    pub fn message_by_guid(&self, guid: &str) -> Result<Option<MessageRow>> {
        let conn = self.lock();
        Ok(query_messages(&conn, "WHERE m.guid = ?1", &[&guid])?.pop())
    }

    /// Tapbacks and stickers on any of `guids`, oldest first.
    pub fn associated(&self, guids: &[String]) -> Result<Vec<MessageRow>> {
        if guids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; guids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m WHERE m.associated_guid IN ({placeholders})
             ORDER BY m.date_created, m.id"
        ))?;
        let rows = stmt
            .query_map(params_from_iter(guids.iter()), message_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        with_attachments(&conn, rows)
    }

    /// How many replies each of `guids` has.
    pub fn reply_counts(&self, guids: &[String]) -> Result<HashMap<String, i64>> {
        if guids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; guids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT thread_originator_guid, COUNT(*) FROM messages
             WHERE thread_originator_guid IN ({placeholders}) GROUP BY thread_originator_guid"
        ))?;
        let rows = stmt.query_map(params_from_iter(guids.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The oldest message we have of a chat, for fetching older history.
    pub fn oldest_message_date(&self, chat_id: i64) -> Result<Option<i64>> {
        Ok(self.lock().query_row(
            "SELECT MIN(date_created) FROM messages WHERE chat_id = ?1 AND send_state = 0",
            [chat_id],
            |r| r.get(0),
        )?)
    }

    /// Whether storing a message keeps its chat's cached history unbroken:
    /// it is cached already, or no older than anything cached. History
    /// pages back from the oldest cached message, so an older row stored
    /// on its own would make paging skip everything in between.
    pub fn fits_history(&self, guid: &str, chat_guid: &str, date: i64) -> Result<bool> {
        let conn = self.lock();
        let known = conn
            .query_row("SELECT 1 FROM messages WHERE guid = ?1", [guid], |_| Ok(()))
            .optional()?
            .is_some();
        if known {
            return Ok(true);
        }
        let chat: Option<(bool, i64, Option<i64>)> = conn
            .query_row(
                "SELECT history_done, last_date,
                   (SELECT MIN(date_created) FROM messages
                    WHERE chat_id = chats.id AND send_state = 0)
                 FROM chats WHERE guid = ?1",
                [chat_guid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match chat {
            None => false,
            Some((true, ..)) => true,
            // last_date is 0 until the chat has a message.
            Some((false, last, oldest)) => oldest
                .or((last > 0).then_some(last))
                .is_some_and(|from| date >= from),
        })
    }

    /// The newest server row id and creation date seen, for catch-up.
    pub fn newest_server_message(&self) -> Result<(Option<i64>, Option<i64>)> {
        Ok(self.lock().query_row(
            "SELECT MAX(server_rowid), MAX(date_created) FROM messages WHERE send_state = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// Conversations with unread messages, not counting muted or archived
    /// ones: the tray's dot and the dock's count.
    pub fn unread_chat_count(&self) -> Result<usize> {
        let n: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM chats WHERE unread = 1 AND muted = 0 AND is_archived = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Contact names and whether an avatar exists, for normalized addresses.
    pub fn contacts_for(&self, addresses: &[String]) -> Result<HashMap<String, ContactRow>> {
        if addresses.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; addresses.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT a.address, c.id, c.display_name, c.first_name, c.last_name, c.nickname,
                    c.avatar IS NOT NULL
             FROM contact_addresses a JOIN contacts c ON c.id = a.contact_id
             WHERE a.address IN ({placeholders})"
        ))?;
        let rows = stmt.query_map(params_from_iter(addresses.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                ContactRow {
                    id: r.get(1)?,
                    display_name: r.get(2)?,
                    first_name: r.get(3)?,
                    last_name: r.get(4)?,
                    nickname: r.get(5)?,
                    has_avatar: r.get(6)?,
                },
            ))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn contact_avatar(&self, contact_id: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT avatar FROM contacts WHERE id = ?1",
                [contact_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Every contact address we know, normalized, with the contact's name.
    pub fn contact_count(&self) -> Result<i64> {
        Ok(self
            .lock()
            .query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0))?)
    }

    pub fn attachment(&self, guid: &str) -> Result<Option<AttachmentRow>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT guid, message_id, position, uti, mime_type, transfer_name, total_bytes,
                        width, height, is_sticker, has_live_photo, hidden, transfer_state, local_path
                 FROM attachments WHERE guid = ?1",
                params![guid],
                attachment_row,
            )
            .optional()?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactRow {
    pub id: String,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub nickname: Option<String>,
    pub has_avatar: bool,
}

fn chat_row(r: &Row) -> rusqlite::Result<ChatRow> {
    Ok(ChatRow {
        id: r.get(0)?,
        guid: r.get(1)?,
        style: r.get(2)?,
        chat_identifier: r.get(3)?,
        display_name: r.get(4)?,
        is_archived: r.get(5)?,
        complete: r.get(6)?,
        participants: Vec::new(),
        last_date: r.get(7)?,
        unread: r.get(8)?,
        muted: r.get(9)?,
        pinned: r.get(10)?,
        draft: r.get(11)?,
        history_done: r.get(12)?,
        last_message: None,
    })
}

fn finish_chats(conn: &Connection, rows: Vec<(ChatRow, Option<i64>)>) -> Result<Vec<ChatRow>> {
    let mut participants: HashMap<i64, Vec<String>> = HashMap::new();
    {
        let mut stmt =
            conn.prepare("SELECT chat_id, address FROM chat_participants ORDER BY rowid")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (chat, address) = row?;
            participants.entry(chat).or_default().push(address);
        }
    }
    let last_ids: Vec<i64> = rows.iter().filter_map(|(_, id)| *id).collect();
    let mut last: HashMap<i64, MessageRow> = HashMap::new();
    for chunk in last_ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m WHERE m.id IN ({placeholders})"
        ))?;
        let found = stmt
            .query_map(params_from_iter(chunk.iter()), message_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        for m in with_attachments(conn, found)? {
            last.insert(m.id, m);
        }
    }
    Ok(rows
        .into_iter()
        .map(|(mut chat, last_id)| {
            chat.participants = participants.remove(&chat.id).unwrap_or_default();
            chat.last_message = last_id.and_then(|id| last.remove(&id));
            chat
        })
        .collect())
}
