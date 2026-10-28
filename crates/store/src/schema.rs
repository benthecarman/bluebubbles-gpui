//! Migrations, applied in order and recorded in `PRAGMA user_version`.

pub const MIGRATIONS: &[&str] = &[
    // 1: the cache and the outbox.
    r#"
CREATE TABLE kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE handles (
    address         TEXT NOT NULL,
    service         TEXT NOT NULL DEFAULT '',
    server_rowid    INTEGER,
    country         TEXT,
    uncanonicalized TEXT,
    PRIMARY KEY (address, service)
);
CREATE INDEX handles_rowid ON handles(server_rowid);

CREATE TABLE chats (
    id              INTEGER PRIMARY KEY,
    guid            TEXT NOT NULL UNIQUE,
    style           INTEGER,
    chat_identifier TEXT,
    display_name    TEXT,
    is_archived     INTEGER NOT NULL DEFAULT 0,
    -- Participants and names are known (a stub is made for a message in a
    -- chat we haven't fetched yet).
    complete        INTEGER NOT NULL DEFAULT 0,
    -- Derived from messages by `recompute_chat`.
    last_message_id INTEGER,
    last_date       INTEGER NOT NULL DEFAULT 0,
    unread          INTEGER NOT NULL DEFAULT 0,
    -- Local read state: when this client last saw the chat read, and a
    -- "marked unread" flag from the user or the server.
    read_at         INTEGER NOT NULL DEFAULT 0,
    marked_unread   INTEGER NOT NULL DEFAULT 0,
    muted           INTEGER NOT NULL DEFAULT 0,
    pinned          INTEGER,
    draft           TEXT,
    -- The oldest page of history has been fetched.
    history_done    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX chats_recent ON chats(last_date DESC);

CREATE TABLE chat_participants (
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    service TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (chat_id, address)
);

CREATE TABLE messages (
    id                     INTEGER PRIMARY KEY,
    guid                   TEXT NOT NULL UNIQUE,
    temp_guid              TEXT UNIQUE,
    chat_id                INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    server_rowid           INTEGER,
    sender                 TEXT,
    service                TEXT,
    is_from_me             INTEGER NOT NULL,
    text                   TEXT,
    subject                TEXT,
    date_created           INTEGER NOT NULL,
    date_delivered         INTEGER,
    date_read              INTEGER,
    date_edited            INTEGER,
    date_retracted         INTEGER,
    date_played            INTEGER,
    is_delivered           INTEGER,
    error                  INTEGER NOT NULL DEFAULT 0,
    item_type              INTEGER NOT NULL DEFAULT 0,
    group_action_type      INTEGER NOT NULL DEFAULT 0,
    group_title            TEXT,
    other_handle           INTEGER,
    balloon_bundle_id      TEXT,
    associated_guid        TEXT,
    associated_type        TEXT,
    associated_part        INTEGER,
    thread_originator_guid TEXT,
    thread_originator_part TEXT,
    effect                 TEXT,
    attributed_body        TEXT,
    summary_info           TEXT,
    payload_data           TEXT,
    part_count             INTEGER,
    delivered_quietly      INTEGER,
    did_notify_recipient   INTEGER,
    is_audio               INTEGER,
    -- 0 on the server, 1 queued or sending, 2 failed to send.
    send_state             INTEGER NOT NULL DEFAULT 0,
    send_error             TEXT,
    -- A desktop notification was shown (or deliberately skipped).
    notified               INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX messages_chat_date ON messages(chat_id, date_created, id);
CREATE INDEX messages_associated ON messages(associated_guid);
CREATE INDEX messages_thread ON messages(thread_originator_guid);
CREATE INDEX messages_rowid ON messages(server_rowid);

CREATE TABLE attachments (
    guid           TEXT PRIMARY KEY,
    message_id     INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,
    uti            TEXT,
    mime_type      TEXT,
    transfer_name  TEXT,
    total_bytes    INTEGER,
    width          INTEGER,
    height         INTEGER,
    is_sticker     INTEGER NOT NULL DEFAULT 0,
    has_live_photo INTEGER NOT NULL DEFAULT 0,
    hidden         INTEGER NOT NULL DEFAULT 0,
    transfer_state INTEGER,
    -- For an attachment this client is sending: the file on disk.
    local_path     TEXT
);
CREATE INDEX attachments_message ON attachments(message_id);

CREATE TABLE contacts (
    id           TEXT PRIMARY KEY,
    display_name TEXT,
    first_name   TEXT,
    last_name    TEXT,
    nickname     TEXT,
    avatar       BLOB
);

-- Addresses in their normalized form (E.164 phone numbers, lowercase
-- emails), so handles match contacts exactly.
CREATE TABLE contact_addresses (
    address    TEXT NOT NULL,
    contact_id TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    PRIMARY KEY (address, contact_id)
);

CREATE TABLE outbox (
    id         INTEGER PRIMARY KEY,
    temp_guid  TEXT NOT NULL UNIQUE,
    chat_guid  TEXT NOT NULL,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL,
    -- 0 queued, 1 in flight, 2 failed.
    state      INTEGER NOT NULL DEFAULT 0,
    attempts   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    error      TEXT
);
"#,
];
