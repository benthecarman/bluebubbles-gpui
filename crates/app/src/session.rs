//! A signed-in server: the engine, a read-only store connection for the
//! views, and the engine's events re-emitted to views as GPUI events.

use crate::config;
use crate::notify::{self, Notifier, Response};
use crate::runtime::Runtime;
use crate::shell;
use bb_api::ServerConfig;
use bb_core::{Engine, EngineConfig, EngineEvent, Region, Status};
use bb_store::{Changes, Store};
use futures::StreamExt;
use gpui_kit::*;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Changed(Arc<Changes>),
    ContactsChanged,
    Typing {
        chat_id: i64,
    },
    Status,
    /// A send wasn't stored, so it wasn't sent.
    NotQueued {
        chat_guid: String,
        text: Option<String>,
    },
}

pub struct Session {
    pub engine: Engine,
    /// A second connection to the same file: views read here so they never
    /// wait on the engine's writes.
    pub store: Store,
    pub region: Region,
    pub status: Status,
    typing: HashSet<i64>,
    notifier: Rc<Notifier>,
    counting: bool,
    count_again: bool,
    _events: Task<()>,
    _responses: Task<()>,
}

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    pub fn start(server: ServerConfig, cx: &mut App) -> anyhow::Result<Entity<Self>> {
        let path = config::database_path(server.url.as_str());
        let attachments_dir = config::attachments_dir(server.url.as_str());
        let engine_store = Store::open(&path)?;
        let store = Store::open(&path)?;
        let region = Region::from_locale();
        let mut engine_config = EngineConfig::new(server, engine_store);
        engine_config.region = region;
        engine_config.attachments_dir = attachments_dir;
        let handle = Runtime::handle(cx);
        let (engine, mut events) = {
            let _runtime = handle.enter();
            Engine::spawn(engine_config)?
        };
        let status = engine.status();
        Ok(cx.new(|cx: &mut Context<Session>| {
            let pump = cx.spawn(async move |this: WeakEntity<Session>, cx| {
                while let Some(event) = events.recv().await {
                    if this
                        .update(cx, |this, cx| this.on_event(event, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let (notifier, mut responses) =
                Notifier::start(store.clone(), region, cx.background_executor());
            let responses = cx.spawn(async move |this: WeakEntity<Session>, cx| {
                while let Some(response) = responses.next().await {
                    if this
                        .update(cx, |this, cx| this.on_response(response, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let mut this = Self {
                engine,
                store,
                region,
                status,
                typing: HashSet::new(),
                notifier: Rc::new(notifier),
                counting: false,
                count_again: false,
                _events: pump,
                _responses: responses,
            };
            this.count_unread(cx);
            this
        }))
    }

    /// Re-counts unread conversations for the tray and the dock, off the
    /// UI thread; a change while counting counts once more afterwards.
    fn count_unread(&mut self, cx: &mut Context<Self>) {
        if self.counting {
            self.count_again = true;
            return;
        }
        self.counting = true;
        let store = self.store.clone();
        let count = cx.background_spawn(async move { store.unread_chat_count() });
        cx.spawn(async move |this, cx| {
            let count = count.await;
            let _ = this.update(cx, |this, cx| {
                this.counting = false;
                match count {
                    Ok(n) => shell::set_unread(n, cx),
                    Err(e) => tracing::warn!("counting unread chats: {e}"),
                }
                if std::mem::take(&mut this.count_again) {
                    this.count_unread(cx);
                }
            });
        })
        .detach();
    }

    /// Sends a sample notification; see [`Notifier::test`].
    pub fn test_notification(&self, cx: &App) -> Task<Result<String, String>> {
        let notifier = self.notifier.clone();
        cx.foreground_executor()
            .spawn(async move { notifier.test().await })
    }

    fn on_event(&mut self, event: EngineEvent, cx: &mut Context<Self>) {
        match event {
            EngineEvent::Changed(changes) => {
                if !changes.chats.is_empty() {
                    self.count_unread(cx);
                }
                cx.emit(SessionEvent::Changed(changes));
            }
            EngineEvent::ContactsChanged => cx.emit(SessionEvent::ContactsChanged),
            EngineEvent::Typing { chat_id, typing } => {
                if typing {
                    self.typing.insert(chat_id);
                } else {
                    self.typing.remove(&chat_id);
                }
                cx.emit(SessionEvent::Typing { chat_id });
            }
            EngineEvent::Notify(n) => {
                if cx.global::<shell::Shell>().config.notifications() {
                    tracing::info!("notifying for chat {}", n.chat_id);
                    self.notifier.show(n, cx.background_executor());
                }
            }
            EngineEvent::ChatRead { chat_id } => self.notifier.withdraw(chat_id),
            EngineEvent::NotQueued { chat_guid, text } => {
                cx.emit(SessionEvent::NotQueued { chat_guid, text });
            }
            EngineEvent::ServerMoved { url } => {
                tracing::info!("the server moved to {url}");
            }
            EngineEvent::Status(status) => {
                self.status = status;
                cx.emit(SessionEvent::Status);
                cx.notify();
            }
        }
    }

    fn on_response(&mut self, response: Response, cx: &mut Context<Self>) {
        match response {
            // Deferred: opening the chat reads this session, which is
            // being updated here.
            Response::Open {
                chat_id,
                activation_token,
            } => {
                let chat = Some(chat_id).filter(|&id| id != notify::NO_CHAT);
                cx.defer(move |cx| shell::show(chat, activation_token, cx));
            }
            Response::MarkRead { chat_id } => self.engine.mark_read(chat_id),
        }
    }

    pub fn is_typing(&self, chat_id: i64) -> bool {
        self.typing.contains(&chat_id)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.engine.shutdown();
    }
}
