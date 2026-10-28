//! REST client and JSON types for the BlueBubbles Server's `/api/v1`.

mod client;
pub mod models;

pub use client::{
    ApiError, Client, Download, MESSAGE_WITH, MessageQuery, Page, SendAttachment, SendText,
    ServerConfig, WhereClause, parse_server_url, socket_password,
};
pub use models::*;
