//! BlueBubbles for Linux: iMessage through your Mac's BlueBubbles Server.

mod config;
mod credentials;
mod runtime;
mod session;
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
        let bounds = Bounds::centered(None, size(px(1080.), px(720.)), cx);
        let opened = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(420.))),
                app_id: Some(config::APP_ID.into()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Messages".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| ui::workspace::Workspace::new(window, cx)),
        );
        if let Err(e) = opened {
            tracing::error!("could not open the window: {e:#}");
            cx.quit();
        }
    });
}
