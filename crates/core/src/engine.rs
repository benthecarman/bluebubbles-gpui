//! The engine: one Tokio task that owns the connection, applies every
//! input to the store, and tells views what changed.
//!
//! Network calls run in tasks of their own and report back to the loop,
//! so a slow upload or a long catch-up never holds up socket events.

use crate::address::Region;
use crate::names::Names;
use crate::now_ms;
use crate::outbox::{self, Job, Outbound, Outcome};
use crate::sync::{self, HISTORY_PAGE, Sync};
use crate::tapback::Tapback;
use bb_api::{ApiError, Chat, Client, Message, ServerConfig, ServerInfo, socket_password};
use bb_socket::{Backoff, Socket, SocketConfig, SocketError, SocketEvent};
use bb_store::{
    Batch, Changes, MessageRow, OUTBOX_FAILED, OUTBOX_IN_FLIGHT, OUTBOX_QUEUED, OutgoingAttachment,
    OutgoingMessage, Store, StoreError,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error(transparent)]
    Socket(#[from] SocketError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Delays and intervals, shortened in tests.
#[derive(Debug, Clone)]
pub struct Timings {
    /// How often timers are checked.
    pub tick: Duration,
    /// A typing bubble with no update disappears after this.
    pub typing_timeout: Duration,
    /// Stop telling the other side we're typing after this much idle.
    pub composing_idle: Duration,
    /// Wait this long before telling the server a chat was read.
    pub read_debounce: Duration,
    /// Catch up this often while connected, in case an event was lost.
    pub catch_up_every: Duration,
    /// While the socket is down, check the server over REST this often.
    pub offline_poll: Duration,
    /// Focus syncs closer together than this are skipped.
    pub focus_debounce: Duration,
    /// A send that can't reach the server waits this long, then fails.
    pub send_give_up: Duration,
    /// Contacts are re-fetched this often.
    pub contacts_every: Duration,
    /// Messages older than this when they arrive don't notify.
    pub notify_window: Duration,
    pub socket_backoff: Backoff,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            tick: Duration::from_millis(250),
            typing_timeout: Duration::from_secs(60),
            composing_idle: Duration::from_secs(3),
            read_debounce: Duration::from_millis(800),
            catch_up_every: Duration::from_secs(5 * 60),
            offline_poll: Duration::from_secs(30),
            focus_debounce: Duration::from_secs(15),
            send_give_up: Duration::from_secs(10 * 60),
            contacts_every: Duration::from_secs(6 * 60 * 60),
            notify_window: Duration::from_secs(15 * 60),
            socket_backoff: Backoff::default(),
        }
    }
}

pub struct EngineConfig {
    pub server: ServerConfig,
    pub store: Store,
    pub region: Region,
    pub timings: Timings,
}

impl EngineConfig {
    pub fn new(server: ServerConfig, store: Store) -> Self {
        Self {
            server,
            store,
            region: Region::from_locale(),
            timings: Timings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Connecting,
    /// Socket connected: events arrive live.
    Online,
    /// The socket is down but REST works; the engine polls.
    Polling,
    Offline,
    /// The server refused the password.
    Unauthorized,
}

/// What the server can do, from `/server/info`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Features {
    /// Tapbacks, replies, effects, typing, read receipts, edits.
    pub private_api: bool,
    /// Edit and unsend (macOS 13+ with the Private API).
    pub edits: bool,
    pub macos: Option<(u32, u32)>,
    pub server_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub connection: Connection,
    /// When the connection was last lost (ms), while it is down.
    pub offline_since: Option<i64>,
    pub syncing: bool,
    /// At least one sync has finished, so the chat list is complete.
    pub synced: bool,
    pub features: Features,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub chat_id: i64,
    pub chat_guid: String,
    pub message_id: i64,
    pub message_guid: String,
    /// The chat's title.
    pub title: String,
    /// The sender, in group chats.
    pub subtitle: Option<String>,
    pub body: String,
    pub sender: Option<String>,
    pub is_group: bool,
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Rows changed; re-read them.
    Changed(Arc<Changes>),
    /// Contact names or avatars changed; re-resolve names.
    ContactsChanged,
    Typing {
        chat_id: i64,
        typing: bool,
    },
    Notify(Notification),
    /// The chat was read (here or on another device): withdraw its
    /// notifications.
    ChatRead {
        chat_id: i64,
    },
    /// The server announced a new address.
    ServerMoved {
        url: String,
    },
    Status(Status),
}

/// A text message to send.
#[derive(Debug, Clone, Default)]
pub struct TextMessage {
    pub chat_guid: String,
    pub text: String,
    pub subject: Option<String>,
    /// An expressive send style, such as `com.apple.MobileSMS.expressivesend.impact`.
    pub effect: Option<String>,
    /// Reply to this message GUID and part.
    pub reply_to: Option<(String, i64)>,
}

#[derive(Debug)]
enum Command {
    SendText(TextMessage),
    SendFile {
        chat_guid: String,
        path: PathBuf,
        reply_to: Option<(String, i64)>,
    },
    React {
        chat_guid: String,
        target_guid: String,
        part: i64,
        tapback: Tapback,
        remove: bool,
    },
    Retry(i64),
    Discard(String),
    MarkRead(i64),
    MarkUnread(i64),
    Composing {
        chat_guid: String,
        active: bool,
    },
    LoadOlder(i64),
    View {
        chat_id: Option<i64>,
        focused: bool,
    },
    SyncNow,
    Reconnect,
    Shutdown,
}

pub(crate) enum Internal {
    Applied {
        changes: Changes,
        notify_since: i64,
    },
    ContactsChanged,
    SyncDone(Result<(), EngineError>),
    Info(Result<ServerInfo, ApiError>),
    Sent(Job, Outcome),
    Chat(String, Result<Chat, ApiError>),
    History {
        chat_id: i64,
        chat_guid: String,
        result: Result<Vec<Message>, ApiError>,
    },
}

/// A handle to the engine. Cheap to clone; the engine stops when the last
/// handle is dropped.
#[derive(Clone)]
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<Status>,
    store: Store,
    region: Region,
}

