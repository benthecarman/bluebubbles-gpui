//! A Socket.IO 4 client for the BlueBubbles Server, over WebSocket only.
//!
//! The client owns one background task that keeps a connection up: it
//! reconnects with jittered exponential backoff, notices a dead connection
//! from missed server pings, and reconnects at once after the machine
//! wakes from suspend (when a TCP connection is often silently gone). The
//! official client misses messages after sleep or network changes; here
//! every (re)connect is reported, so the sync layer can catch up from its
//! cursor.

mod client;
pub mod packet;

pub use client::{
    Backoff, ConnectionState, DisconnectReason, Socket, SocketConfig, SocketError, SocketEvent,
};
