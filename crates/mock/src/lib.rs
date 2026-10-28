//! A fake BlueBubbles Server: the REST routes and Socket.IO events this
//! client uses, with the real server's quirks (duplicate echoes of sends,
//! error-shaped send failures, no replay after a reconnect).
//!
//! Tests drive it through [`MockServer`]'s methods: incoming messages,
//! typing, receipts, dropped sockets and failing sends.

pub mod demo;
mod http;
mod socket;

pub use demo::seed_demo;

use bb_api::{Attachment, Chat, Contact, Handle, Message, ServerInfo};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Apple's epoch (2001-01-01) in Unix milliseconds; message date columns
/// in `where` clauses are nanoseconds since it.
pub const APPLE_EPOCH_MS: i64 = 978_307_200_000;

/// How the next sends behave.
#[derive(Debug, Clone, Default)]
pub struct SendBehaviour {
    /// Messages rejects the next send with this error code (HTTP 500 with
    /// the message, and a `message-send-error` event).
    pub fail_with: Option<i64>,
    /// The send goes through, but the HTTP reply is an error (a timeout
    /// between the client and server).
    pub lose_reply: bool,
    /// Emit the full `new-message` before the tempGuid one.
    pub full_echo_first: bool,
    /// Wait this long before answering.
    pub delay: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct Stored {
    pub message: Message,
    /// When the message last changed, Unix ms; the reconcile query
    /// matches on it.
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) enum Outgoing {
    Event(String, Value),
    Drop,
}

pub(crate) struct State {
    pub password: String,
    pub info: ServerInfo,
    pub chats: Vec<Chat>,
    pub messages: Vec<Stored>,
    pub next_rowid: i64,
    pub attachment_data: HashMap<String, (String, Vec<u8>)>,
    pub contacts: Vec<Contact>,
    pub send: SendBehaviour,
    pub requests: Vec<String>,
    pub ping_interval_ms: u64,
    pub ping_timeout_ms: u64,
    pub sockets: usize,
}

#[derive(Clone)]
pub(crate) struct Shared {
    pub state: Arc<Mutex<State>>,
    pub events: broadcast::Sender<Outgoing>,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn emit(&self, name: &str, data: Value) {
        let _ = self.events.send(Outgoing::Event(name.to_owned(), data));
    }
}

pub struct MockServer {
    pub addr: SocketAddr,
    shared: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub const ME: &str = "me@icloud.com";

impl MockServer {
    /// Starts on a free localhost port.
    pub async fn start(password: &str) -> Self {
        Self::start_on("127.0.0.1:0".parse().expect("valid address"), password).await
    }