impl Engine {
    /// Starts the engine on the current Tokio runtime.
    pub fn spawn(
        config: EngineConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<EngineEvent>), EngineError> {
        let api = Client::new(&config.server)?;
        let mut socket_config = SocketConfig::new(config.server.url.clone());
        socket_config
            .query
            .push(("password".into(), socket_password(&config.server.password)));
        socket_config.headers = config.server.effective_headers();
        socket_config.backoff = config.timings.socket_backoff;
        let (socket, socket_events) = Socket::spawn(socket_config)?;
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        let (internal, internal_rx) = mpsc::unbounded_channel();
        let status = Status {
            connection: Connection::Connecting,
            offline_since: None,
            syncing: false,
            synced: config.store.get_i64(sync::ROWID_KEY)?.is_some(),
            features: Features::default(),
            last_error: None,
        };
        let (status_tx, status_rx) = watch::channel(status.clone());
        let last_online = config
            .store
            .get_i64(LAST_ONLINE_KEY)?
            .unwrap_or_else(now_ms);
        let mut core = Core {
            timings: config.timings,
            region: config.region,
            store: config.store.clone(),
            api,
            socket,
            events,
            status_tx,
            status,
            internal,
            info: None,
            syncing: false,
            sync_dirty: false,
            last_sync: None,
            last_online,
            last_online_saved: Instant::now(),
            poll_at: None,
            queues: HashMap::new(),
            busy: HashSet::new(),
            waiting: HashSet::new(),
            typing_in: HashMap::new(),
            typing_out: HashMap::new(),
            read_due: HashMap::new(),
            viewing: None,
            focused: false,
            fetching_chats: HashSet::new(),
            loading_history: HashSet::new(),
        };
        core.restore_outbox()?;
        tokio::spawn(core.run(command_rx, socket_events, internal_rx));
        Ok((
            Self {
                commands,
                status: status_rx,
                store: config.store,
                region: config.region,
            },
            event_rx,
        ))
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn region(&self) -> Region {
        self.region
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    pub fn watch_status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    fn command(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn send_text(&self, message: TextMessage) {
        self.command(Command::SendText(message));
    }

    pub fn send_file(&self, chat_guid: &str, path: PathBuf, reply_to: Option<(String, i64)>) {
        self.command(Command::SendFile {
            chat_guid: chat_guid.to_owned(),
            path,
            reply_to,
        });
    }

    /// Adds `tapback` to a message part, or takes it off with `remove`.
    pub fn react(
        &self,
        chat_guid: &str,
        target_guid: &str,
        part: i64,
        tapback: Tapback,
        remove: bool,
    ) {
        self.command(Command::React {
            chat_guid: chat_guid.to_owned(),
            target_guid: target_guid.to_owned(),
            part,
            tapback,
            remove,
        });
    }

    /// Sends a failed message again.
    pub fn retry(&self, message_id: i64) {
        self.command(Command::Retry(message_id));
    }

    /// Deletes a message that never reached the server.
    pub fn discard(&self, temp_guid: &str) {
        self.command(Command::Discard(temp_guid.to_owned()));
    }

    pub fn mark_read(&self, chat_id: i64) {
        self.command(Command::MarkRead(chat_id));
    }

    pub fn mark_unread(&self, chat_id: i64) {
        self.command(Command::MarkUnread(chat_id));
    }

    /// The composer has text (`active`) or was cleared.
    pub fn composing(&self, chat_guid: &str, active: bool) {
        self.command(Command::Composing {
            chat_guid: chat_guid.to_owned(),
            active,
        });
    }

    /// Fetches the page of history before the oldest cached message.
    pub fn load_older(&self, chat_id: i64) {
        self.command(Command::LoadOlder(chat_id));
    }

    /// What the user is looking at: incoming messages there are marked
    /// read and don't notify while the window is focused.
    pub fn view(&self, chat_id: Option<i64>, focused: bool) {
        self.command(Command::View { chat_id, focused });
    }

    /// Catch up now (the window gained focus).
    pub fn sync_now(&self) {
        self.command(Command::SyncNow);
    }

    /// Reconnect at once, also after the password was refused.
    pub fn reconnect(&self) {
        self.command(Command::Reconnect);
    }

    pub fn shutdown(&self) {
        self.command(Command::Shutdown);
    }
}

const LAST_ONLINE_KEY: &str = "engine.last_online";
/// The Mac's clock may run behind ours.
const CLOCK_SLACK: i64 = 2 * 60_000;

struct Core {
    timings: Timings,
    region: Region,
    store: Store,
    api: Client,
    socket: Socket,
    events: mpsc::UnboundedSender<EngineEvent>,
    status_tx: watch::Sender<Status>,
    status: Status,
    internal: mpsc::UnboundedSender<Internal>,
    info: Option<ServerInfo>,

    syncing: bool,
    sync_dirty: bool,
    last_sync: Option<Instant>,
    /// The last time we were connected (ms), for notification cut-offs.
    last_online: i64,
    last_online_saved: Instant,
    /// When to check the server over REST while the socket is down.
    poll_at: Option<Instant>,

    /// Sends waiting, per chat; one in flight per chat.
    queues: HashMap<String, VecDeque<Job>>,
    busy: HashSet<String>,
    /// Chats whose queue waits for the server to be reachable.
    waiting: HashSet<String>,

    typing_in: HashMap<i64, Instant>,
    typing_out: HashMap<String, Instant>,
    read_due: HashMap<String, Instant>,
    viewing: Option<i64>,
    focused: bool,
    fetching_chats: HashSet<String>,
    loading_history: HashSet<i64>,
}

impl Core {
    async fn run(
        mut self,
        mut commands: mpsc::UnboundedReceiver<Command>,
        mut socket_events: mpsc::UnboundedReceiver<SocketEvent>,
        mut internal: mpsc::UnboundedReceiver<Internal>,
    ) {
        let mut tick = tokio::time::interval(self.timings.tick);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    None | Some(Command::Shutdown) => break,
                    Some(command) => self.on_command(command),
                },
                Some(event) = socket_events.recv() => self.on_socket(event),
                Some(message) = internal.recv() => self.on_internal(message),
                _ = tick.tick() => self.on_tick(),
            }
        }
        self.socket.shutdown();
        if self.status.connection == Connection::Online {
            let _ = self.store.set_i64(LAST_ONLINE_KEY, now_ms());
        }
    }

