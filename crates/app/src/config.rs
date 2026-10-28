//! Settings on disk (`~/.config/bluebubbles-gpui/config.json`) and where
//! each server's cache lives. The password is not here: it is in the
//! keyring (see `credentials`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP_ID: &str = "bluebubbles-gpui";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_url: Option<String>,
    /// For servers behind a self-signed certificate.
    pub accept_invalid_certs: bool,
    /// `system`, `light` or `dark`.
    pub appearance: Option<String>,
    /// Closing the window keeps the app in the tray (default on).
    pub keep_running: Option<bool>,
    /// Desktop notifications for new messages (default on).
    pub notifications: Option<bool>,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_ID)
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_ID)
}

impl Config {
    pub fn keep_running(&self) -> bool {
        self.keep_running.unwrap_or(true)
    }

    pub fn notifications(&self) -> bool {
        self.notifications.unwrap_or(true)
    }

    pub fn load() -> Self {
        let path = config_dir().join("config.json");
        std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, dir.join("config.json"))
    }
}

/// The cache for one server: a file per server address, so two servers
/// never share rows.
pub fn database_path(server_url: &str) -> PathBuf {
    data_dir()
        .join("servers")
        .join(format!("{}.sqlite", server_key(server_url)))
}

/// Downloaded attachments for one server. They can always be fetched
/// again, so they live in the cache directory.
pub fn attachments_dir(server_url: &str) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_ID)
        .join(server_key(server_url))
        .join("attachments")
}

/// Pasted images, kept until their send is done with them.
pub fn outgoing_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_ID)
        .join("outgoing")
}

/// A stable name for a server address (FNV-1a: the same across runs and
/// Rust versions).
fn server_key(server_url: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in server_url
        .trim_end_matches('/')
        .to_ascii_lowercase()
        .bytes()
    {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}
