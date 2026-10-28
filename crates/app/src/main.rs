//! BlueBubbles for Linux: iMessage through your Mac's BlueBubbles Server.

mod badge;
mod config;
mod credentials;
mod instance;
mod notify;
mod runtime;
mod session;
mod shell;
mod tray;
mod ui;

use gpui_kit::*;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,bb_core=info".into()),
        )
        .init();
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
        if shell::open_window(cx).is_none() {
            cx.quit();
        }
    });
}
