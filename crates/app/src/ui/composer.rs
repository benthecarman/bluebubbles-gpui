//! The message field: a rounded field that grows with its text, Enter to
//! send, Shift+Enter for a new line, and the round send arrow once there
//! is something to send. Drafts are kept per chat.
//!
//! Files dropped on the conversation, and images or files pasted, wait
//! in the field until the message is sent; they go out before its text.
//!
//! Misspelled words get a red squiggle once typed past; right-click one
//! for suggestions, Ignore Spelling or Add to Dictionary.
//!
//! `:joy:` becomes 😂 when its closing colon is typed, and from `:jo` a
//! list of matching emoji shows above the field: Up and Down pick, Enter
//! or Tab puts one in, Escape hides the list.

use crate::config;
use crate::emoji::{self, Suggestion};
use crate::session::{Session, SessionEvent};
use crate::shell::Shell;
use crate::spell;
use crate::ui::media;
use crate::ui::people;
use bb_core::TextMessage;
use bb_ui::{ActiveTheme, Icon, icon};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{
    Copy, Cut, Enter, Escape, IndentInline, InputEvent, MoveDown, MoveEnd, MoveHome, MoveLeft,
    MoveRight, MoveUp, Paste, SelectAll, Textarea, TextareaState,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde::Deserialize;
use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Most emoji the shortcode list shows.
const EMOJI_SUGGESTIONS: usize = 8;

/// The emoji list for the `:name` before the caret.
struct EmojiMenu {
    /// The `:name` typed, colon included.
    range: Range<usize>,
    query: String,
    items: Vec<Suggestion>,
    selected: usize,
}

/// Replaces a misspelled `word` at `range` with a suggestion.
#[derive(Clone, PartialEq, Deserialize, Action)]
#[action(namespace = composer, no_json)]
struct ReplaceMisspelling {
    range: Range<usize>,
    word: String,
    with: String,
}

#[derive(Clone, PartialEq, Deserialize, Action)]
#[action(namespace = composer, no_json)]
struct IgnoreSpelling {
    word: String,
}

#[derive(Clone, PartialEq, Deserialize, Action)]
#[action(namespace = composer, no_json)]
struct LearnSpelling {
    word: String,
}

/// The misspellings of the field's text, worked out again only when the
/// text, the word being typed or the accepted words change.
#[derive(Default)]
struct Spelling {
    key: Option<(SharedString, Option<Range<usize>>, u64)>,
    ranges: Vec<Range<usize>>,
}

impl Spelling {
    /// Misspelled ranges as `input` is now. While the field has focus, the
    /// word at the caret is left until it's typed past.
    fn ranges(&mut self, input: &TextareaState, focused: bool) -> &[Range<usize>] {
        let text = input.value();
        let caret = focused.then(|| input.cursor());
        let typing = caret.and_then(|c| word_at(&text, c));
        let key = (text.clone(), typing, spell::generation());
        if self.key.as_ref() != Some(&key) {
            self.ranges = spell::misspelled(&text, caret);
            self.key = Some(key);
        }
        &self.ranges
    }
}

/// The word containing or ending at `offset`.
fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    use unicode_segmentation::UnicodeSegmentation;
    text.unicode_word_indices()
        .map(|(start, w)| start..start + w.len())
        .find(|r| r.start <= offset && offset <= r.end)
}

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
    spelling: Rc<RefCell<Spelling>>,
    emoji: Option<EmojiMenu>,
    /// Where the `:name` whose list Escape hid starts; it stays hidden
    /// while that name is typed.
    emoji_dismissed: Option<usize>,
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
                        InputEvent::Change => {
                            this.changed(cx);
                            this.shortcodes(window, cx);
                        }
                        InputEvent::Blur => this.close_emoji(cx),
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
            spelling: Default::default(),
            emoji: None,
            emoji_dismissed: None,
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

    /// Turns a finished `:name:` into its emoji, and keeps the list for
    /// the one being typed up to date.
    fn shortcodes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.input.read(cx);
        let text = state.value();
        let caret = state.cursor();
        if !state.selected_range().is_empty() {
            self.close_emoji(cx);
            return;
        }
        if let Some((range, emoji)) = emoji::completed_at(&text, caret) {
            // The edit changes the text again, which closes the list.
            self.input.update(cx, |s, cx| {
                s.set_selected_range(range, cx);
                s.replace(emoji, window, cx);
            });
            return;
        }
        let typed = emoji::query_at(&text, caret);
        if self.emoji_dismissed != typed.as_ref().map(|(r, _)| r.start) {
            self.emoji_dismissed = None;
        }
        let menu = typed
            .filter(|(r, _)| self.emoji_dismissed != Some(r.start))
            .map(|(range, query)| (range, query, emoji::suggestions(query, EMOJI_SUGGESTIONS)))
            .filter(|(_, _, items)| !items.is_empty())
            .map(|(range, query, items)| EmojiMenu {
                // Keep the pick while the list is the same one.
                selected: self
                    .emoji
                    .as_ref()
                    .filter(|m| m.range.start == range.start && m.items == items)
                    .map_or(0, |m| m.selected),
                range,
                query: query.to_owned(),
                items,
            });
        if menu.is_some() || self.emoji.is_some() {
            self.emoji = menu;
            cx.notify();
        }
    }

    fn close_emoji(&mut self, cx: &mut Context<Self>) {
        if self.emoji.take().is_some() {
            cx.notify();
        }
    }

    /// Puts the picked emoji (or the selected one) in place of its
    /// `:name`.
    fn accept_emoji(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.emoji.take() else {
            return;
        };
        let Some(item) = menu.items.get(ix.unwrap_or(menu.selected)) else {
            return;
        };
        self.input.update(cx, |s, cx| {
            s.set_selected_range(menu.range.clone(), cx);
            s.replace(item.emoji, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    fn move_emoji(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        let Some(menu) = &mut self.emoji else {
            return false;
        };
        let n = menu.items.len();
        menu.selected = if down {
            (menu.selected + 1) % n
        } else {
            (menu.selected + n - 1) % n
        };
        cx.notify();
        true
    }

    /// The emoji list, over the transcript just above the field.
    fn emoji_menu(&self, menu: &EmojiMenu, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let rows = menu.items.iter().enumerate().map(|(ix, item)| {
            let selected = ix == menu.selected;
            div()
                .id(("emoji", ix))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .when(selected, |d| d.bg(colors.accent))
                .child(div().w(px(24.)).text_size(px(18.)).child(item.emoji))
                .child(
                    div()
                        .text_size(m.body())
                        .text_color(if selected {
                            gpui_kit::white()
                        } else {
                            colors.text
                        })
                        .truncate()
                        .child(format!(":{}:", item.shortcode)),
                )
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if let Some(menu) = &mut this.emoji
                        && *hovered
                        && menu.selected != ix
                    {
                        menu.selected = ix;
                        cx.notify();
                    }
                }))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.accept_emoji(Some(ix), window, cx)),
                )
        });
        deferred(
            div()
                .id("emoji-menu")
                .occlude()
                .absolute()
                .bottom_full()
                .left_0()
                .mb(px(6.))
                .w(px(280.))
                .p(px(4.))
                .rounded(px(10.))
                .bg(colors.popover)
                .border_1()
                .border_color(colors.popover_border)
                .shadow_lg()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(8.))
                        .pt(px(2.))
                        .pb(px(4.))
                        .text_size(m.caption())
                        .text_color(colors.text_secondary)
                        .child(format!("Emoji matching :{}", menu.query)),
                )
                .children(rows),
        )
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

    fn replace_misspelling(
        &mut self,
        action: &ReplaceMisspelling,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.update(cx, |s, cx| {
            // The text may have changed since the menu opened.
            if s.value().get(action.range.clone()) != Some(action.word.as_str()) {
                return;
            }
            s.set_selected_range(action.range.clone(), cx);
            s.replace(action.with.clone(), window, cx);
        });
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
            .on_action(cx.listener(Self::replace_misspelling))
            .on_action(cx.listener(|_, a: &IgnoreSpelling, _, cx| {
                spell::ignore(&a.word);
                cx.notify();
            }))
            .on_action(cx.listener(|_, a: &LearnSpelling, _, cx| {
                spell::learn(&a.word);
                cx.notify();
            }))
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
                    .relative()
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
                    .when_some(self.emoji.as_ref(), |d, menu| {
                        d.child(self.emoji_menu(menu, cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(!self.staged.is_empty(), |d| d.child(self.staged_row(cx)))
                            .child(
                                div()
                                    .relative()
                                    // While the emoji list shows, its keys
                                    // go to it rather than the field.
                                    .capture_action(cx.listener(|this, a: &Enter, window, cx| {
                                        if this.emoji.is_some() && !a.secondary {
                                            this.accept_emoji(None, window, cx);
                                            cx.stop_propagation();
                                        }
                                    }))
                                    .capture_action(cx.listener(
                                        |this, _: &IndentInline, window, cx| {
                                            if this.emoji.is_some() {
                                                this.accept_emoji(None, window, cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                                        if this.move_emoji(false, cx) {
                                            cx.stop_propagation();
                                        }
                                    }))
                                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                                        if this.move_emoji(true, cx) {
                                            cx.stop_propagation();
                                        }
                                    }))
                                    .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                                        if let Some(menu) = this.emoji.take() {
                                            this.emoji_dismissed = Some(menu.range.start);
                                            cx.notify();
                                            cx.stop_propagation();
                                        }
                                    }))
                                    // Moving the caret leaves the name.
                                    .capture_action(
                                        cx.listener(|this, _: &MoveLeft, _, cx| {
                                            this.close_emoji(cx)
                                        }),
                                    )
                                    .capture_action(cx.listener(|this, _: &MoveRight, _, cx| {
                                        this.close_emoji(cx)
                                    }))
                                    .capture_action(
                                        cx.listener(|this, _: &MoveHome, _, cx| {
                                            this.close_emoji(cx)
                                        }),
                                    )
                                    .capture_action(
                                        cx.listener(|this, _: &MoveEnd, _, cx| {
                                            this.close_emoji(cx)
                                        }),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| this.close_emoji(cx)),
                                    )
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
                                            })
                                            .context_menu({
                                                let input = self.input.clone();
                                                let spelling = self.spelling.clone();
                                                move |menu, _, cx| {
                                                    context_menu(menu, &input, &spelling, cx)
                                                }
                                            }),
                                    )
                                    .child(squiggles(
                                        self.input.clone(),
                                        self.spelling.clone(),
                                        colors.error,
                                    )),
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

