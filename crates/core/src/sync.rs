//! First sync, catch-up and reconcile. The server keeps no event log, so
//! after any gap the engine asks for new rows by ROWID and for rows whose
//! receipts or edits changed since the last sync.

use crate::address::{self, Region};
use crate::engine::{EngineError, Internal};
use crate::now_ms;
use bb_api::{Client, Message, MessageQuery, WhereClause};
use bb_store::{Batch, ContactInput, Store, StoreError};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// The highest server ROWID every earlier row of which we have.
pub(crate) const ROWID_KEY: &str = "sync.rowid";
/// When the last sync started (ms).
pub(crate) const RECONCILE_KEY: &str = "sync.reconciled_at";
pub(crate) const CONTACTS_KEY: &str = "sync.contacts_at";

/// 2001-01-01 in Unix ms: `where` clauses compare Apple-epoch nanoseconds.
const APPLE_EPOCH_MS: i64 = 978_307_200_000;

const CHAT_PAGE: u32 = 200;
const MESSAGE_PAGE: u32 = 500;
/// After a very long gap, stop here; older rows arrive with history.
const CATCH_UP_LIMIT: u32 = 20_000;
/// Receipts can carry the Mac's clock; look this far behind.
const RECONCILE_OVERLAP: i64 = 10 * 60_000;
const PREFETCH_CHATS: usize = 10;
pub(crate) const HISTORY_PAGE: u32 = 50;

pub(crate) struct Sync {
    pub api: Client,
    pub store: Store,
    pub region: Region,
    pub tx: UnboundedSender<Internal>,
    /// Messages created before this don't notify.
    pub notify_since: i64,
    pub macos: Option<(u32, u32)>,
    pub contacts_every: Duration,
}

impl Sync {
    pub async fn run(self) -> Result<(), EngineError> {
        let started = now_ms();
        if self.store.get_i64(ROWID_KEY)?.is_none() {
            self.first_sync().await?;
        }
        self.catch_up().await?;
        self.reconcile(started).await?;
        self.store.set_i64(RECONCILE_KEY, started)?;
        if let Err(e) = self.contacts().await {
            tracing::warn!("contacts sync failed: {e}");
        }
        Ok(())
    }

    fn apply(&self, batch: Batch) -> Result<(), EngineError> {
        self.apply_with(batch, self.notify_since)
    }

    /// Applies without notifying: what the first sync finds is old news.
    fn apply_quiet(&self, batch: Batch) -> Result<(), EngineError> {
        self.apply_with(batch, i64::MAX)
    }

    fn apply_with(&self, batch: Batch, notify_since: i64) -> Result<(), EngineError> {
        let changes = self.store.apply(batch)?;
        let _ = self.tx.send(Internal::Applied {
            changes,
            notify_since,
        });
        Ok(())
    }

    async fn first_sync(&self) -> Result<(), EngineError> {
        // The cursor comes first, so rows that arrive while the chat list
        // pages in are picked up by the catch-up that follows.
        let mut newest = MessageQuery::new(1);
        newest.sort = "DESC".into();
        newest.with = Vec::new();
        let cursor = self
            .api
            .query_messages(&newest)
            .await?
            .items
            .first()
            .and_then(|m| m.original_rowid)
            .unwrap_or(0);
        let mut offset = 0;
        loop {
            let page = self.api.chats(offset, CHAT_PAGE).await?;
            let n = page.items.len() as u32;
            self.apply_quiet(Batch {
                chats: page.items,
                ..Default::default()
            })?;
            if n < CHAT_PAGE {
                break;
            }
            offset += n;
        }
        self.store.set_i64(ROWID_KEY, cursor)?;
        for chat in self.store.chats(false)?.into_iter().take(PREFETCH_CHATS) {
            let messages = self.api.chat_messages(&chat.guid, None, 25).await?;
            let done = messages.len() < 25;
            self.apply_quiet(history_batch(&chat.guid, messages, done))?;
        }
        Ok(())
    }

