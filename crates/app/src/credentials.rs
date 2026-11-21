//! The server password, in the system's keyring: the Secret Service
//! (GNOME Keyring, KWallet) on Linux, Credential Manager on Windows.
//! Without one, it goes in a file only the user can read, and the login
//! screen says so.

use crate::config::data_dir;

fn fallback_path() -> std::path::PathBuf {
    data_dir().join("password")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    Keyring,
    File,
}

pub async fn load(server: &str) -> Option<String> {
    if let Some(password) = keyring::load(server).await {
        return Some(password);
    }
    let stored = std::fs::read_to_string(fallback_path()).ok()?;
    let (url, password) = stored.split_once('\n')?;
    (url == server).then(|| password.to_owned())
}

pub async fn save(server: &str, password: &str) -> Where {
    match keyring::save(server, password).await {
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

#[cfg(target_os = "linux")]
mod keyring {
    use crate::config::APP_ID;
    use std::collections::HashMap;

    fn attributes(server: &str) -> HashMap<&str, &str> {
        HashMap::from([("application", APP_ID), ("server", server)])
    }

    pub async fn load(server: &str) -> Option<String> {
        let keyring = oo7::Keyring::new().await.ok()?;
        let _ = keyring.unlock().await;
        let items = keyring.search_items(&attributes(server)).await.ok()?;
        let secret = items.first()?.secret().await.ok()?;
        String::from_utf8(secret.to_vec()).ok()
    }

    pub async fn save(server: &str, password: &str) -> Result<(), String> {
        let saved: oo7::Result<()> = async {
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
        .await;
        saved.map_err(|e| e.to_string())
    }
}

/// Generic credentials in Credential Manager, one per server, named
/// `bluebubbles-gpui:<server>` (shown under Windows Credentials).
#[cfg(windows)]
mod keyring {
    use crate::config::APP_ID;
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Security::Credentials::{
        CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree,
        CredReadW, CredWriteW,
    };
    use windows::core::{HSTRING, PWSTR};

    fn target(server: &str) -> HSTRING {
        HSTRING::from(format!("{APP_ID}:{server}"))
    }

    pub async fn load(server: &str) -> Option<String> {
        let target = target(server);
        let mut found: *mut CREDENTIALW = std::ptr::null_mut();
        unsafe {
            CredReadW(&target, CRED_TYPE_GENERIC, None, &mut found).ok()?;
            let cred = &*found;
            let bytes =
                std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize);
            let password = String::from_utf8(bytes.to_vec()).ok();
            CredFree(found as *const _);
            password
        }
    }

    pub async fn save(server: &str, password: &str) -> Result<(), String> {
        let target = target(server);
        let user = HSTRING::from("BlueBubbles");
        let mut blob = password.as_bytes().to_vec();
        let cred = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_ptr() as *mut _),
            Comment: PWSTR::null(),
            LastWritten: FILETIME::default(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            // Kept for this user on this computer, not roamed.
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: PWSTR::null(),
            UserName: PWSTR(user.as_ptr() as *mut _),
        };
        unsafe { CredWriteW(&cred, 0) }.map_err(|e| e.to_string())
    }
}

fn write_private(contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let path = fallback_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // On Windows the user's AppData is already theirs alone.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path)?.write_all(contents.as_bytes())
}
