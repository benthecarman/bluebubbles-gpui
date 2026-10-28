//! BlueBubbles for Linux: iMessage through your Mac's BlueBubbles Server.

mod config;
mod credentials;
mod notify;
mod runtime;
mod session;
mod shell;
mod ui;

use gpui_kit::*;

actions!(bluebubbles, [Quit]);

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,bb_core=info".into()),
        )
        .init();
    application().with_assets(bb_ui::Assets).run(|cx| {
        gpui_kit::init(cx);
        bb_ui::init(cx);
        runtime::Runtime::init(cx);
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        shell::init(cx);
        if shell::open_window(cx).is_none() {
            cx.quit();
        }
    });
}