fn spell_check_on(cx: &App) -> bool {
    cx.global::<Shell>().config.spell_check()
}

/// Red squiggles under misspelled words, drawn over the field after it
/// lays out its text.
fn squiggles(
    input: Entity<TextareaState>,
    spelling: Rc<RefCell<Spelling>>,
    color: Hsla,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, _, window, cx| {
            if !spell_check_on(cx) {
                return;
            }
            let state = input.read(cx);
            let focused = state.focus_handle(cx).is_focused(window);
            let mut spelling = spelling.borrow_mut();
            let ranges = spelling.ranges(state, focused);
            if ranges.is_empty() {
                return;
            }
            let style = UnderlineStyle {
                color: Some(color),
                thickness: px(1.),
                wavy: true,
            };
            let clip = ContentMask {
                bounds: state.input_bounds(),
            };
            window.with_content_mask(Some(clip), |window| {
                for range in ranges {
                    let Some(word) = state.range_to_bounds(range) else {
                        continue;
                    };
                    let line = state
                        .range_to_bounds(&(range.start..range.start))
                        .map_or(word.size.height, |b| b.size.height);
                    // A word broken across lines; rare enough to leave bare.
                    if word.size.height > line * 1.5 {
                        continue;
                    }
                    let origin = point(word.left(), word.bottom() - line * 0.18);
                    window.paint_underline(origin, word.size.width, &style);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

/// The field's right-click menu: for a misspelled word, its suggestions,
/// Ignore Spelling and Add to Dictionary above the usual editing items.
fn context_menu(
    mut menu: NativeMenu,
    input: &Entity<TextareaState>,
    spelling: &Rc<RefCell<Spelling>>,
    cx: &mut App,
) -> NativeMenu {
    let state = input.read(cx);
    let selection = state.selected_range();
    if spell_check_on(cx) {
        let text = state.value();
        // A right-click moved the caret to the word; it isn't being typed.
        let ranges = spelling.borrow_mut().ranges(state, false).to_vec();
        let caret = state.cursor();
        if let Some(range) = ranges
            .into_iter()
            .find(|r| r.start <= caret && caret <= r.end)
        {
            let word = text[range.clone()].to_owned();
            let suggestions = spell::suggestions(&word);
            if suggestions.is_empty() {
                menu = menu.menu_with_disabled("No Guesses Found", true, Box::new(NoAction));
            }
            for with in suggestions {
                menu = menu.menu(
                    with.clone(),
                    Box::new(ReplaceMisspelling {
                        range: range.clone(),
                        word: word.clone(),
                        with,
                    }),
                );
            }
            menu = menu
                .separator()
                .menu(
                    "Ignore Spelling",
                    Box::new(IgnoreSpelling { word: word.clone() }),
                )
                .menu("Add to Dictionary", Box::new(LearnSpelling { word }))
                .separator();
        }
    }
    let copyable = !selection.is_empty();
    menu.menu_with_disabled("Cut", !copyable, Box::new(Cut))
        .menu_with_disabled("Copy", !copyable, Box::new(Copy))
        .menu("Paste", Box::new(Paste))
        .separator()
        .menu("Select All", Box::new(SelectAll))
}
