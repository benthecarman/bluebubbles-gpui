//! The app around its one window. The signed-in session lives here, not
//! in the window, so closing the window can leave the app running in the
//! tray: notifications keep arriving, and the tray, a notification or a
//! second launch brings the window back.

use crate::config::{self, Config};
use crate::session::Session;
use crate::tray::{self, TrayEvent};
use crate::ui::workspace::Workspace;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui_kit::*;

actions!(bluebubbles, [Quit, OpenSettings]);

pub struct Shell {
    window: Option<(AnyWindowHandle, WeakEntity<Workspace>)>,
    session: Option<Entity<Session>>,
    pub config: Config,
    tray: Option<tray::Handle>,
    /// The desktop shows tray icons right now.
    tray_shown: bool,
    unread: usize,
    bus: Option<zbus::Connection>,
}

impl Global for Shell {}

pub fn init(shows: Option<UnboundedReceiver<Option<String>>>, cx: &mut App) {
    cx.set_global(Shell {
        window: None,
        session: None,
        config: Config::load(),
        tray: None,
        tray_shown: false,
        unread: 0,
        bus: None,
    });
    // Closing the last window quits only when nothing keeps the app in
    // the tray; see `window_closed`.
    cx.set_quit_mode(QuitMode::Explicit);
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    // The window handles this itself while focused; this is the way in
    // when it's closed or another app has focus.
    cx.on_action(|_: &OpenSettings, cx| cx.defer(|cx| show_settings(None, cx)));
    cx.on_window_closed(window_closed).detach();
    if let Some(mut shows) = shows {
        cx.spawn(async move |cx| {
            while let Some(token) = shows.next().await {
                cx.update(|cx| show(None, token, cx));
            }
        })
        .detach();
    }
    start_tray(cx);
    let bus = cx.background_spawn(zbus::Connection::session());
    cx.spawn(async move |cx| {
        if let Ok(bus) = bus.await {
            cx.update(|cx| {
                cx.global_mut::<Shell>().bus = Some(bus);
                publish_unread(cx);
            });
        }
    })
    .detach();
}

fn start_tray(cx: &mut App) {
    let (tx, mut events) = unbounded();
    let started = cx.background_spawn(tray::start(tx));
    cx.spawn(async move |cx| {
        match started.await {
            Ok(handle) => {
                cx.update(|cx| {
                    let shell = cx.global_mut::<Shell>();
                    shell.tray = Some(handle);
                    shell.tray_shown = true;
                    publish_unread(cx);
                });
            }
            Err(e) => tracing::info!("no tray icon: {e}"),
        }
        while let Some(event) = events.next().await {
            cx.update(|cx| match event {
                TrayEvent::Open => show(None, None, cx),
                TrayEvent::MarkAllRead => mark_all_read(cx),
                TrayEvent::Settings => show_settings(None, cx),
                TrayEvent::Quit => cx.quit(),
                TrayEvent::Available(shown) => {
                    cx.global_mut::<Shell>().tray_shown = shown;
                    // Without the icon there'd be no way back to a closed
                    // window.
                    if !shown && cx.global::<Shell>().window.is_none() {
                        show(None, None, cx);
                    }
                }
            });
        }
    })
    .detach();
}

/// Whether closing the window leaves the app running: the setting is on
/// and the desktop shows the tray icon to come back with.
pub fn keeps_running(cx: &App) -> bool {
    let shell = cx.global::<Shell>();
    shell.config.keep_running() && shell.tray_shown
}

/// The desktop shows tray icons, so keeping the app running is possible.
pub fn tray_shown(cx: &App) -> bool {
    cx.global::<Shell>().tray_shown
}

fn window_closed(cx: &mut App, id: WindowId) {
    let shell = cx.global_mut::<Shell>();
    if !shell
        .window
        .as_ref()
        .is_some_and(|(w, _)| w.window_id() == id)
    {
        return;
    }
    shell.window = None;
    if !keeps_running(cx) {
        cx.quit();
        return;
    }
    // Nothing is on screen: every chat notifies.
    if let Some(session) = session(cx) {
        session.read(cx).engine.view(None, false);
    }
}

pub fn session(cx: &App) -> Option<Entity<Session>> {
    cx.global::<Shell>().session.clone()
}

/// The signed-in session, or `None` after signing out.
pub fn set_session(session: Option<Entity<Session>>, cx: &mut App) {
    let signed_out = session.is_none();
    cx.global_mut::<Shell>().session = session;
    if signed_out {
        set_unread(0, cx);
    }
}

/// Marks every chat read; the tray dot and dock count follow.
pub fn mark_all_read(cx: &mut App) {
    if let Some(session) = session(cx) {
        session.read(cx).engine.mark_all_read();
    }
}

pub fn update_config(cx: &mut App, f: impl FnOnce(&mut Config)) {
    let config = &mut cx.global_mut::<Shell>().config;
    f(config);
    if let Err(e) = config.save() {
        tracing::warn!("saving settings: {e}");
    }
}

/// Unread conversations, for the tray dot and the dock's count.
pub fn set_unread(unread: usize, cx: &mut App) {
    let shell = cx.global_mut::<Shell>();
    if shell.unread != unread {
        shell.unread = unread;
        publish_unread(cx);
    }
}

fn publish_unread(cx: &mut App) {
    let shell = cx.global::<Shell>();
    let unread = shell.unread;
    let (tray, bus) = (shell.tray.clone(), shell.bus.clone());
    cx.background_spawn(async move {
        if let Some(tray) = tray {
            tray.update(|t| t.set_unread(unread)).await;
        }
        if let Some(bus) = bus
            && let Err(e) = crate::badge::set(&bus, unread).await
        {
            tracing::debug!("dock badge: {e}");
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
    let Some((window, workspace)) = front(cx) else {
        return;
    };
    if let Some(chat) = chat {
        let _ = window.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| workspace.open_chat(chat, window, cx))
        });
    }
}

/// Shows the window with its settings open.
pub fn show_settings(activation_token: Option<String>, cx: &mut App) {
    if let Some(token) = activation_token {
        gpui_linux::set_activation_token(token);
    }
    let Some((window, workspace)) = front(cx) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.open_settings(window, cx))
    });
}

fn front(cx: &mut App) -> Option<(AnyWindowHandle, WeakEntity<Workspace>)> {
    match cx.global::<Shell>().window.clone() {
        Some(open) => {
            let _ = open.0.update(cx, |_, window, _| window.activate_window());
            Some(open)
        }
        None => open_window(cx),
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
