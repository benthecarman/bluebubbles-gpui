//! The engine between the server and the views: it keeps the store in
//! step with the server and tells views what changed.
//!
//! Views read from [`bb_store::Store`] and send intents (send, react, mark
//! read) through [`Engine`]; they never talk to the server.

pub mod address;
mod engine;
pub mod names;
mod outbox;
mod sync;
pub mod tapback;

pub use address::Region;
pub use engine::{
    Connection, Engine, EngineConfig, EngineError, EngineEvent, Features, Notification, Status,
    TextMessage, Timings,
};
pub use names::Names;
pub use tapback::{Reaction, Tapback};

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
