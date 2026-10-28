use crate::{Changes, Result, Store, StoreError, chat_service, split_part_guid};
use bb_api::{Attachment, Chat, Handle, Message};
use rusqlite::{OptionalExtension, Transaction, named_params, params};

/// One unit of input from the server, applied atomically.
#[derive(Debug, Default)]
pub struct Batch {
    /// Chats with participants, from the chat endpoints. A chat's
    /// `lastMessage` is applied too.
    pub chats: Vec<Chat>,
    /// Messages, with the chat they belong to when the payload doesn't
    /// say (chat history pages don't).
    pub messages: Vec<(Option<String>, Message)>,
    /// Server read-state changes: `(chat guid, read)`.
    pub read_states: Vec<(String, bool)>,
    /// Chats whose oldest message has been fetched.
    pub history_done: Vec<String>,
}

/// A message this client is sending, shown before the server has it.
#[derive(Debug, Clone, Default)]
pub struct OutgoingMessage {
    pub temp_guid: String,
    pub chat_guid: String,
    pub text: Option<String>,
    pub subject: Option<String>,
    pub date_created: i64,
    pub effect: Option<String>,
    /// Reply target: GUID and part.
    pub reply_to: Option<(String, i64)>,
    /// Tapback: target GUID, type (`love`, `-love`, …), part.
    pub tapback: Option<(String, String, i64)>,
    pub attachments: Vec<OutgoingAttachment>,
}

#[derive(Debug, Clone, Default)]
pub struct OutgoingAttachment {
    pub guid: String,
    pub path: String,
    pub name: String,
    pub mime_type: String,
    pub total_bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

/// A contact from the server, with addresses already normalized.
#[derive(Debug, Clone, Default)]
pub struct ContactInput {
    pub id: String,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub nickname: Option<String>,
    pub addresses: Vec<String>,
    /// `None` keeps the stored avatar.
    pub avatar: Option<Vec<u8>>,
}

/// A queued request in the outbox.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxItem {
    pub id: i64,
    pub temp_guid: String,
    pub chat_guid: String,
    pub kind: String,
    pub payload: String,
    pub state: i64,
    pub attempts: i64,
    pub created_at: i64,
}

pub const OUTBOX_QUEUED: i64 = 0;
pub const OUTBOX_IN_FLIGHT: i64 = 1;
pub const OUTBOX_FAILED: i64 = 2;

pub const SEND_SENT: i64 = 0;
pub const SEND_PENDING: i64 = 1;
pub const SEND_FAILED: i64 = 2;

