//! The server password, in the Secret Service keyring (GNOME Keyring,
//! KWallet). Without one, it goes in a file only the user can read, and
//! the login screen says so.

use crate::config::{APP_ID, data_dir};
use std::collections::HashMap;

fn attributes(server: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", APP_ID), ("server", server)])
}

fn fallback_path() -> std::path::PathBuf {
    data_dir().join("password")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    Keyring,
    File,
}

pub async fn load(server: &str) -> Option<String> {
    if let Ok(keyring) = oo7::Keyring::new().await {
        let _ = keyring.unlock().await;
        if let Ok(items) = keyring.search_items(&attributes(server)).await
            && let Some(item) = items.first()
            && let Ok(secret) = item.secret().await
        {
            return String::from_utf8(secret.to_vec()).ok();
        }
    }
    let stored = std::fs::read_to_string(fallback_path()).ok()?;
    let (url, password) = stored.split_once('\n')?;
    (url == server).then(|| password.to_owned())
}

pub async fn save(server: &str, password: &str) -> Where {
    match save_keyring(server, password).await {
        Ok(()) => {
            let _ = std::fs::remove_file(fallback_path());
            Where::Keyring
        }
        Err(e) => {
            tracing::warn!("no keyring ({e}); keeping the password in a private file");
            match write_private(&format!("{server}\n{password}")) {
                Ok(()) => Where::File,
                Err(e) => {
                    tracing::error!("could not store the password: {e}");
                    Where::File
                }
            }
        }
    }
}

#[allow(clippy::result_large_err)] // oo7's error type
async fn save_keyring(server: &str, password: &str) -> oo7::Result<()> {
    let keyring = oo7::Keyring::new().await?;
    keyring.unlock().await?;
    keyring
        .create_item(
            &format!("BlueBubbles server {server}"),
            &attributes(server),
            password,
            true,
        )
        .await
}

fn write_private(contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = fallback_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(contents.as_bytes())
}
