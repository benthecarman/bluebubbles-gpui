//! The window's root: the sign-in screen until a server is set up, then
//! the conversations.

use crate::config::Config;
use crate::credentials;
use crate::runtime::Runtime;
use crate::session::Session;
use crate::ui::login::{Login, LoginEvent};
use crate::ui::main_view::{MainEvent, MainView};
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
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system(window, cx);
        });
        Theme::sync_system(window, cx);
        let appearance_setting = std::env::var("BB_APPEARANCE")
            .ok()
            .or(Config::load().appearance);
        let preference = match appearance_setting.as_deref() {
            Some("light") => Preference::Light,
            Some("dark") => Preference::Dark,
            _ => Preference::System,
        };
        Theme::set_preference(preference, cx);
        let mut this = Self {
            screen: Screen::Starting,
            _subscriptions: vec![appearance],
        };
        this.start(window, cx);
        this
    }

    /// Signs in with what was saved, or shows the sign-in screen.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // For development and screenshots against the mock server.
        if let (Ok(url), Ok(password)) = (std::env::var("BB_SERVER"), std::env::var("BB_PASSWORD"))
            && let Ok(url) = parse_server_url(&url)
        {
            self.open_session(ServerConfig::new(url, password), window, cx);
            return;
        }
        let config = Config::load();
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

    fn show_login(
        &mut self,
        url: Option<String>,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                let main = cx.new(|cx| MainView::new(session, window, cx));
                let sub = cx.subscribe_in(
                    &main,
                    window,
                    move |this, _, event: &MainEvent, window, cx| match event {
                        MainEvent::SignedOut => {
                            if matches!(this.screen, Screen::Main(_)) {
                                this.show_login(
                                    Some(url.clone()),
                                    Some(
                                        "The server refused the saved password. Sign in again."
                                            .into(),
                                    ),
                                    window,
                                    cx,
                                );
                            }
                        }
                    },
                );
                self._subscriptions.push(sub);
                self.screen = Screen::Main(main);
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
                .bg(colors.window)
                .text_color(colors.text)
                .child(content),
        )
    }
}