    fn emit(&self, event: EngineEvent) {
        let _ = self.events.send(event);
    }

    fn set_status(&mut self, f: impl FnOnce(&mut Status)) {
        let before = self.status.clone();
        f(&mut self.status);
        if self.status != before {
            let _ = self.status_tx.send(self.status.clone());
            self.emit(EngineEvent::Status(self.status.clone()));
        }
    }

    fn report<T>(&mut self, what: &str, result: Result<T, StoreError>) -> Option<T> {
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::error!("{what}: {e}");
                self.set_status(|s| s.last_error = Some(format!("{what}: {e}")));
                None
            }
        }
    }

    fn private_api(&self) -> bool {
        self.status.features.private_api
    }

    // Commands from the UI.

    fn on_command(&mut self, command: Command) {
        match command {
            Command::SendText(m) => self.send_text(m),
            Command::SendFile {
                chat_guid,
                path,
                reply_to,
            } => self.send_file(chat_guid, path, reply_to),
            Command::React {
                chat_guid,
                target_guid,
                part,
                tapback,
                remove,
            } => self.react(chat_guid, target_guid, part, tapback, remove),
            Command::Retry(id) => self.retry(id),
            Command::Discard(temp) => {
                if let Some(changes) = self.report("discard", self.store.discard_outgoing(&temp)) {
                    let _ = self.store.outbox_remove(&temp);
                    for queue in self.queues.values_mut() {
                        queue.retain(|j| j.temp_guid != temp);
                    }
                    self.after_apply(changes, i64::MAX);
                }
            }
            Command::MarkRead(chat_id) => self.mark_read(chat_id),
            Command::MarkUnread(chat_id) => self.mark_unread(chat_id),
            Command::Composing { chat_guid, active } => self.composing(chat_guid, active),
            Command::LoadOlder(chat_id) => self.load_older(chat_id),
            Command::View { chat_id, focused } => {
                let was_focused = self.focused;
                self.viewing = chat_id;
                self.focused = focused;
                if focused && let Some(id) = chat_id {
                    let unread = self.store.chat(id).ok().flatten().is_some_and(|c| c.unread);
                    if unread {
                        self.mark_read(id);
                    }
                }
                if focused && !was_focused {
                    self.focus_sync();
                }
            }
            Command::SyncNow => self.focus_sync(),
            Command::Reconnect => {
                self.socket.reconnect_now();
                self.set_status(|s| {
                    if s.connection == Connection::Unauthorized {
                        s.connection = Connection::Connecting;
                    }
                });
                self.request_sync();
            }
            Command::Shutdown => {}
        }
    }

    fn new_temp_guid() -> String {
        format!("temp-{}", uuid::Uuid::new_v4())
    }

    fn queue_outgoing(&mut self, message: OutgoingMessage, outbound: Outbound) {
        let payload = serde_json::to_string(&outbound).unwrap_or_default();
        let Some((changes, _)) = self.report("send", self.store.insert_outgoing(&message)) else {
            return;
        };
        self.report(
            "outbox",
            self.store.outbox_push(
                &message.temp_guid,
                &message.chat_guid,
                outbound.kind(),
                &payload,
                message.date_created,
            ),
        );
        self.after_apply(changes, i64::MAX);
        self.enqueue(Job {
            temp_guid: message.temp_guid,
            chat_guid: message.chat_guid,
            outbound,
            created_at: message.date_created,
            attempts: 0,
        });
    }

    fn send_text(&mut self, m: TextMessage) {
        let text = m.text.trim_end().to_owned();
        if text.trim().is_empty() {
            return;
        }
        self.stop_composing(&m.chat_guid);
        let outgoing = OutgoingMessage {
            temp_guid: Self::new_temp_guid(),
            chat_guid: m.chat_guid.clone(),
            text: Some(text.clone()),
            subject: m.subject.clone(),
            date_created: now_ms(),
            effect: m.effect.clone(),
            reply_to: m.reply_to.clone(),
            tapback: None,
            attachments: Vec::new(),
        };
        self.queue_outgoing(
            outgoing,
            Outbound::Text {
                text,
                subject: m.subject,
                effect: m.effect,
                reply_to: m.reply_to,
            },
        );
    }

    fn send_file(&mut self, chat_guid: String, path: PathBuf, reply_to: Option<(String, i64)>) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let mime = outbox::mime_for(&name).to_owned();
        let size = std::fs::metadata(&path)
            .map(|m| m.len() as i64)
            .unwrap_or(0);
        let path_str = path.to_string_lossy().into_owned();
        let outgoing = OutgoingMessage {
            temp_guid: Self::new_temp_guid(),
            chat_guid: chat_guid.clone(),
            text: None,
            subject: None,
            date_created: now_ms(),
            effect: None,
            reply_to: reply_to.clone(),
            tapback: None,
            attachments: vec![OutgoingAttachment {
                guid: Self::new_temp_guid(),
                path: path_str.clone(),
                name: name.clone(),
                mime_type: mime.clone(),
                total_bytes: size,
                width: None,
                height: None,
            }],
        };
        self.queue_outgoing(
            outgoing,
            Outbound::Attachment {
                path: path_str,
                name,
                mime,
                reply_to,
            },
        );
    }

    fn react(
        &mut self,
        chat_guid: String,
        target: String,
        part: i64,
        tapback: Tapback,
        remove: bool,
    ) {
        let target_text = self
            .store
            .message_by_guid(&target)
            .ok()
            .flatten()
            .and_then(|m| m.text)
            .unwrap_or_default();
        let name = tapback.api_name();
        let reaction = if remove { format!("-{name}") } else { name };
        let text = if remove {
            format!("Removed a tapback from “{target_text}”")
        } else {
            format!("{} “{target_text}”", tapback.verb())
        };
        let outgoing = OutgoingMessage {
            temp_guid: Self::new_temp_guid(),
            chat_guid: chat_guid.clone(),
            text: Some(text),
            subject: None,
            date_created: now_ms(),
            effect: None,
            reply_to: None,
            tapback: Some((target.clone(), reaction.clone(), part)),
            attachments: Vec::new(),
        };
        self.queue_outgoing(
            outgoing,
            Outbound::React {
                target,
                part,
                reaction,
                target_text,
            },
        );
    }

    fn retry(&mut self, message_id: i64) {
        let Some(row) = self
            .report("retry", self.store.messages_by_id(&[message_id]))
            .and_then(|rows| rows.into_iter().next())
        else {
            return;
        };
        if !row.is_failed() {
            return;
        }
        let item = self.store.outbox().ok().and_then(|items| {
            items
                .into_iter()
                .find(|i| Some(&i.temp_guid) == row.temp_guid.as_ref())
        });
        if row.temp_guid.as_deref() == Some(row.guid.as_str())
            && let Some(item) = item
            && let Ok(outbound) = serde_json::from_str::<Outbound>(&item.payload)
        {
            // Never reached the server: send the same row again.
            let Some(changes) = self.report("retry", self.store.retry_outgoing(&row.guid)) else {
                return;
            };
            let _ = self
                .store
                .outbox_set_state(&item.temp_guid, OUTBOX_QUEUED, None);
            self.after_apply(changes, i64::MAX);
            self.enqueue(Job {
                temp_guid: item.temp_guid,
                chat_guid: item.chat_guid,
                outbound,
                created_at: now_ms(),
                attempts: item.attempts,
            });
        } else if let Some(text) = row.text.clone().filter(|t| !t.trim().is_empty()) {
            // Messages refused it: send a copy. The failed one stays on the
            // Mac too.
            let chat_guid = self.store.chat(row.chat_id).ok().flatten().map(|c| c.guid);
            if let Some(chat_guid) = chat_guid {
                self.send_text(TextMessage {
                    chat_guid,
                    text,
                    ..Default::default()
                });
            }
        }
    }

    fn mark_read(&mut self, chat_id: i64) {
        let Some(chat) = self.store.chat(chat_id).ok().flatten() else {
            return;
        };
        if let Some(changes) = self.report("mark read", self.store.mark_read_local(chat_id)) {
            self.after_apply(changes, i64::MAX);
        }
        self.emit(EngineEvent::ChatRead { chat_id });
        self.read_due
            .insert(chat.guid, Instant::now() + self.timings.read_debounce);
    }

    fn mark_unread(&mut self, chat_id: i64) {
        let Some(chat) = self.store.chat(chat_id).ok().flatten() else {
            return;
        };
        self.read_due.remove(&chat.guid);
        let batch = Batch {
            read_states: vec![(chat.guid.clone(), false)],
            ..Default::default()
        };
        if let Some(changes) = self.report("mark unread", self.store.apply(batch)) {
            self.after_apply(changes, i64::MAX);
        }
        if self.private_api() {
            let api = self.api.clone();
            tokio::spawn(async move {
                if let Err(e) = api.mark_unread(&chat.guid).await {
                    tracing::warn!("mark unread: {e}");
                }
            });
        }
    }

    /// Typing indicators work in iMessage DMs with the Private API.
    fn can_type(&self, chat_guid: &str) -> bool {
        self.private_api()
            && chat_guid.contains(";-;")
            && !chat_guid.starts_with("SMS")
            && !chat_guid.starts_with("RCS")
    }

    fn composing(&mut self, chat_guid: String, active: bool) {
        if !active {
            self.stop_composing(&chat_guid);
            return;
        }
        if !self.can_type(&chat_guid) {
            return;
        }
        if self
            .typing_out
            .insert(chat_guid.clone(), Instant::now())
            .is_none()
        {
            self.send_typing(chat_guid, true);
        }
    }

    fn stop_composing(&mut self, chat_guid: &str) {
        if self.typing_out.remove(chat_guid).is_some() {
            self.send_typing(chat_guid.to_owned(), false);
        }
    }

    fn send_typing(&self, chat_guid: String, typing: bool) {
        let api = self.api.clone();
        tokio::spawn(async move {
            if let Err(e) = api.set_typing(&chat_guid, typing).await {
                tracing::debug!("typing: {e}");
            }
        });
    }

    fn load_older(&mut self, chat_id: i64) {
        if !self.loading_history.insert(chat_id) {
            return;
        }
        let chat = self.store.chat(chat_id).ok().flatten();
        let Some(chat) = chat.filter(|c| !c.history_done) else {
            self.loading_history.remove(&chat_id);
            return;
        };
        let before = self.store.oldest_message_date(chat_id).ok().flatten();
        let api = self.api.clone();
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let result = api.chat_messages(&chat.guid, before, HISTORY_PAGE).await;
            let _ = tx.send(Internal::History {
                chat_id,
                chat_guid: chat.guid,
                result,
            });
        });
    }

    // Sync.

    fn focus_sync(&mut self) {
        if self.status.connection != Connection::Online {
            self.socket.reconnect_now();
        }
        let recent = self
            .last_sync
            .is_some_and(|t| t.elapsed() < self.timings.focus_debounce);
        if !recent {
            self.request_sync();
        }
    }

    fn request_sync(&mut self) {
        if self.status.connection == Connection::Unauthorized {
            return;
        }
        if self.syncing {
            self.sync_dirty = true;
            return;
        }
        self.syncing = true;
        self.set_status(|s| s.syncing = true);
        let now = now_ms();
        let window = self.timings.notify_window.as_millis() as i64;
        let job = Sync {
            api: self.api.clone(),
            store: self.store.clone(),
            region: self.region,
            tx: self.internal.clone(),
            notify_since: (self.last_online - CLOCK_SLACK).max(now - window),
            macos: self.info.as_ref().and_then(ServerInfo::macos),
            contacts_every: self.timings.contacts_every,
        };
        let tx = self.internal.clone();
        let api = self.api.clone();
        let need_info = self.info.is_none();
        tokio::spawn(async move {
            // Catch-up adapts to the macOS version.
            let mut job = job;
            if need_info {
                let info = api.server_info().await;
                if let Ok(info) = &info {
                    job.macos = info.macos();
                }
                let _ = tx.send(Internal::Info(info));
            }
            let result = job.run().await;
            let _ = tx.send(Internal::SyncDone(result));
        });
    }

    fn fetch_chat(&mut self, guid: String) {
        if !self.fetching_chats.insert(guid.clone()) {
            return;
        }
        let api = self.api.clone();
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let result = api.chat(&guid).await;
            let _ = tx.send(Internal::Chat(guid, result));
        });
    }

    fn fetch_info(&self) {
        let api = self.api.clone();
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let _ = tx.send(Internal::Info(api.server_info().await));
        });
    }

    // Outbox.

    fn restore_outbox(&mut self) -> Result<(), EngineError> {
        for item in self.store.outbox()? {
            if item.state == OUTBOX_FAILED {
                continue;
            }
            match serde_json::from_str::<Outbound>(&item.payload) {
                Ok(outbound) => {
                    let job = Job {
                        temp_guid: item.temp_guid,
                        chat_guid: item.chat_guid,
                        outbound,
                        created_at: item.created_at,
                        // In flight when we stopped: it may have gone out.
                        attempts: item.attempts.max(i64::from(item.state == OUTBOX_IN_FLIGHT)),
                    };
                    self.queues
                        .entry(job.chat_guid.clone())
                        .or_default()
                        .push_back(job);
                }
                Err(e) => {
                    tracing::warn!("dropping an unreadable outbox item: {e}");
                    let _ = self
                        .store
                        .fail_outgoing(&item.temp_guid, "could not be sent");
                    let _ = self.store.outbox_remove(&item.temp_guid);
                }
            }
        }
        let chats: Vec<String> = self.queues.keys().cloned().collect();
        for chat in chats {
            self.waiting.insert(chat);
        }
        Ok(())
    }

    fn enqueue(&mut self, job: Job) {
        let chat = job.chat_guid.clone();
        self.queues.entry(chat.clone()).or_default().push_back(job);
        self.pump(&chat);
    }

    fn pump(&mut self, chat: &str) {
        if self.busy.contains(chat) || self.waiting.contains(chat) {
            return;
        }
        if self.status.connection == Connection::Unauthorized {
            self.waiting.insert(chat.to_owned());
            return;
        }
        let Some(job) = self.queues.get_mut(chat).and_then(VecDeque::pop_front) else {
            return;
        };
        let _ = self
            .store
            .outbox_set_state(&job.temp_guid, OUTBOX_IN_FLIGHT, None);
        self.busy.insert(chat.to_owned());
        let api = self.api.clone();
        let tx = self.internal.clone();
        let private_api = self.private_api();
        let give_up_at = job.created_at + self.timings.send_give_up.as_millis() as i64;
        tokio::spawn(async move {
            let outcome = outbox::send(&api, &job, private_api, give_up_at).await;
            let _ = tx.send(Internal::Sent(job, outcome));
        });
    }

    /// The server is reachable again: send what waited.
    fn resume_sends(&mut self) {
        let chats: Vec<String> = self.waiting.drain().collect();
        for chat in chats {
            self.pump(&chat);
        }
    }

    fn on_sent(&mut self, mut job: Job, outcome: Outcome) {
        let chat = job.chat_guid.clone();
        self.busy.remove(&chat);
        match outcome {
            Outcome::Sent(message) => {
                let batch = Batch {
                    messages: vec![(Some(chat.clone()), *message)],
                    ..Default::default()
                };
                if let Some(changes) = self.report("send", self.store.apply(batch)) {
                    self.after_apply(changes, i64::MAX);
                }
                let _ = self.store.outbox_remove(&job.temp_guid);
            }
            Outcome::Failed(error) => {
                tracing::warn!("send failed: {error}");
                if let Some(changes) =
                    self.report("send", self.store.fail_outgoing(&job.temp_guid, &error))
                {
                    self.after_apply(changes, i64::MAX);
                }
                let _ = self
                    .store
                    .outbox_set_state(&job.temp_guid, OUTBOX_FAILED, Some(&error));
            }
            Outcome::Offline => {
                let _ = self
                    .store
                    .outbox_set_state(&job.temp_guid, OUTBOX_QUEUED, None);
                job.attempts += 1;
                self.queues.entry(chat.clone()).or_default().push_front(job);
                self.waiting.insert(chat.clone());
                return;
            }
        }
        self.pump(&chat);
    }

    /// Fails sends that have waited too long for the server.
    fn expire_waiting(&mut self) {
        let cutoff = now_ms() - self.timings.send_give_up.as_millis() as i64;
        for chat in self.waiting.clone() {
            let Some(queue) = self.queues.get_mut(&chat) else {
                continue;
            };
            let (old, keep): (Vec<Job>, Vec<Job>) =
                queue.drain(..).partition(|j| j.created_at < cutoff);
            queue.extend(keep);
            for job in old {
                let error = "The server could not be reached";
                if let Ok(changes) = self.store.fail_outgoing(&job.temp_guid, error) {
                    self.after_apply(changes, i64::MAX);
                }
                let _ = self
                    .store
                    .outbox_set_state(&job.temp_guid, OUTBOX_FAILED, Some(error));
            }
        }
    }

    // The socket.

    fn on_socket(&mut self, event: SocketEvent) {
        match event {
            SocketEvent::Connecting { .. } => self.set_status(|s| {
                if s.connection == Connection::Offline && s.offline_since.is_none() {
                    s.connection = Connection::Connecting;
                }
            }),
            SocketEvent::Connected { .. } => {
                self.last_online = now_ms();
                self.poll_at = None;
                self.set_status(|s| {
                    s.connection = Connection::Online;
                    s.offline_since = None;
                });
                self.fetch_info();
                self.request_sync();
                self.resume_sends();
            }
            SocketEvent::Disconnected { reason, .. } => {
                tracing::info!("socket disconnected: {reason:?}");
                let now = now_ms();
                self.poll_at = Some(Instant::now() + self.timings.offline_poll);
                self.set_status(|s| {
                    if matches!(s.connection, Connection::Online | Connection::Connecting) {
                        s.connection = Connection::Offline;
                    }
                    s.offline_since.get_or_insert(now);
                });
                // Nobody will say they stopped typing.
                for chat_id in std::mem::take(&mut self.typing_in).into_keys() {
                    self.emit(EngineEvent::Typing {
                        chat_id,
                        typing: false,
                    });
                }
            }
            SocketEvent::Unauthorized { message } => {
                tracing::warn!("server refused the password: {message}");
                self.set_status(|s| {
                    s.connection = Connection::Unauthorized;
                    s.last_error = Some("The server refused the password".into());
                });
            }
            SocketEvent::Event { name, data } => self.on_server_event(&name, data),
        }
    }

    fn on_server_event(&mut self, name: &str, data: Value) {
        match name {
            "new-message"
            | "updated-message"
            | "message-send-error"
            | "group-name-change"
            | "participant-added"
            | "participant-removed"
            | "participant-left"
            | "group-icon-changed"
            | "group-icon-removed" => {
                let message: Message = match serde_json::from_value(data) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("unreadable {name}: {e}");
                        return;
                    }
                };
                // An update to an old message we never cached would leave a
                // hole in history; reconcile picks it up once paged in.
                if name == "updated-message"
                    && !self
                        .report(name, sync::fits_history(&self.store, &message))
                        .unwrap_or(false)
                {
                    return;
                }
                let group_change = name.starts_with("group-") || name.starts_with("participant-");
                let chat_guid = message.chats.first().map(|c| c.guid.clone());
                let batch = Batch {
                    messages: vec![(None, message)],
                    ..Default::default()
                };
                if let Some(changes) = self.report(name, self.store.apply(batch)) {
                    let since = now_ms() - self.timings.notify_window.as_millis() as i64;
                    self.after_apply(changes, since);
                }
                if group_change && let Some(guid) = chat_guid {
                    self.fetching_chats.remove(&guid);
                    self.fetch_chat(guid);
                }
            }
            "typing-indicator" => {
                let guid = data["guid"].as_str().unwrap_or_default();
                let display = data["display"].as_bool().unwrap_or(false);
                let Some(chat) = self.store.chat_by_guid(guid).ok().flatten() else {
                    return;
                };
                if display {
                    let expires = Instant::now() + self.timings.typing_timeout;
                    if self.typing_in.insert(chat.id, expires).is_none() {
                        self.emit(EngineEvent::Typing {
                            chat_id: chat.id,
                            typing: true,
                        });
                    }
                } else if self.typing_in.remove(&chat.id).is_some() {
                    self.emit(EngineEvent::Typing {
                        chat_id: chat.id,
                        typing: false,
                    });
                }
            }
            "chat-read-status-changed" => {
                let guid = data["chatGuid"].as_str().unwrap_or_default().to_owned();
                let read = data["read"].as_bool().unwrap_or(true);
                let batch = Batch {
                    read_states: vec![(guid.clone(), read)],
                    ..Default::default()
                };
                if let Some(changes) = self.report(name, self.store.apply(batch)) {
                    if read {
                        for &chat_id in &changes.chats {
                            self.emit(EngineEvent::ChatRead { chat_id });
                        }
                    }
                    self.after_apply(changes, i64::MAX);
                }
            }
            "new-server" => {
                let url = data
                    .as_str()
                    .or_else(|| data["server"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                if !url.is_empty() {
                    self.emit(EngineEvent::ServerMoved { url });
                }
            }
            _ => tracing::debug!("ignored event {name}"),
        }
    }

    // Results of background work.

    fn on_internal(&mut self, message: Internal) {
        match message {
            Internal::Applied {
                changes,
                notify_since,
            } => self.after_apply(changes, notify_since),
            Internal::ContactsChanged => self.emit(EngineEvent::ContactsChanged),
            Internal::SyncDone(result) => {
                self.syncing = false;
                self.last_sync = Some(Instant::now());
                match result {
                    Ok(()) => {
                        self.set_status(|s| {
                            s.syncing = false;
                            s.synced = true;
                            s.last_error = None;
                            if s.connection != Connection::Online {
                                s.connection = Connection::Polling;
                            }
                        });
                        self.resume_sends();
                    }
                    Err(EngineError::Api(ApiError::Unauthorized)) => self.set_status(|s| {
                        s.syncing = false;
                        s.connection = Connection::Unauthorized;
                        s.last_error = Some("The server refused the password".into());
                    }),
                    Err(e) => {
                        tracing::warn!("sync failed: {e}");
                        let now = now_ms();
                        self.set_status(|s| {
                            s.syncing = false;
                            s.last_error = Some(e.to_string());
                            if s.connection == Connection::Polling {
                                s.connection = Connection::Offline;
                                s.offline_since.get_or_insert(now);
                            }
                        });
                    }
                }
                if self.status.connection != Connection::Online {
                    self.poll_at = Some(Instant::now() + self.timings.offline_poll);
                }
                if std::mem::take(&mut self.sync_dirty) {
                    self.request_sync();
                }
            }
            Internal::Info(Ok(info)) => {
                let features = Features {
                    private_api: info.private_api_ready(),
                    edits: info.supports_edits(),
                    macos: info.macos(),
                    server_version: info.server_version.clone(),
                };
                self.info = Some(info);
                self.set_status(|s| s.features = features);
            }
            Internal::Info(Err(e)) => tracing::debug!("server info: {e}"),
            Internal::Sent(job, outcome) => self.on_sent(job, outcome),
            Internal::Chat(guid, result) => {
                self.fetching_chats.remove(&guid);
                match result {
                    Ok(chat) => {
                        let batch = Batch {
                            chats: vec![chat],
                            ..Default::default()
                        };
                        if let Some(changes) = self.report("chat", self.store.apply(batch)) {
                            self.after_apply(changes, i64::MAX);
                        }
                    }
                    Err(e) => tracing::warn!("fetching a chat: {e}"),
                }
            }
            Internal::History {
                chat_id,
                chat_guid,
                result,
            } => {
                self.loading_history.remove(&chat_id);
                match result {
                    Ok(messages) => {
                        let done = (messages.len() as u32) < HISTORY_PAGE;
                        let batch = sync::history_batch(&chat_guid, messages, done);
                        if let Some(changes) = self.report("history", self.store.apply(batch)) {
                            // Even an empty page changes the chat's
                            // history_done, which the view waits on.
                            let mut changes = changes;
                            changes.chats.insert(chat_id);
                            self.after_apply(changes, i64::MAX);
                        }
                    }
                    Err(e) => tracing::warn!("loading history: {e}"),
                }
            }
        }
    }

    /// Everything that follows a write: tell views, fetch unknown chats,
    /// clear typing bubbles, mark the open chat read, notify.
    fn after_apply(&mut self, changes: Changes, notify_since: i64) {
        if changes.is_empty() && changes.incomplete_chats.is_empty() {
            return;
        }
        for guid in &changes.incomplete_chats {
            self.fetch_chat(guid.clone());
        }
        let inserted: Vec<MessageRow> = if changes.inserted.is_empty() {
            Vec::new()
        } else {
            self.store
                .messages_by_id(&changes.inserted)
                .unwrap_or_default()
        };
        let incoming: Vec<&MessageRow> = inserted.iter().filter(|m| !m.is_from_me).collect();
        for m in &incoming {
            if m.associated_guid.is_none() && self.typing_in.remove(&m.chat_id).is_some() {
                self.emit(EngineEvent::Typing {
                    chat_id: m.chat_id,
                    typing: false,
                });
            }
        }
        let open = self.viewing.filter(|_| self.focused);
        let read_open = open.is_some_and(|id| incoming.iter().any(|m| m.chat_id == id));
        let notifiable = if notify_since == i64::MAX || changes.inserted.is_empty() {
            Vec::new()
        } else {
            self.store
                .take_notifiable(&changes.inserted, notify_since)
                .unwrap_or_default()
        };
        self.emit(EngineEvent::Changed(Arc::new(changes)));
        if read_open && let Some(id) = open {
            self.mark_read(id);
        }
        let notify: Vec<&MessageRow> = inserted
            .iter()
            .filter(|m| notifiable.contains(&m.id) && Some(m.chat_id) != open)
            .collect();
        if !notify.is_empty() {
            self.notify(&notify);
        }
    }

    fn notify(&self, messages: &[&MessageRow]) {
        for m in messages {
            // Group events don't notify; tapbacks only on my messages.
            if m.item_type != 0 || m.date_retracted.is_some() {
                continue;
            }
            if let Some(target) = &m.associated_guid {
                let mine = self
                    .store
                    .message_by_guid(target)
                    .ok()
                    .flatten()
                    .is_some_and(|t| t.is_from_me);
                if !mine {
                    continue;
                }
            }
            let Some(chat) = self.store.chat(m.chat_id).ok().flatten() else {
                continue;
            };
            if chat.muted {
                continue;
            }
            let addresses = chat
                .participants
                .iter()
                .map(String::as_str)
                .chain(m.sender.as_deref());
            let names = Names::load(&self.store, self.region, addresses).unwrap_or_default();
            let is_group = chat.is_group();
            let sender_name = m.sender.as_deref().map(|s| names.full(s));
            let body = names.preview(m);
            self.emit(EngineEvent::Notify(Notification {
                chat_id: chat.id,
                chat_guid: chat.guid.clone(),
                message_id: m.id,
                message_guid: m.guid.clone(),
                title: names.chat_title(&chat),
                subtitle: if is_group { sender_name } else { None },
                body,
                sender: m.sender.clone(),
                is_group,
            }));
        }
    }

    // Timers.

    fn on_tick(&mut self) {
        let now = Instant::now();

        let expired: Vec<i64> = self
            .typing_in
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        for chat_id in expired {
            self.typing_in.remove(&chat_id);
            self.emit(EngineEvent::Typing {
                chat_id,
                typing: false,
            });
        }

        let idle: Vec<String> = self
            .typing_out
            .iter()
            .filter(|(_, at)| now.duration_since(**at) >= self.timings.composing_idle)
            .map(|(chat, _)| chat.clone())
            .collect();
        for chat in idle {
            self.stop_composing(&chat);
        }

        let due: Vec<String> = self
            .read_due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(chat, _)| chat.clone())
            .collect();
        for chat in due {
            self.read_due.remove(&chat);
            if self.private_api() {
                let api = self.api.clone();
                tokio::spawn(async move {
                    if let Err(e) = api.mark_read(&chat).await {
                        tracing::warn!("mark read: {e}");
                    }
                });
            }
        }

        match self.status.connection {
            Connection::Online => {
                self.last_online = now_ms();
                if self.last_online_saved.elapsed() >= Duration::from_secs(60) {
                    self.last_online_saved = now;
                    let _ = self.store.set_i64(LAST_ONLINE_KEY, self.last_online);
                }
                let stale = self
                    .last_sync
                    .is_none_or(|t| t.elapsed() >= self.timings.catch_up_every);
                if stale && !self.syncing {
                    self.request_sync();
                }
            }
            Connection::Unauthorized => {}
            _ => {
                if self.poll_at.is_some_and(|t| t <= now) && !self.syncing {
                    self.poll_at = None;
                    self.request_sync();
                }
            }
        }

        if !self.waiting.is_empty() {
            self.expire_waiting();
        }
    }
}
