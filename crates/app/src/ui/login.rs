//! Sign in: the server's address and password, checked against the
//! server before anything is saved.

use crate::config::Config;
use crate::credentials;
use crate::runtime::Runtime;
use crate::ui::window_controls;
use bb_api::{ApiError, Client, ServerConfig, parse_server_url};
use bb_ui::ActiveTheme;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

pub enum LoginEvent {
    SignedIn(ServerConfig),
}

pub struct Login {
    url: Entity<InputState>,
    password: Entity<InputState>,
    error: Option<SharedString>,
    note: Option<SharedString>,
    busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LoginEvent> for Login {}

impl Login {
    pub fn new(
        url: Option<String>,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://example.ngrok.app"));
        if let Some(url) = url {
            url_input.update(cx, |s, cx| s.set_value(url, window, cx));
        }
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let subscriptions = vec![
            cx.subscribe_in(&url_input, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    this.password.update(cx, |s, cx| s.focus(window, cx));
                }
            }),
            cx.subscribe_in(&password, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    this.connect(window, cx);
                }
            }),
        ];
        url_input.update(cx, |s, cx| s.focus(window, cx));
        Self {
            url: url_input,
            password,
            error: error.map(Into::into),
            note: None,
            busy: false,
            _subscriptions: subscriptions,
        }
    }

    fn connect(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let address = self.url.read(cx).value().trim().to_owned();
        let password = self.password.read(cx).value().to_string();
        let url = match parse_server_url(&address) {
            Ok(url) => url,
            Err(_) => {
                self.error =
                    Some("Enter the server address shown in the BlueBubbles Server app.".into());
                cx.notify();
                return;
            }
        };
        if password.is_empty() {
            self.error = Some("Enter the server password.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let server = ServerConfig::new(url, password);
        let check = {
            let server = server.clone();
            Runtime::handle(cx).spawn(async move {
                let client = Client::new(&server)?;
                client.server_info().await?;
                let stored = credentials::save(server.url.as_str(), &server.password).await;
                Ok::<_, ApiError>(stored)
            })
        };
        cx.spawn(async move |this, cx| {
            let result = check.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(stored)) => {
                        let mut config = Config::load();
                        config.server_url = Some(server.url.to_string());
                        if let Err(e) = config.save() {
                            tracing::error!("saving settings: {e}");
                        }
                        if stored == credentials::Where::File {
                            this.note = Some("No keyring is available, so the password is kept in a file only you can read.".into());
                        }
                        cx.emit(LoginEvent::SignedIn(server));
                    }
                    Ok(Err(e)) => this.error = Some(explain(&e).into()),
                    Err(_) => this.error = Some("Something went wrong. Try again.".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

pub fn explain(e: &ApiError) -> String {
    match e {
        ApiError::Unauthorized => "The server refused the password.".into(),
        ApiError::BadUrl(_) => "That doesn't look like a server address.".into(),
        ApiError::Timeout => "The server took too long to answer. Is it running?".into(),
        ApiError::Network(_) => {
            "Couldn't reach the server. Check the address, and that the server is running.".into()
        }
        ApiError::NotFound => "That address isn't a BlueBubbles server.".into(),
        other => other.to_string(),
    }
}

impl Render for Login {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let field = |input: &Entity<InputState>| {
            div()
                .w_full()
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(7.))
                .border_1()
                .border_color(colors.field_border)
                .bg(colors.field)
                .child(Input::new(input).appearance(false).text_size(m.body()))
        };
        let label = |text: &'static str| {
            div()
                .text_size(m.small())
                .text_color(colors.text_secondary)
                .mb(px(4.))
                .child(text)
        };
        let busy = self.busy;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.window)
            .child(
                window_controls::title_bar(
                    div()
                        .id("login-title")
                        .h(px(44.))
                        .w_full()
                        .flex()
                        .items_center()
                        .px(px(16.)),
                )
                .child(window_controls::traffic_lights(window)),
            )
            .child(
                div().flex_1().flex().items_center().justify_center().child(
                    div()
                        .w(px(340.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(img("bb/app-icon.png").size(px(72.)).mb(px(14.)))
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .mb(px(4.))
                                .child("Welcome to BlueBubbles"),
                        )
                        .child(
                            div()
                                .text_size(m.body())
                                .text_color(colors.text_secondary)
                                .text_center()
                                .mb(px(22.))
                                .child("Sign in to the BlueBubbles Server running on your Mac."),
                        )
                        .child(
                            div()
                                .w_full()
                                .child(label("Server address"))
                                .child(field(&self.url)),
                        )
                        .child(div().h(px(12.)))
                        .child(
                            div()
                                .w_full()
                                .child(label("Password"))
                                .child(field(&self.password)),
                        )
                        .when_some(self.error.clone(), |el, error| {
                            el.child(
                                div()
                                    .w_full()
                                    .mt(px(12.))
                                    .text_size(m.small())
                                    .text_color(colors.error)
                                    .child(error),
                            )
                        })
                        .when_some(self.note.clone(), |el, note| {
                            el.child(
                                div()
                                    .w_full()
                                    .mt(px(12.))
                                    .text_size(m.small())
                                    .text_color(colors.text_secondary)
                                    .child(note),
                            )
                        })
                        .child(
                            div()
                                .id("connect")
                                .mt(px(20.))
                                .w_full()
                                .h(px(34.))
                                .rounded(px(7.))
                                .bg(colors.accent)
                                .when(busy, |b| b.opacity(0.6))
                                .text_color(colors.on_bubble)
                                .text_size(m.body())
                                .font_weight(FontWeight::MEDIUM)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .child(if busy { "Connecting…" } else { "Connect" })
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.connect(window, cx)),
                                ),
                        ),
                ),
            )
    }
}
