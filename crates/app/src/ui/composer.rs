//! The message field: a rounded field that grows with its text, Enter to
//! send, Shift+Enter for a new line, and the round send arrow once there
//! is something to send. Drafts are kept per chat.
//!
//! Files dropped on the conversation, and images or files pasted, wait
//! in the field until the message is sent; they go out before its text.

use crate::config;
use crate::session::{Session, SessionEvent};
use crate::ui::media;
use crate::ui::people;
use bb_core::TextMessage;
use bb_ui::{ActiveTheme, Icon, icon};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The message being replied to, shown above the field.
#[derive(Clone)]
pub struct ReplyTo {
    pub guid: String,
    pub part: i64,
    /// "Alex", "You".
    pub sender: String,
    pub text: String,
}

/// A file waiting to be sent with the message.
struct Staged {
    path: PathBuf,
    name: String,
    /// A small copy for the field, for images.
    thumb: Option<Arc<RenderImage>>,
}

pub struct Composer {
    session: Entity<Session>,
    chat_id: i64,
    chat_guid: String,
    sms: bool,
    input: Entity<TextareaState>,
    empty: bool,
    save_draft: Option<Task<()>>,
    error: Option<SharedString>,
    staged: Vec<Staged>,
    reply_to: Option<ReplyTo>,
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
            staged: Vec::new(),
            reply_to: None,
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

    /// Makes the next message a reply (sent as one only where the server
    /// has the Private API).
    pub fn reply(this: &Entity<Self>, reply: ReplyTo, window: &mut Window, cx: &mut App) {
        this.update(cx, |this, cx| {
            this.reply_to = Some(reply);
            cx.notify();
        });
        Self::focus(this, window, cx);
    }

    /// Adds files to send with the message.
    pub fn stage(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            if !path.is_file() || self.staged.iter().any(|s| s.path == path) {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.staged.push(Staged {
                path: path.clone(),
                name,
                thumb: None,
            });
            let task = cx.background_spawn({
                let path = path.clone();
                async move { media::thumbnail(&path) }
            });
            cx.spawn(async move |this, cx| {
                if let Some(thumb) = task.await {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(s) = this.staged.iter_mut().find(|s| s.path == path) {
                            s.thumb = Some(thumb);
                        }
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// Files and images on the clipboard are staged; text pastes as text.
    fn paste(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let mut paths = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::ExternalPaths(p) => paths.extend(p.paths().iter().cloned()),
                ClipboardEntry::Image(image) => match save_pasted(image) {
                    Ok(path) => paths.push(path),
                    Err(e) => tracing::warn!("saving a pasted image: {e}"),
                },
                ClipboardEntry::String(_) => {}
            }
        }
        if paths.is_empty() {
            return false;
        }
        self.stage(paths, cx);
        true
    }

    fn unstage(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.staged.len() {
            self.staged.remove(ix);
            cx.notify();
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if text.trim().is_empty() && self.staged.is_empty() {
            return;
        }
        let engine = self.session.read(cx).engine.clone();
        let mut reply_to = self.reply_to.take().map(|r| (r.guid, r.part));
        for staged in std::mem::take(&mut self.staged) {
            engine.send_file(&self.chat_guid, staged.path, reply_to.take());
        }
        if !text.trim().is_empty() {
            engine.send_text(TextMessage {
                chat_guid: self.chat_guid.clone(),
                text,
                reply_to,
                ..Default::default()
            });
        }
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

/// A pasted image becomes a file the outbox can send (and retry) from.
fn save_pasted(image: &Image) -> std::io::Result<PathBuf> {
    let ext = match image.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::Webp => "webp",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
        _ => "png",
    };
    let dir = config::outgoing_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let path = dir.join(format!("Pasted Image {stamp}-{}.{ext}", image.id));
    std::fs::write(&path, &image.bytes)?;
    Ok(path)
}

impl Composer {
    fn staged_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        div()
            .w_full()
            .pt(px(6.))
            .pb(px(2.))
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(self.staged.iter().enumerate().map(|(ix, s)| {
                let remove = div()
                    .id(("unstage", ix))
                    .absolute()
                    .top(px(-5.))
                    .right(px(-5.))
                    .size(px(18.))
                    .rounded_full()
                    .bg(colors.text_secondary)
                    .border_1()
                    .border_color(colors.field)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(icon(Icon::WindowClose, px(12.)).text_color(colors.field))
                    .on_click(cx.listener(move |this, _, _, cx| this.unstage(ix, cx)));
                let chip = match &s.thumb {
                    Some(thumb) => div().child(
                        img(thumb.clone())
                            .h(px(64.))
                            .max_w(px(160.))
                            .rounded(px(10.))
                            .object_fit(ObjectFit::Cover),
                    ),
                    None => div()
                        .h(px(40.))
                        .max_w(px(220.))
                        .px(px(10.))
                        .rounded(px(10.))
                        .bg(colors.received)
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(Icon::Document, px(16.)).text_color(colors.accent))
                        .child(
                            div()
                                .truncate()
                                .text_size(m.small())
                                .text_color(colors.on_received)
                                .child(s.name.clone()),
                        ),
                };
                chip.relative().child(remove)
            }))
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
            .when_some(self.reply_to.clone(), |el, reply| {
                el.child(
                    div()
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(m.small())
                        .text_color(colors.text_secondary)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(format!("Replying to {}: {}", reply.sender, reply.text)),
                        )
                        .child(
                            div()
                                .id("cancel-reply")
                                .cursor_pointer()
                                .child(
                                    icon(Icon::WindowClose, px(14.))
                                        .text_color(colors.text_secondary),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reply_to = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
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
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(!self.staged.is_empty(), |d| d.child(self.staged_row(cx)))
                            .child(
                                Textarea::new(&self.input)
                                    .appearance(false)
                                    .small()
                                    .text_size(m.message_text())
                                    .on_paste({
                                        let this = cx.entity().downgrade();
                                        move |item, _, cx| {
                                            this.update(cx, |this, cx| this.paste(item, cx))
                                                .unwrap_or(false)
                                        }
                                    }),
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
                            .when(!self.empty || !self.staged.is_empty(), |d| {
                                d.bg(send_color).cursor_pointer().child(
                                    icon(Icon::ArrowUp, px(14.)).text_color(colors.on_bubble),
                                )
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
    }
}
