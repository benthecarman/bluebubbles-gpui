//! A signed-in server: the engine, a read-only store connection for the
//! views, and the engine's events re-emitted to views as GPUI events.

use crate::config;
use crate::runtime::Runtime;
use bb_api::ServerConfig;
use bb_core::{Engine, EngineConfig, EngineEvent, Region, Status};
use bb_store::{Changes, Store};
use gpui_kit::*;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Changed(Arc<Changes>),
    ContactsChanged,
    Typing { chat_id: i64 },
    Status,
}

pub struct Session {
    pub engine: Engine,
    /// A second connection to the same file: views read here so they never
    /// wait on the engine's writes.
    pub store: Store,
    pub region: Region,
    pub status: Status,
    typing: HashSet<i64>,
    _events: Task<()>,
}

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    pub fn start(server: ServerConfig, cx: &mut App) -> anyhow::Result<Entity<Self>> {
        let path = config::database_path(server.url.as_str());
        let engine_store = Store::open(&path)?;
        let store = Store::open(&path)?;
        let region = Region::from_locale();
        let mut engine_config = EngineConfig::new(server, engine_store);
        engine_config.region = region;
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
            Self {
                engine,
                store,
                region,
                status,
                typing: HashSet::new(),
                _events: pump,
            }
        }))
    }

    fn on_event(&mut self, event: EngineEvent, cx: &mut Context<Self>) {
        match event {
            EngineEvent::Changed(changes) => cx.emit(SessionEvent::Changed(changes)),
            EngineEvent::ContactsChanged => cx.emit(SessionEvent::ContactsChanged),
            EngineEvent::Typing { chat_id, typing } => {
                if typing {
                    self.typing.insert(chat_id);
                } else {
                    self.typing.remove(&chat_id);
                }
                cx.emit(SessionEvent::Typing { chat_id });
            }
            // Desktop notifications come with Phase 4.
            EngineEvent::Notify(_) | EngineEvent::ChatRead { .. } => {}
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

    pub fn is_typing(&self, chat_id: i64) -> bool {
        self.typing.contains(&chat_id)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.engine.shutdown();
    }
}