    async fn catch_up(&self) -> Result<(), EngineError> {
        let cursor = self.store.get_i64(ROWID_KEY)?.unwrap_or(0);
        let mut newest = cursor;
        let mut offset = 0;
        // Pages sort by date, not ROWID, so the cursor only moves once
        // every page is in; offsets keep the window fixed meanwhile.
        loop {
            let mut query = MessageQuery::new(MESSAGE_PAGE);
            query.r#where.push(WhereClause {
                statement: "message.ROWID > :rowid".into(),
                args: json!({ "rowid": cursor }),
            });
            query.offset = offset;
            let page = self.api.query_messages(&query).await?;
            let n = page.items.len() as u32;
            newest = page
                .items
                .iter()
                .filter_map(|m| m.original_rowid)
                .fold(newest, i64::max);
            self.apply(Batch {
                messages: page.items.into_iter().map(|m| (None, m)).collect(),
                ..Default::default()
            })?;
            offset += n;
            if n < MESSAGE_PAGE || offset >= CATCH_UP_LIMIT {
                break;
            }
        }
        self.store.set_i64(ROWID_KEY, newest)?;
        Ok(())
    }

    async fn reconcile(&self, started: i64) -> Result<(), EngineError> {
        let since = self.store.get_i64(RECONCILE_KEY)?.unwrap_or(started) - RECONCILE_OVERLAP;
        let apple_ns = (since - APPLE_EPOCH_MS).max(0) * 1_000_000;
        let mut columns = vec!["date_delivered", "date_read"];
        // Edits and unsends exist from macOS 13.
        if self.macos.is_some_and(|v| v >= (13, 0)) {
            columns.extend(["date_edited", "date_retracted"]);
        }
        let statement = format!(
            "({})",
            columns
                .iter()
                .map(|c| format!("message.{c} >= :since"))
                .collect::<Vec<_>>()
                .join(" OR ")
        );
        let mut offset = 0;
        loop {
            let mut query = MessageQuery::new(MESSAGE_PAGE);
            query.r#where.push(WhereClause {
                statement: statement.clone(),
                args: json!({ "since": apple_ns }),
            });
            query.offset = offset;
            let page = self.api.query_messages(&query).await?;
            let n = page.items.len() as u32;
            // Catch-up brings in new rows; here only changes to rows that
            // keep history unbroken (an old receipt or edit is not one).
            let mut fit = Vec::new();
            for m in page.items {
                if fits_history(&self.store, &m)? {
                    fit.push((None, m));
                }
            }
            self.apply(Batch {
                messages: fit,
                ..Default::default()
            })?;
            offset += n;
            if n < MESSAGE_PAGE || offset >= CATCH_UP_LIMIT {
                break;
            }
        }
        Ok(())
    }

    async fn contacts(&self) -> Result<(), EngineError> {
        let last = self.store.get_i64(CONTACTS_KEY)?.unwrap_or(0);
        if now_ms() - last < self.contacts_every.as_millis() as i64 {
            return Ok(());
        }
        let contacts = self.api.contacts(true).await?;
        let inputs: Vec<ContactInput> = contacts
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let mut addresses: Vec<String> = c
                    .phone_numbers
                    .iter()
                    .chain(&c.emails)
                    .map(|a| address::normalize(&a.address, self.region))
                    .filter(|a| !a.is_empty())
                    .collect();
                addresses.sort();
                addresses.dedup();
                ContactInput {
                    id: c.id.clone().unwrap_or_else(|| format!("contact-{i}")),
                    display_name: c.display_name.clone(),
                    first_name: c.first_name.clone(),
                    last_name: c.last_name.clone(),
                    nickname: c.nickname.clone(),
                    addresses,
                    avatar: c.avatar.as_deref().and_then(decode_base64),
                }
            })
            .collect();
        self.store.replace_contacts(&inputs)?;
        self.store.set_i64(CONTACTS_KEY, now_ms())?;
        let _ = self.tx.send(Internal::ContactsChanged);
        Ok(())
    }
}

/// See [`Store::fits_history`]; a message with no chat can't be placed.
pub(crate) fn fits_history(store: &Store, m: &Message) -> Result<bool, StoreError> {
    match m.chats.first() {
        Some(chat) => store.fits_history(&m.guid, &chat.guid, m.date_created.unwrap_or(0)),
        None => Ok(false),
    }
}

pub(crate) fn history_batch(chat_guid: &str, messages: Vec<Message>, done: bool) -> Batch {
    Batch {
        messages: messages
            .into_iter()
            .map(|m| (Some(chat_guid.to_owned()), m))
            .collect(),
        history_done: if done {
            vec![chat_guid.to_owned()]
        } else {
            Vec::new()
        },
        ..Default::default()
    }
}

/// Standard base64, ignoring whitespace and a `data:` URL prefix.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let s = s.split_once("base64,").map_or(s, |(_, rest)| rest);
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64() {
        assert_eq!(super::decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(
            super::decode_base64("data:image/png;base64,aGk=").unwrap(),
            b"hi"
        );
        assert_eq!(super::decode_base64("!!"), None);
    }
}
