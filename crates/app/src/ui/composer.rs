//! The message field: a rounded field that grows with its text, Enter to
//! send, Shift+Enter for a new line, and the round send arrow once there
//! is something to send. Drafts are kept per chat.

use crate::session::{Session, SessionEvent};
use crate::ui::people;
use bb_core::TextMessage;
use bb_ui::{ActiveTheme, Icon, icon};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::time::Duration;

pub struct Composer {
    session: Entity<Session>,
    chat_id: i64,
    chat_guid: String,
    sms: bool,
    input: Entity<TextareaState>,
    empty: bool,
    save_draft: Option<Task<()>>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl Composer {
    pub fn new(
        session: Entity<Session>,
        chat: &bb_store::ChatRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let service = people::chat_service(chat);
        let sms = people::is_sms(&service);
        let placeholder = if sms {
            "Text Message • SMS"
        } else {
            "iMessage"
        };
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(placeholder)
                .submit_on_enter(true)
                .auto_grow(1, 8)
        });
        let draft = chat.draft.clone().unwrap_or_default();
        if !draft.is_empty() {
            input.update(cx, |s, cx| s.set_value(draft.clone(), window, cx));
        }
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                        InputEvent::Change => this.changed(cx),
                        _ => {}
                    }
                }),
                cx.subscribe_in(
                    &session,
                    window,
                    |this, _, event: &SessionEvent, window, cx| {
                        if let SessionEvent::NotQueued { chat_guid, text } = event
                            && *chat_guid == this.chat_guid
                        {
                            this.not_queued(text.clone(), window, cx);
                        }
                    },
                ),
            ];
        Self {
            session,
            chat_id: chat.id,
            chat_guid: chat.guid.clone(),
            sms,
            input,
            empty: draft.trim().is_empty(),
            save_draft: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let input = this.read(cx).input.clone();
        input.update(cx, |s, cx| s.focus(window, cx));
    }

    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// The message never left: give its text back, unless something new
    /// has been typed since.
    fn not_queued(&mut self, text: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = text
            && self.text(cx).trim().is_empty()
        {
            self.input.update(cx, |s, cx| s.set_value(text, window, cx));
            self.changed(cx);
        }
        self.error = Some("The message couldn't be saved, so it wasn't sent.".into());
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let empty = text.trim().is_empty();
        if empty != self.empty {
            self.empty = empty;
            cx.notify();
        }
        // Typing indicators go out only where the server supports them;
        // the engine decides.
        self.session
            .read(cx)
            .engine
            .composing(&self.chat_guid, !empty);
        let store = self.session.read(cx).store.clone();
        let chat_id = self.chat_id;
        self.save_draft = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            cx.background_spawn(async move {
                let draft = (!text.trim().is_empty()).then_some(text);
                if let Err(e) = store.set_draft(chat_id, draft.as_deref()) {
                    tracing::warn!("saving a draft: {e}");
                }
            })
            .await;
        }));
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if text.trim().is_empty() {
            return;
        }
        self.session.read(cx).engine.send_text(TextMessage {
            chat_guid: self.chat_guid.clone(),
            text,
            ..Default::default()
        });
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.error = None;
        self.empty = true;
        self.save_draft = None;
        let store = self.session.read(cx).store.clone();
        let chat_id = self.chat_id;
        cx.background_spawn(async move {
            let _ = store.set_draft(chat_id, None);
        })
        .detach();
        cx.notify();
    }
}

impl Render for Composer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let send_color = if self.sms {
            colors.sms
        } else {
            colors.imessage
        };
        div()
            .flex_none()
            .px(px(14.))
            .pt(px(6.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .px(px(8.))
                        .text_size(m.small())
                        .text_color(colors.error)
                        .child(error),
                )
            })
            .child(
                div()
                    .w_full()
                    .min_h(px(30.))
                    .pl(px(6.))
                    .pr(px(3.))
                    .py(px(2.))
                    .rounded(px(17.))
                    .border_1()
                    .border_color(colors.field_border)
                    .bg(colors.field)
                    .flex()
                    .items_end()
                    .gap(px(6.))
                    .child(
                        div().flex_1().min_w_0().child(
                            Textarea::new(&self.input)
                                .appearance(false)
                                .small()
                                .text_size(m.message_text()),
                        ),
                    )
                    .child(
                        div()
                            .id("send")
                            .size(px(24.))
                            .flex_none()
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(!self.empty, |d| {
                                d.bg(send_color).cursor_pointer().child(
                                    icon(Icon::ArrowUp, px(14.)).text_color(colors.on_bubble),
                                )
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
    }
}
