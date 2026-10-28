//! The window's root: the sign-in screen until a server is set up, then
//! the conversations.

use crate::credentials;
use crate::runtime::Runtime;
use crate::session::Session;
use crate::shell::{self, Shell};
use crate::ui::login::{Login, LoginEvent};
use crate::ui::main_view::{MainEvent, MainView};
use crate::ui::settings::{Settings, SettingsEvent};
use bb_api::{ServerConfig, parse_server_url};
use bb_ui::{ActiveTheme, Preference, Theme, TypeStyled};
use gpui_kit::component::window_border;
use gpui_kit::*;

enum Screen {
    Starting,
    Login(Entity<Login>),
    Main(Entity<MainView>),
}

pub struct Workspace {
    screen: Screen,
    settings: Option<Entity<Settings>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system(window, cx);
        });
        Theme::sync_system(window, cx);
        // GNOME's button-layout setting moves the window buttons.
        let buttons = window.observe_button_layout_changed(|window, _| window.refresh());
        let appearance_setting =
            std::env::var("BB_APPEARANCE")
                .ok()
                .or(cx.global::<Shell>().config.appearance.clone());
        let preference = match appearance_setting.as_deref() {
            Some("light") => Preference::Light,
            Some("dark") => Preference::Dark,
            _ => Preference::System,
        };
        Theme::set_preference(preference, cx);
        let mut this = Self {
            screen: Screen::Starting,
            settings: None,
            _subscriptions: vec![appearance, buttons],
        };
        this.start(window, cx);
        this
    }

    /// Signs in with what was saved, or shows the sign-in screen.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The window was closed to the tray and is back: same session.
        if let Some(session) = shell::session(cx) {
            self.show_main(session, window, cx);
            return;
        }
        // For development and screenshots against the mock server.
        if let (Ok(url), Ok(password)) = (std::env::var("BB_SERVER"), std::env::var("BB_PASSWORD"))
            && let Ok(url) = parse_server_url(&url)
        {
            self.open_session(ServerConfig::new(url, password), window, cx);
            return;
        }
        let config = cx.global::<Shell>().config.clone();
        let Some(url) = config.server_url.clone() else {
            self.show_login(None, None, window, cx);
            return;
        };
        let lookup = Runtime::handle(cx).spawn({
            let url = url.clone();
            async move { credentials::load(&url).await }
        });
        cx.spawn_in(window, async move |this, cx| {
            let password = lookup.await.ok().flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                match (password, parse_server_url(&url)) {
                    (Some(password), Ok(parsed)) => {
                        let mut server = ServerConfig::new(parsed, password);
                        server.accept_invalid_certs = config.accept_invalid_certs;
                        this.open_session(server, window, cx);
                    }
                    _ => this.show_login(Some(url), None, window, cx),
                }
            });
        })
        .detach();
    }

    /// Opens `chat_id` once signed in, as for a clicked notification.
    pub fn open_chat(&mut self, chat_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Screen::Main(main) = &self.screen {
            main.update(cx, |main, cx| main.open(chat_id, window, cx));
        }
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let settings = cx.new(|cx| Settings::new(window, cx));
        let sub = cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &SettingsEvent, _, cx| match event {
                SettingsEvent::Close => {
                    this.settings = None;
                    cx.notify();
                }
            },
        );
        self._subscriptions.push(sub);
        settings.focus_handle(cx).focus(window, cx);
        self.settings = Some(settings);
        cx.notify();
    }

    fn show_login(
        &mut self,
        url: Option<String>,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        shell::set_session(None, cx);
        let login = cx.new(|cx| Login::new(url, error, window, cx));
        let sub =
            cx.subscribe_in(
                &login,
                window,
                |this, _, event: &LoginEvent, window, cx| match event {
                    LoginEvent::SignedIn(server) => this.open_session(server.clone(), window, cx),
                },
            );
        self._subscriptions.push(sub);
        self.screen = Screen::Login(login);
        cx.notify();
    }

    fn open_session(&mut self, server: ServerConfig, window: &mut Window, cx: &mut Context<Self>) {
        let url = server.url.to_string();
        match Session::start(server, cx) {
            Ok(session) => {
                shell::set_session(Some(session.clone()), cx);
                self.show_main(session, window, cx);
            }
            Err(e) => {
                tracing::error!("starting the session: {e:#}");
                self.show_login(
                    Some(url),
                    Some(format!("Couldn't open the local cache: {e}")),
                    window,
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn show_main(&mut self, session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) {
        let main = cx.new(|cx| MainView::new(session, window, cx));
        let sub = cx.subscribe_in(
            &main,
            window,
            move |this, _, event: &MainEvent, window, cx| match event {
                MainEvent::SignedOut => {
                    if matches!(this.screen, Screen::Main(_)) {
                        let url = cx.global::<Shell>().config.server_url.clone();
                        this.show_login(
                            url,
                            Some("The server refused the saved password. Sign in again.".into()),
                            window,
                            cx,
                        );
                    }
                }
            },
        );
        self._subscriptions.push(sub);
        self.screen = Screen::Main(main);
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let content = match &self.screen {
            Screen::Starting => div().size_full().bg(colors.window).into_any_element(),
            Screen::Login(login) => login.clone().into_any_element(),
            Screen::Main(main) => main.clone().into_any_element(),
        };
        window_border().shadow_size(px(0.)).child(
            div()
                .messages_type()
                .size_full()
                .relative()
                .on_action(cx.listener(|this, _: &shell::OpenSettings, window, cx| {
                    this.open_settings(window, cx)
                }))
                .bg(colors.window)
                .text_color(colors.text)
                .child(content)
                .children(self.settings.clone()),
        )
    }
}