impl Store {
    pub fn apply(&self, batch: Batch) -> Result<Changes> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        let mut changes = Changes::default();
        for chat in &batch.chats {
            let id = upsert_chat(&tx, chat, true)?;
            changes.chats.insert(id);
            if let Some(last) = &chat.last_message {
                upsert_message(&tx, Some(&chat.guid), last, &mut changes)?;
            }
        }
        for (hint, message) in &batch.messages {
            upsert_message(&tx, hint.as_deref(), message, &mut changes)?;
        }
        for (guid, read) in &batch.read_states {
            if let Some(id) = find_chat(&tx, guid)? {
                set_read(&tx, id, *read)?;
                changes.chats.insert(id);
            }
        }
        for guid in &batch.history_done {
            if let Some(id) = find_chat(&tx, guid)? {
                tx.execute("UPDATE chats SET history_done = 1 WHERE id = ?1", [id])?;
            }
        }
        for &chat in &changes.chats.clone() {
            recompute_chat(&tx, chat)?;
        }
        tx.commit()?;
        Ok(changes)
    }

    /// Shows a message being sent. Returns its row id.
    pub fn insert_outgoing(&self, m: &OutgoingMessage) -> Result<(Changes, i64)> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        let mut changes = Changes::default();
        let chat = ensure_chat(&tx, &m.chat_guid, &mut changes)?;
        let service = service_for_chat(&tx, chat, &m.chat_guid)?;
        tx.execute(
            "INSERT INTO messages (guid, temp_guid, chat_id, is_from_me, service, text, subject,
                 date_created, effect, thread_originator_guid, thread_originator_part,
                 associated_guid, associated_type, associated_part, send_state, notified)
             VALUES (?1, ?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1)",
            params![
                m.temp_guid,
                chat,
                service,
                m.text,
                m.subject,
                m.date_created,
                m.effect,
                m.reply_to.as_ref().map(|r| &r.0),
                m.reply_to.as_ref().map(|r| format!("{}:0:0", r.1)),
                m.tapback.as_ref().map(|t| &t.0),
                m.tapback.as_ref().map(|t| &t.1),
                m.tapback.as_ref().map(|t| t.2),
                SEND_PENDING,
            ],
        )?;
        let id = tx.last_insert_rowid();
        for (position, a) in m.attachments.iter().enumerate() {
            tx.execute(
                "INSERT INTO attachments (guid, message_id, position, mime_type, transfer_name,
                     total_bytes, width, height, local_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    a.guid,
                    id,
                    position as i64,
                    a.mime_type,
                    a.name,
                    a.total_bytes,
                    a.width,
                    a.height,
                    a.path
                ],
            )?;
        }
        changes.message(chat, id);
        recompute_chat(&tx, chat)?;
        tx.commit()?;
        Ok((changes, id))
    }

    /// Marks a pending message failed, unless the server already has it.
    pub fn fail_outgoing(&self, temp_guid: &str, error: &str) -> Result<Changes> {
        self.set_send_state(temp_guid, SEND_FAILED, Some(error))
    }

    /// Marks a failed message pending again, for a retry.
    pub fn retry_outgoing(&self, temp_guid: &str) -> Result<Changes> {
        self.set_send_state(temp_guid, SEND_PENDING, None)
    }

    fn set_send_state(&self, temp_guid: &str, state: i64, error: Option<&str>) -> Result<Changes> {
        let conn = self.lock();
        let mut changes = Changes::default();
        // Only rows still under their temp GUID: a resolved message is sent.
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT id, chat_id FROM messages WHERE guid = ?1 AND temp_guid = ?1",
                [temp_guid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, chat)) = row {
            conn.execute(
                "UPDATE messages SET send_state = ?2, send_error = ?3 WHERE id = ?1",
                params![id, state, error],
            )?;
            changes.message(chat, id);
        }
        Ok(changes)
    }

    /// Removes a message that never reached the server.
    pub fn discard_outgoing(&self, temp_guid: &str) -> Result<Changes> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        let mut changes = Changes::default();
        let row: Option<(i64, i64)> = tx
            .query_row(
                "SELECT id, chat_id FROM messages WHERE guid = ?1 AND temp_guid = ?1",
                [temp_guid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, chat)) = row {
            tx.execute("DELETE FROM messages WHERE id = ?1", [id])?;
            changes.message(chat, id);
            recompute_chat(&tx, chat)?;
        }
        tx.commit()?;
        Ok(changes)
    }

    /// The user read the chat here.
    pub fn mark_read_local(&self, chat_id: i64) -> Result<Changes> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        set_read(&tx, chat_id, true)?;
        recompute_chat(&tx, chat_id)?;
        tx.commit()?;
        Ok(Changes {
            chats: [chat_id].into(),
            ..Default::default()
        })
    }

    /// The user read every chat here, muted and archived ones too.
    /// Returns the chats that were unread, with their GUIDs.
    pub fn mark_all_read_local(&self) -> Result<(Changes, Vec<(i64, String)>)> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        let unread: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, guid FROM chats WHERE unread = 1")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        for (id, _) in &unread {
            set_read(&tx, *id, true)?;
            recompute_chat(&tx, *id)?;
        }
        tx.commit()?;
        let changes = Changes {
            chats: unread.iter().map(|(id, _)| *id).collect(),
            ..Default::default()
        };
        Ok((changes, unread))
    }

    /// Of `ids`, the incoming messages not yet notified and created at or
    /// after `since` (ms); all of `ids` are marked notified.
    pub fn take_notifiable(&self, ids: &[i64], since: i64) -> Result<Vec<i64>> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        let mut out = Vec::new();
        for &id in ids {
            let row: Option<(bool, bool, i64)> = tx
                .query_row(
                    "SELECT notified, is_from_me, date_created FROM messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if let Some((notified, from_me, date)) = row {
                if !notified && !from_me && date >= since {
                    out.push(id);
                }
                tx.execute("UPDATE messages SET notified = 1 WHERE id = ?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(out)
    }

    /// Records where an attachment's file was saved; the message changes.
    pub fn set_attachment_file(&self, guid: &str, path: &str) -> Result<Changes> {
        let conn = self.lock();
        conn.execute(
            "UPDATE attachments SET local_path = ?2 WHERE guid = ?1",
            params![guid, path],
        )?;
        let mut changes = Changes::default();
        let found: Option<(i64, i64)> = conn
            .query_row(
                "SELECT m.chat_id, m.id FROM attachments a JOIN messages m ON m.id = a.message_id
                 WHERE a.guid = ?1",
                [guid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((chat, id)) = found {
            changes.message(chat, id);
        }
        Ok(changes)
    }

    pub fn set_draft(&self, chat_id: i64, draft: Option<&str>) -> Result<()> {
        self.lock().execute(
            "UPDATE chats SET draft = ?2 WHERE id = ?1",
            params![chat_id, draft.filter(|d| !d.is_empty())],
        )?;
        Ok(())
    }

    pub fn set_muted(&self, chat_id: i64, muted: bool) -> Result<Changes> {
        self.lock().execute(
            "UPDATE chats SET muted = ?2 WHERE id = ?1",
            params![chat_id, muted],
        )?;
        Ok(Changes {
            chats: [chat_id].into(),
            ..Default::default()
        })
    }

    pub fn set_pinned(&self, chat_id: i64, pinned: Option<i64>) -> Result<Changes> {
        self.lock().execute(
            "UPDATE chats SET pinned = ?2 WHERE id = ?1",
            params![chat_id, pinned],
        )?;
        Ok(Changes {
            chats: [chat_id].into(),
            ..Default::default()
        })
    }

    /// Replaces every contact. Addresses must be normalized already.
    pub fn replace_contacts(&self, contacts: &[ContactInput]) -> Result<()> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        let keep: Vec<&str> = contacts.iter().map(|c| c.id.as_str()).collect();
        // Delete the ones that are gone, keeping avatars of the others.
        let existing: Vec<String> = tx
            .prepare("SELECT id FROM contacts")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for id in existing.iter().filter(|id| !keep.contains(&id.as_str())) {
            tx.execute("DELETE FROM contacts WHERE id = ?1", [id])?;
        }
        tx.execute("DELETE FROM contact_addresses", [])?;
        for c in contacts {
            tx.execute(
                "INSERT INTO contacts (id, display_name, first_name, last_name, nickname, avatar)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name,
                     first_name = excluded.first_name, last_name = excluded.last_name,
                     nickname = excluded.nickname,
                     avatar = COALESCE(excluded.avatar, avatar)",
                params![
                    c.id,
                    c.display_name,
                    c.first_name,
                    c.last_name,
                    c.nickname,
                    c.avatar
                ],
            )?;
            for a in &c.addresses {
                tx.execute(
                    "INSERT OR IGNORE INTO contact_addresses (address, contact_id) VALUES (?1, ?2)",
                    params![a, c.id],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets the photos of known contacts; `None` removes one. Unknown ids
    /// are skipped. Returns how many photos changed.
    pub fn set_contact_avatars(&self, avatars: &[(String, Option<Vec<u8>>)]) -> Result<usize> {
        let mut conn = self.lock();
        let tx = crate::write_transaction(&mut conn)?;
        let mut changed = 0;
        for (id, avatar) in avatars {
            changed += tx.execute(
                "UPDATE contacts SET avatar = ?2 WHERE id = ?1 AND avatar IS NOT ?2",
                params![id, avatar],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn outbox_push(
        &self,
        temp_guid: &str,
        chat_guid: &str,
        kind: &str,
        payload: &str,
        now: i64,
    ) -> Result<()> {
        self.lock().execute(
            "INSERT INTO outbox (temp_guid, chat_guid, kind, payload, state, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)
             ON CONFLICT(temp_guid) DO UPDATE SET state = 0, error = NULL",
            params![temp_guid, chat_guid, kind, payload, now],
        )?;
        Ok(())
    }

    pub fn outbox(&self) -> Result<Vec<OutboxItem>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, temp_guid, chat_guid, kind, payload, state, attempts, created_at
             FROM outbox ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(OutboxItem {
                id: r.get(0)?,
                temp_guid: r.get(1)?,
                chat_guid: r.get(2)?,
                kind: r.get(3)?,
                payload: r.get(4)?,
                state: r.get(5)?,
                attempts: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn outbox_set_state(&self, temp_guid: &str, state: i64, error: Option<&str>) -> Result<()> {
        self.lock().execute(
            "UPDATE outbox SET state = ?2, error = ?3,
                 attempts = attempts + (CASE WHEN ?2 = 1 THEN 1 ELSE 0 END)
             WHERE temp_guid = ?1",
            params![temp_guid, state, error],
        )?;
        Ok(())
    }

    pub fn outbox_remove(&self, temp_guid: &str) -> Result<()> {
        self.lock()
            .execute("DELETE FROM outbox WHERE temp_guid = ?1", [temp_guid])?;
        Ok(())
    }
}

/// Finds a chat by GUID. On macOS 26 the server names chats `any;-;…`
/// while older caches (and requests) use `iMessage;-;…`: either matches
/// the other when only one chat has that address part.
pub(crate) fn find_chat(tx: &Transaction, guid: &str) -> Result<Option<i64>> {
    if let Some(id) = tx
        .query_row("SELECT id FROM chats WHERE guid = ?1", [guid], |r| r.get(0))
        .optional()?
    {
        return Ok(Some(id));
    }
    let Some((service, rest)) = guid.split_once(';') else {
        return Ok(None);
    };
    let pattern = format!("%;{}", rest.replace('%', "\\%").replace('_', "\\_"));
    let candidates: Vec<(i64, String)> = tx
        .prepare("SELECT id, guid FROM chats WHERE guid LIKE ?1 ESCAPE '\\'")?
        .query_map([&pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let matching: Vec<i64> = candidates
        .into_iter()
        .filter(|(_, g)| service == "any" || chat_service(g) == "any")
        .map(|(id, _)| id)
        .collect();
    Ok(if matching.len() == 1 {
        Some(matching[0])
    } else {
        None
    })
}

fn ensure_chat(tx: &Transaction, guid: &str, changes: &mut Changes) -> Result<i64> {
    if let Some(id) = find_chat(tx, guid)? {
        return Ok(id);
    }
    tx.execute("INSERT INTO chats (guid, complete) VALUES (?1, 0)", [guid])?;
    let id = tx.last_insert_rowid();
    changes.chats.insert(id);
    changes.incomplete_chats.push(guid.to_owned());
    Ok(id)
}

fn upsert_handle(tx: &Transaction, h: &Handle) -> Result<()> {
    if h.address.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO handles (address, service, server_rowid, country, uncanonicalized)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(address, service) DO UPDATE SET
             server_rowid = COALESCE(excluded.server_rowid, server_rowid),
             country = COALESCE(excluded.country, country),
             uncanonicalized = COALESCE(excluded.uncanonicalized, uncanonicalized)",
        params![
            h.address,
            h.service.clone().unwrap_or_default(),
            h.original_rowid,
            h.country,
            h.uncanonicalized_id
        ],
    )?;
    Ok(())
}

/// Upserts a chat. `full` chats come from the chat endpoints and carry
/// the authoritative name and participants.
fn upsert_chat(tx: &Transaction, chat: &Chat, full: bool) -> Result<i64> {
    let id = match find_chat(tx, &chat.guid)? {
        Some(id) => id,
        None => {
            tx.execute("INSERT INTO chats (guid) VALUES (?1)", [&chat.guid])?;
            tx.last_insert_rowid()
        }
    };
    let participants = chat.participants.as_ref().filter(|p| !p.is_empty());
    tx.execute(
        "UPDATE chats SET
             style = COALESCE(?2, style),
             chat_identifier = COALESCE(?3, chat_identifier),
             display_name = CASE WHEN ?4 THEN ?5 ELSE COALESCE(NULLIF(?5, ''), display_name) END,
             is_archived = COALESCE(?6, is_archived),
             complete = complete OR ?7
         WHERE id = ?1",
        params![
            id,
            chat.style,
            chat.chat_identifier,
            full,
            chat.display_name.clone().filter(|n| !n.is_empty()),
            chat.is_archived,
            full && participants.is_some(),
        ],
    )?;
    if let Some(participants) = participants {
        tx.execute("DELETE FROM chat_participants WHERE chat_id = ?1", [id])?;
        for h in participants {
            upsert_handle(tx, h)?;
            tx.execute(
                "INSERT OR IGNORE INTO chat_participants (chat_id, address, service) VALUES (?1, ?2, ?3)",
                params![id, h.address, h.service.clone().unwrap_or_default()],
            )?;
        }
    }
    Ok(id)
}

fn service_for_chat(tx: &Transaction, chat: i64, guid: &str) -> Result<Option<String>> {
    let service = chat_service(guid);
    if service != "any" && !service.is_empty() {
        return Ok(Some(service.to_owned()));
    }
    Ok(tx
        .query_row(
            "SELECT service FROM chat_participants WHERE chat_id = ?1 AND service != '' LIMIT 1",
            [chat],
            |r| r.get(0),
        )
        .optional()?)
}

fn json(v: &Option<serde_json::Value>) -> Option<String> {
    v.as_ref().filter(|v| !v.is_null()).map(|v| v.to_string())
}

fn upsert_message(
    tx: &Transaction,
    hint: Option<&str>,
    m: &Message,
    changes: &mut Changes,
) -> Result<()> {
    if m.guid.is_empty() {
        return Ok(());
    }
    // A message already stored (or our pending row) decides the chat;
    // otherwise its payload or the caller does.
    let by_guid: Option<(i64, i64)> = tx
        .query_row(
            "SELECT id, chat_id FROM messages WHERE guid = ?1",
            [&m.guid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let pending: Option<(i64, i64)> = match m.temp_guid.as_deref() {
        Some(temp) => tx
            .query_row(
                "SELECT id, chat_id FROM messages WHERE guid = ?1 AND temp_guid = ?1",
                [temp],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?,
        // The full echo of our own send carries no temp GUID and can beat
        // the reply that does; adopt the matching pending row.
        None if m.is_from_me && by_guid.is_none() => adoptable_pending(tx, m)?,
        None => None,
    };
    let existing = match (by_guid, pending) {
        (Some(real), Some((temp_id, chat))) => {
            // Both rows exist: fold ours into the server's.
            tx.execute(
                "UPDATE attachments SET local_path = COALESCE(local_path,
                     (SELECT t.local_path FROM attachments t
                      WHERE t.message_id = ?2 AND t.position = attachments.position))
                 WHERE message_id = ?1",
                params![real.0, temp_id],
            )?;
            let temp: String =
                tx.query_row("SELECT guid FROM messages WHERE id = ?1", [temp_id], |r| {
                    r.get(0)
                })?;
            tx.execute("DELETE FROM messages WHERE id = ?1", [temp_id])?;
            changes.message(chat, temp_id);
            changes.resolved.push((temp, m.guid.clone()));
            Some(real)
        }
        (Some(real), None) => Some(real),
        (None, pending) => pending,
    };
    let chat_guid = m.chats.first().map(|c| c.guid.as_str()).or(hint);
    let chat = match (existing, chat_guid) {
        (Some((_, chat)), _) => chat,
        (None, Some(guid)) => ensure_chat(tx, guid, changes)?,
        (None, None) => {
            return Err(StoreError::Invalid(format!(
                "message {} has no chat",
                m.guid
            )));
        }
    };
    // Message payloads carry slim chats (no participants); they may still
    // fill in a name or style for a stub.
    for c in &m.chats {
        upsert_chat(tx, c, false)?;
    }
    if let Some(h) = &m.handle {
        upsert_handle(tx, h)?;
    }
    let sender: Option<String> = if m.is_from_me {
        None
    } else if let Some(h) = m.handle.as_ref().filter(|h| !h.address.is_empty()) {
        Some(h.address.clone())
    } else if let Some(rowid) = m.handle_id.filter(|&r| r > 0) {
        tx.query_row(
            "SELECT address FROM handles WHERE server_rowid = ?1",
            [rowid],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    let service = m
        .handle
        .as_ref()
        .and_then(|h| h.service.clone())
        .filter(|s| !s.is_empty());
    let service = match service {
        Some(s) => Some(s),
        None => {
            let guid: String =
                tx.query_row("SELECT guid FROM chats WHERE id = ?1", [chat], |r| r.get(0))?;
            service_for_chat(tx, chat, &guid)?
        }
    };
    let (associated_guid, associated_part) = match m.associated_message_guid.as_deref() {
        Some(raw) if !raw.is_empty() => {
            let (guid, part) = split_part_guid(raw);
            (Some(guid), m.associated_message_part.or(part).or(Some(0)))
        }
        _ => (None, None),
    };
    let other_handle = m.other_handle.filter(|&h| h > 0);
    let error = m.error.unwrap_or(0);
    let send_state = if error > 0 && m.is_from_me {
        SEND_FAILED
    } else {
        SEND_SENT
    };
    let p = named_params! {
        ":guid": m.guid,
        ":chat": chat,
        ":rowid": m.original_rowid,
        ":sender": sender,
        ":service": service,
        ":from_me": m.is_from_me,
        ":text": m.text,
        ":subject": m.subject,
        ":created": m.date_created,
        ":delivered": m.date_delivered,
        ":read": m.date_read,
        ":edited": m.date_edited,
        ":retracted": m.date_retracted,
        ":played": m.date_played,
        ":is_delivered": m.is_delivered,
        ":error": error,
        ":item_type": m.item_type.unwrap_or(0),
        ":action": m.group_action_type.unwrap_or(0),
        ":group_title": m.group_title,
        ":other_handle": other_handle,
        ":balloon": m.balloon_bundle_id,
        ":assoc_guid": associated_guid,
        ":assoc_type": m.associated_message_type,
        ":assoc_part": associated_part,
        ":thread_guid": m.thread_originator_guid,
        ":thread_part": m.thread_originator_part,
        ":effect": m.expressive_send_style_id,
        ":body": json(&m.attributed_body),
        ":summary": json(&m.message_summary_info),
        ":payload": json(&m.payload_data),
        ":part_count": m.part_count,
        ":quietly": m.was_delivered_quietly,
        ":notify": m.did_notify_recipient,
        ":audio": m.is_audio_message,
        ":send_state": send_state,
    };
    let id = match existing {
        Some((id, _)) => {
            let renamed: Option<String> = tx
                .query_row(
                    "SELECT guid FROM messages WHERE id = ?1 AND guid != ?2",
                    params![id, m.guid],
                    |r| r.get(0),
                )
                .optional()?;
            // Dates only move forward; a field the payload leaves out keeps
            // its value (slim payloads leave out most of them).
            tx.execute(
                "UPDATE messages SET
                     guid = :guid,
                     server_rowid = COALESCE(:rowid, server_rowid),
                     sender = COALESCE(:sender, sender),
                     service = COALESCE(:service, service),
                     text = COALESCE(:text, text),
                     subject = COALESCE(:subject, subject),
                     date_created = COALESCE(:created, date_created),
                     date_delivered = NULLIF(MAX(COALESCE(:delivered, 0), COALESCE(date_delivered, 0)), 0),
                     date_read = NULLIF(MAX(COALESCE(:read, 0), COALESCE(date_read, 0)), 0),
                     date_edited = NULLIF(MAX(COALESCE(:edited, 0), COALESCE(date_edited, 0)), 0),
                     date_retracted = NULLIF(MAX(COALESCE(:retracted, 0), COALESCE(date_retracted, 0)), 0),
                     date_played = NULLIF(MAX(COALESCE(:played, 0), COALESCE(date_played, 0)), 0),
                     is_delivered = MAX(COALESCE(:is_delivered, 0), COALESCE(is_delivered, 0)),
                     error = :error,
                     item_type = :item_type,
                     group_action_type = :action,
                     group_title = COALESCE(:group_title, group_title),
                     other_handle = COALESCE(:other_handle, other_handle),
                     balloon_bundle_id = COALESCE(:balloon, balloon_bundle_id),
                     associated_guid = COALESCE(:assoc_guid, associated_guid),
                     associated_type = COALESCE(:assoc_type, associated_type),
                     associated_part = COALESCE(:assoc_part, associated_part),
                     thread_originator_guid = COALESCE(:thread_guid, thread_originator_guid),
                     thread_originator_part = COALESCE(:thread_part, thread_originator_part),
                     effect = COALESCE(:effect, effect),
                     attributed_body = COALESCE(:body, attributed_body),
                     summary_info = COALESCE(:summary, summary_info),
                     payload_data = COALESCE(:payload, payload_data),
                     part_count = COALESCE(:part_count, part_count),
                     delivered_quietly = COALESCE(:quietly, delivered_quietly),
                     did_notify_recipient = COALESCE(:notify, did_notify_recipient),
                     is_audio = COALESCE(:audio, is_audio),
                     send_state = :send_state,
                     send_error = CASE WHEN :send_state = 0 THEN NULL ELSE send_error END
                 WHERE id = :id AND chat_id = :chat",
                {
                    let mut all: Vec<(&str, &dyn rusqlite::ToSql)> =
                        p.iter().filter(|(name, _)| *name != ":from_me").copied().collect();
                    all.push((":id", &id));
                    all
                }
                .as_slice(),
            )?;
            if let Some(temp) = renamed {
                changes.resolved.push((temp, m.guid.clone()));
            }
            id
        }
        None => {
            tx.execute(
                "INSERT INTO messages (guid, chat_id, server_rowid, sender, service, is_from_me,
                     text, subject, date_created, date_delivered, date_read, date_edited,
                     date_retracted, date_played, is_delivered, error, item_type,
                     group_action_type, group_title, other_handle, balloon_bundle_id,
                     associated_guid, associated_type, associated_part, thread_originator_guid,
                     thread_originator_part, effect, attributed_body, summary_info, payload_data,
                     part_count, delivered_quietly, did_notify_recipient, is_audio, send_state)
                 VALUES (:guid, :chat, :rowid, :sender, :service, :from_me, :text, :subject,
                     COALESCE(:created, 0), :delivered, :read, :edited, :retracted, :played,
                     :is_delivered, :error, :item_type, :action, :group_title, :other_handle,
                     :balloon, :assoc_guid, :assoc_type, :assoc_part, :thread_guid, :thread_part,
                     :effect, :body, :summary, :payload, :part_count, :quietly, :notify, :audio,
                     :send_state)",
                p,
            )?;
            let id = tx.last_insert_rowid();
            changes.inserted.push(id);
            id
        }
    };
    upsert_attachments(tx, id, &m.attachments)?;
    changes.message(chat, id);
    Ok(())
}

/// A pending send in the message's chat that `m` is the server's copy of:
/// the oldest one with the same text, or with attachments when `m` has
/// some. Sends in a chat are sequential, so the oldest match is the one.
fn adoptable_pending(tx: &Transaction, m: &Message) -> Result<Option<(i64, i64)>> {
    let Some(chat_guid) = m.chats.first().map(|c| c.guid.as_str()) else {
        return Ok(None);
    };
    let Some(chat) = find_chat(tx, chat_guid)? else {
        return Ok(None);
    };
    let text = m.text.as_deref().map(str::trim).unwrap_or_default();
    let has_attachments = !m.attachments.is_empty();
    let found = tx
        .query_row(
            "SELECT m.id FROM messages m
             WHERE m.chat_id = ?1 AND m.guid = m.temp_guid AND m.is_from_me = 1
               AND m.associated_guid IS NULL
               AND ((?3 AND EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id))
                    OR (NOT ?3 AND TRIM(COALESCE(m.text, '')) = ?2 AND ?2 != ''))
             ORDER BY m.date_created, m.id LIMIT 1",
            params![chat, text, has_attachments],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    Ok(found.map(|id| (id, chat)))
}

fn upsert_attachments(tx: &Transaction, message: i64, attachments: &[Attachment]) -> Result<()> {
    if attachments.is_empty() {
        return Ok(());
    }
    // Our pending attachments (temp GUIDs) give way to the server's, which
    // inherit the local file at the same position.
    let pending: Vec<(String, i64, Option<String>)> = tx
        .prepare(
            "SELECT guid, position, local_path FROM attachments
             WHERE message_id = ?1 AND guid LIKE 'temp-%'",
        )?
        .query_map([message], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (guid, _, _) in &pending {
        tx.execute("DELETE FROM attachments WHERE guid = ?1", [guid])?;
    }
    for (position, a) in attachments.iter().enumerate() {
        if a.guid.is_empty() {
            continue;
        }
        let local = pending
            .iter()
            .find(|(_, p, _)| *p == position as i64)
            .and_then(|(_, _, path)| path.clone());
        tx.execute(
            "INSERT INTO attachments (guid, message_id, position, uti, mime_type, transfer_name,
                 total_bytes, width, height, is_sticker, has_live_photo, hidden, transfer_state,
                 local_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(guid) DO UPDATE SET
                 message_id = excluded.message_id,
                 position = excluded.position,
                 uti = COALESCE(excluded.uti, uti),
                 mime_type = COALESCE(excluded.mime_type, mime_type),
                 transfer_name = COALESCE(excluded.transfer_name, transfer_name),
                 total_bytes = COALESCE(excluded.total_bytes, total_bytes),
                 width = COALESCE(NULLIF(excluded.width, 0), width),
                 height = COALESCE(NULLIF(excluded.height, 0), height),
                 is_sticker = excluded.is_sticker OR is_sticker,
                 has_live_photo = excluded.has_live_photo OR has_live_photo,
                 hidden = excluded.hidden,
                 transfer_state = COALESCE(excluded.transfer_state, transfer_state),
                 local_path = COALESCE(local_path, excluded.local_path)",
            params![
                a.guid,
                message,
                position as i64,
                a.uti,
                a.mime_type,
                a.transfer_name,
                a.total_bytes,
                a.width.filter(|&w| w > 0),
                a.height.filter(|&h| h > 0),
                a.is_sticker.unwrap_or(false),
                a.has_live_photo.unwrap_or(false),
                a.hide_attachment.unwrap_or(false),
                a.transfer_state,
                local,
            ],
        )?;
    }
    Ok(())
}

fn set_read(tx: &Transaction, chat: i64, read: bool) -> Result<()> {
    if read {
        // Read up to the newest message we have, not the wall clock, so a
        // skewed clock can't hide later messages.
        tx.execute(
            "UPDATE chats SET marked_unread = 0,
                 read_at = MAX(read_at, COALESCE(
                     (SELECT MAX(date_created) FROM messages WHERE chat_id = ?1), 0))
             WHERE id = ?1",
            [chat],
        )?;
    } else {
        tx.execute("UPDATE chats SET marked_unread = 1 WHERE id = ?1", [chat])?;
    }
    Ok(())
}

/// Derives the chat's last message and unread state from its messages.
pub(crate) fn recompute_chat(tx: &Transaction, chat: i64) -> Result<()> {
    let last: Option<(i64, i64)> = tx
        .query_row(
            "SELECT id, date_created FROM messages WHERE chat_id = ?1
             ORDER BY date_created DESC, id DESC LIMIT 1",
            [chat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    // Unread: the newest message from someone else is newer than anything
    // we sent and than our last read, and Messages hasn't marked it read.
    let incoming: Option<(i64, Option<i64>)> = tx
        .query_row(
            "SELECT date_created, date_read FROM messages
             WHERE chat_id = ?1 AND is_from_me = 0 AND item_type = 0
               AND associated_guid IS NULL
             ORDER BY date_created DESC, id DESC LIMIT 1",
            [chat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (mine, read_at, marked): (i64, i64, bool) = tx.query_row(
        "SELECT COALESCE((SELECT MAX(date_created) FROM messages
                          WHERE chat_id = ?1 AND is_from_me = 1), 0),
                read_at, marked_unread
         FROM chats WHERE id = ?1",
        [chat],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let unread = marked
        || incoming.is_some_and(|(date, date_read)| {
            date > mine.max(read_at) && date_read.is_none_or(|d| d <= 0)
        });
    tx.execute(
        "UPDATE chats SET last_message_id = ?2, last_date = COALESCE(?3, last_date), unread = ?4
         WHERE id = ?1",
        params![chat, last.map(|l| l.0), last.map(|l| l.1), unread],
    )?;
    Ok(())
}