    pub async fn start_on(addr: SocketAddr, password: &str) -> Self {
        let (events, _) = broadcast::channel(1024);
        let shared = Shared {
            state: Arc::new(Mutex::new(State {
                password: password.to_owned(),
                info: ServerInfo {
                    computer_id: Some("mock-mac".into()),
                    os_version: Some("15.1".into()),
                    server_version: Some("1.9.12".into()),
                    private_api: Some(true),
                    helper_connected: Some(true),
                    detected_imessage: Some(ME.into()),
                    ..Default::default()
                },
                chats: Vec::new(),
                messages: Vec::new(),
                next_rowid: 1,
                attachment_data: HashMap::new(),
                contacts: Vec::new(),
                send: SendBehaviour::default(),
                requests: Vec::new(),
                ping_interval_ms: 25_000,
                ping_timeout_ms: 20_000,
                sockets: 0,
            })),
            events,
        };
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bind the mock server");
        let addr = listener.local_addr().expect("local address");
        let app = http::router(shared.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { addr, shared, task }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn set_password(&self, password: &str) {
        self.shared.lock().password = password.to_owned();
        let _ = self.shared.events.send(Outgoing::Drop);
    }

    pub fn set_info(&self, f: impl FnOnce(&mut ServerInfo)) {
        f(&mut self.shared.lock().info);
    }

    pub fn set_ping(&self, interval: Duration, timeout: Duration) {
        let mut s = self.shared.lock();
        s.ping_interval_ms = interval.as_millis() as u64;
        s.ping_timeout_ms = timeout.as_millis() as u64;
    }

    pub fn set_send_behaviour(&self, behaviour: SendBehaviour) {
        self.shared.lock().send = behaviour;
    }

    /// `"METHOD /path"` of every authenticated request so far.
    pub fn requests(&self) -> Vec<String> {
        self.shared.lock().requests.clone()
    }

    pub fn connected_sockets(&self) -> usize {
        self.shared.lock().sockets
    }

    /// Closes every socket, as a network drop would.
    pub fn drop_sockets(&self) {
        let _ = self.shared.events.send(Outgoing::Drop);
    }

    pub fn add_chat(&self, guid: &str, display_name: Option<&str>, participants: &[&str]) {
        let service = guid.split(';').next().unwrap_or("iMessage");
        let mut s = self.shared.lock();
        let handles = participants
            .iter()
            .map(|address| s.handle(address, service))
            .collect::<Vec<_>>();
        let rowid = s.chats.len() as i64 + 1;
        s.chats.push(Chat {
            original_rowid: Some(rowid),
            guid: guid.to_owned(),
            style: Some(if handles.len() > 1 || guid.contains(";+;") {
                43
            } else {
                45
            }),
            chat_identifier: Some(guid.rsplit(';').next().unwrap_or(guid).to_owned()),
            is_archived: Some(false),
            display_name: Some(display_name.unwrap_or_default().to_owned()),
            participants: Some(handles),
            ..Default::default()
        });
    }

    pub fn add_contact(&self, name: &str, phones: &[&str], emails: &[&str]) {
        let mut parts = name.splitn(2, ' ');
        let contact = Contact {
            id: Some(format!("c{}", self.shared.lock().contacts.len() + 1)),
            display_name: Some(name.to_owned()),
            first_name: parts.next().map(str::to_owned),
            last_name: parts.next().map(str::to_owned),
            phone_numbers: phones
                .iter()
                .map(|p| bb_api::ContactAddress {
                    address: (*p).to_owned(),
                })
                .collect(),
            emails: emails
                .iter()
                .map(|e| bb_api::ContactAddress {
                    address: (*e).to_owned(),
                })
                .collect(),
            ..Default::default()
        };
        self.shared.lock().contacts.push(contact);
    }

    pub fn add_attachment_data(&self, guid: &str, mime: &str, bytes: Vec<u8>) {
        self.shared
            .lock()
            .attachment_data
            .insert(guid.to_owned(), (mime.to_owned(), bytes));
    }

    /// A message from `from` arrives and is pushed to sockets.
    pub fn receive(&self, chat: &str, from: &str, text: &str) -> Message {
        self.receive_with(chat, from, text, |_| {})
    }

    pub fn receive_with(
        &self,
        chat: &str,
        from: &str,
        text: &str,
        edit: impl FnOnce(&mut Message),
    ) -> Message {
        let m = self.insert(chat, Some(from), text, now_ms(), edit);
        self.shared.emit("new-message", full(&m));
        m
    }

    /// A message arrives while nobody is listening (no event), as when the
    /// client is offline.
    pub fn insert_silently(
        &self,
        chat: &str,
        from: Option<&str>,
        text: &str,
        date: i64,
    ) -> Message {
        self.insert(chat, from, text, date, |_| {})
    }

    fn insert(
        &self,
        chat: &str,
        from: Option<&str>,
        text: &str,
        date: i64,
        edit: impl FnOnce(&mut Message),
    ) -> Message {
        let mut s = self.shared.lock();
        let mut m = s.new_message(chat, from, text, date);
        edit(&mut m);
        s.messages.push(Stored {
            message: m.clone(),
            updated_at: date,
        });
        m
    }

    /// Changes a stored message, then emits `updated-message` unless
    /// `silent`.
    pub fn update(
        &self,
        guid: &str,
        silent: bool,
        edit: impl FnOnce(&mut Message),
    ) -> Option<Message> {
        let m = {
            let mut s = self.shared.lock();
            let stored = s.messages.iter_mut().find(|m| m.message.guid == guid)?;
            edit(&mut stored.message);
            stored.updated_at = now_ms();
            stored.message.clone()
        };
        if !silent {
            self.shared.emit("updated-message", full(&m));
        }
        Some(m)
    }

    pub fn typing(&self, chat: &str, display: bool) {
        self.shared.emit(
            "typing-indicator",
            json!({ "guid": chat, "display": display }),
        );
    }

    pub fn read_status(&self, chat: &str, read: bool) {
        self.shared.emit(
            "chat-read-status-changed",
            json!({ "chatGuid": chat, "read": read }),
        );
    }

    pub fn messages(&self) -> Vec<Message> {
        self.shared
            .lock()
            .messages
            .iter()
            .map(|m| m.message.clone())
            .collect()
    }

    pub fn emit(&self, name: &str, data: Value) {
        self.shared.emit(name, data);
    }
}

impl State {
    pub fn handle(&mut self, address: &str, service: &str) -> Handle {
        Handle {
            original_rowid: Some(address.bytes().map(i64::from).sum::<i64>() % 10_000 + 1),
            address: address.to_owned(),
            service: Some(
                if service == "any" {
                    "iMessage"
                } else {
                    service
                }
                .to_owned(),
            ),
            country: Some("us".into()),
            ..Default::default()
        }
    }

    pub fn chat(&self, guid: &str) -> Option<&Chat> {
        self.chats.iter().find(|c| c.guid == guid)
    }

    pub fn new_message(
        &mut self,
        chat: &str,
        from: Option<&str>,
        text: &str,
        date: i64,
    ) -> Message {
        let rowid = self.next_rowid;
        self.next_rowid += 1;
        let service = chat.split(';').next().unwrap_or("iMessage").to_owned();
        let handle = from.map(|a| self.handle(a, &service));
        Message {
            original_rowid: Some(rowid),
            guid: format!("MOCK-{rowid:06}"),
            text: Some(text.to_owned()),
            handle_id: handle.as_ref().and_then(|h| h.original_rowid),
            handle,
            is_from_me: from.is_none(),
            date_created: Some(date),
            is_delivered: Some(true),
            error: Some(0),
            item_type: Some(0),
            group_action_type: Some(0),
            part_count: Some(1),
            chats: self
                .chat(chat)
                .map(|c| vec![slim_chat(c)])
                .unwrap_or_else(|| {
                    vec![Chat {
                        guid: chat.to_owned(),
                        ..Default::default()
                    }]
                }),
            ..Default::default()
        }
    }

    pub fn last_message(&self, chat: &str) -> Option<Message> {
        self.messages
            .iter()
            .filter(|m| m.message.chats.first().is_some_and(|c| c.guid == chat))
            .max_by_key(|m| (m.message.date_created, m.message.original_rowid))
            .map(|m| {
                let mut last = m.message.clone();
                last.chats = Vec::new();
                last
            })
    }

    pub fn attachment(&mut self, transfer_name: &str, mime: &str, bytes: Vec<u8>) -> Attachment {
        let guid = format!("at_{}_{transfer_name}", self.next_rowid);
        self.attachment_data
            .insert(guid.clone(), (mime.to_owned(), bytes.clone()));
        Attachment {
            guid,
            mime_type: Some(mime.to_owned()),
            transfer_name: Some(transfer_name.to_owned()),
            total_bytes: Some(bytes.len() as i64),
            transfer_state: Some(5),
            ..Default::default()
        }
    }
}

/// JavaScript's `decodeURI`, near enough: `%XX` escapes become bytes, and
/// a malformed escape is an error.
pub(crate) fn decode_uri(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub(crate) fn slim_chat(c: &Chat) -> Chat {
    Chat {
        participants: None,
        last_message: None,
        ..c.clone()
    }
}

/// The full socket payload: chats without participants.
pub(crate) fn full(m: &Message) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.remove("tempGuid");
    }
    v
}

/// The slim payload the server sends with `tempGuid` for a matched send.
pub(crate) fn slim(m: &Message, temp_guid: &str) -> Value {
    let mut v = serde_json::to_value(Message {
        attributed_body: None,
        message_summary_info: None,
        payload_data: None,
        ..m.clone()
    })
    .unwrap_or_default();
    v["tempGuid"] = json!(temp_guid);
    v
}
