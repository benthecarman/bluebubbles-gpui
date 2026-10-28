//! The local cache: one SQLite file per server, and the only state views
//! read.
//!
//! Every input (sync pages, socket events, send replies) is applied with
//! [`Store::apply`] in one transaction, which reports what changed. Messages
//! are upserted by GUID and dates only move forward, so the same message
//! can arrive twice, out of order, or through both the socket and a sync
//! without harm. Chat summaries (last message, unread) are derived from the
//! messages rather than kept as separate flags that can drift.

mod read;
mod schema;
mod write;

pub use read::{AttachmentRow, ChatRow, ContactRow, MessageRow, PageCursor};
pub use write::{
    Batch, ContactInput, OUTBOX_FAILED, OUTBOX_IN_FLIGHT, OUTBOX_QUEUED, OutboxItem,
    OutgoingAttachment, OutgoingMessage, SEND_FAILED, SEND_PENDING, SEND_SENT,
};

use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Every write transaction takes the write lock when it begins. A deferred
/// one that reads first and then writes fails at once with "database is
/// locked" if another connection wrote in between, instead of waiting out
/// the busy timeout.
pub(crate) fn write_transaction(
    conn: &mut Connection,
) -> rusqlite::Result<rusqlite::Transaction<'_>> {
    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
}

/// What one write changed, for views to re-read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    /// Chats whose row or summary changed.
    pub chats: BTreeSet<i64>,
    /// Messages added or changed, by chat.
    pub messages: BTreeMap<i64, BTreeSet<i64>>,
    /// Messages seen for the first time (not echoes of our own sends).
    pub inserted: Vec<i64>,
    /// Temp GUIDs that now have their server GUID: `(temp, real)`.
    pub resolved: Vec<(String, String)>,
    /// Chats we only know from a message; fetch them.
    pub incomplete_chats: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.chats.is_empty() && self.messages.is_empty() && self.resolved.is_empty()
    }

    pub fn merge(&mut self, other: Changes) {
        self.chats.extend(other.chats);
        for (chat, ids) in other.messages {
            self.messages.entry(chat).or_default().extend(ids);
        }
        self.inserted.extend(other.inserted);
        self.resolved.extend(other.resolved);
        for guid in other.incomplete_chats {
            if !self.incomplete_chats.contains(&guid) {
                self.incomplete_chats.push(guid);
            }
        }
    }

    fn message(&mut self, chat: i64, id: i64) {
        self.chats.insert(chat);
        self.messages.entry(chat).or_default().insert(id);
    }
}

/// A handle to one database file. Clones share the connection; open a
/// second [`Store`] on the same file for reads on another thread.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Invalid(e.to_string()))?;
        }
        Self::setup(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut conn = conn;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite consistent (the
        // transaction rolls back), so a poisoned lock is still usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO kv(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_i64(&self, key: &str) -> Result<Option<i64>> {
        Ok(self.get(key)?.and_then(|v| v.parse().ok()))
    }

    pub fn set_i64(&self, key: &str, value: i64) -> Result<()> {
        self.set(key, &value.to_string())
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let version = usize::try_from(version).unwrap_or(0);
    if version > schema::MIGRATIONS.len() {
        return Err(StoreError::Invalid(format!(
            "database version {version} is newer than this app"
        )));
    }
    for (i, sql) in schema::MIGRATIONS.iter().enumerate().skip(version) {
        let tx = write_transaction(conn)?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

/// `p:0/GUID` and `bp:GUID` name a message part; returns the GUID and the
/// part, if it says one.
pub fn split_part_guid(raw: &str) -> (String, Option<i64>) {
    if let Some(rest) = raw.strip_prefix("bp:") {
        return (rest.to_owned(), None);
    }
    if let Some(rest) = raw.strip_prefix("p:")
        && let Some((part, guid)) = rest.split_once('/')
    {
        return (guid.to_owned(), part.parse().ok());
    }
    (raw.to_owned(), None)
}

/// The service a chat GUID names: `iMessage`, `SMS`, `RCS` or `any`.
pub fn chat_service(guid: &str) -> &str {
    guid.split(';').next().unwrap_or_default()
}

#[cfg(test)]
mod tests;
