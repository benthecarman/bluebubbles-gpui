//! The app around its one window: opens it, and brings it forward on a
//! chat when a notification is clicked.

use crate::config;
use crate::ui::workspace::Workspace;
use gpui_kit::*;

#[derive(Default)]
pub struct Shell {
    window: Option<(AnyWindowHandle, WeakEntity<Workspace>)>,
}

impl Global for Shell {}

pub fn init(cx: &mut App) {
    cx.set_global(Shell::default());
    cx.on_window_closed(|cx, id| {
        let shell = cx.global_mut::<Shell>();
        if shell
            .window
            .as_ref()
            .is_some_and(|(w, _)| w.window_id() == id)
        {
            shell.window = None;
        }
    })
    .detach();
}

/// Opens the window, or brings it forward, then opens `chat` in it.
/// `activation_token` lets a Wayland compositor raise the window while
/// another app has focus.
pub fn show(chat: Option<i64>, activation_token: Option<String>, cx: &mut App) {
    if let Some(token) = activation_token {
        gpui_linux::set_activation_token(token);
    }
    let (window, workspace) = match cx.global::<Shell>().window.clone() {
        Some(open) => {
            let _ = open.0.update(cx, |_, window, _| window.activate_window());
            open
        }
        None => match open_window(cx) {
            Some(open) => open,
            None => return,
        },
    };
    if let Some(chat) = chat {
        let _ = window.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| workspace.open_chat(chat, window, cx))
        });
    }
}

pub fn open_window(cx: &mut App) -> Option<(AnyWindowHandle, WeakEntity<Workspace>)> {
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
        |window, cx| cx.new(|cx| Workspace::new(window, cx)),
    );
    match opened {
        Ok((window, workspace)) => {
            let open = (window, workspace.downgrade());
            cx.global_mut::<Shell>().window = Some(open.clone());
            Some(open)
        }
        Err(e) => {
            tracing::error!("could not open the window: {e:#}");
            None
        }
    }
}
