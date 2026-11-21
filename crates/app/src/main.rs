//! BlueBubbles for Linux and Windows: iMessage through your Mac's
//! BlueBubbles Server.

// A windowed app on Windows: no console window behind it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod badge;
mod config;
mod credentials;
mod emoji;
mod instance;
#[cfg_attr(windows, path = "notify_windows.rs")]
mod notify;
mod process;
mod runtime;
mod session;
mod shell;
mod sound;
mod spell;
#[cfg_attr(windows, path = "tray_windows.rs")]
mod tray;
mod tray_art;
mod ui;

use gpui_kit::*;

fn main() {
    let log = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,bb_core=info".into()),
    );
    // A windowed app has no terminal to log to, so the log is a file:
    // %LOCALAPPDATA%\bluebubbles-gpui\bluebubbles.log.
    #[cfg(windows)]
    match log_file() {
        Some(file) => log
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init(),
        None => log.init(),
    }
    #[cfg(not(windows))]
    log.init();
    #[cfg(windows)]
    notify::set_app_id();
    let shows = match instance::claim() {
        instance::Claim::Primary(shows) => shows,
        instance::Claim::Handed => {
            tracing::info!("BlueBubbles is already running; showing its window");
            return;
        }
    };
    application().with_assets(bb_ui::Assets).run(|cx| {
        gpui_kit::init(cx);
        bb_ui::init(cx);
        runtime::Runtime::init(cx);
        shell::init(Some(shows), cx);
        ui::settings::init(cx);
        ui::conversation::init(cx);
        spell::init(cx);
        if shell::open_window(cx).is_none() {
            cx.quit();
        }
    });
}

/// The log, added to by each run (a second launch logs a line before
/// handing over to the first) and started over past 10 MB.
#[cfg(windows)]
fn log_file() -> Option<std::fs::File> {
    let dir = dirs::cache_dir()?.join(config::APP_ID);
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("bluebubbles.log");
    let full = std::fs::metadata(&path).is_ok_and(|m| m.len() > 10 << 20);
    std::fs::OpenOptions::new()
        .create(true)
        .append(!full)
        .write(true)
        .truncate(full)
        .open(path)
        .ok()
}
