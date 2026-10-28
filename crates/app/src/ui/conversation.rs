//! One conversation: the header, the transcript and the composer.

use crate::session::{Session, SessionEvent};
use crate::ui::composer::Composer;
use crate::ui::people::{self, Photos};
use crate::ui::transcript::{self, Bubble as Item_, Item, Key, Kind, Receipt, Transcript};
use crate::ui::{time, window_controls};
use bb_core::Names;
use bb_store::{ChatRow, MessageRow};
use bb_ui::theme::{GROUP_GAP, GROUPED_GAP, TAIL};
use bb_ui::{ActiveTheme, Avatar, Bubble, Face, Icon, Tail, icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use std::time::Duration;

/// Messages read from the cache at a time.
const PAGE: usize = 60;
/// Load older messages when the top is this close.
const LOAD_WITHIN: usize = 6;
const SMALL_AVATAR: f32 = 28.;

pub struct Conversation {
    session: Entity<Session>,
    pub chat_id: i64,
    chat: Option<ChatRow>,
    rows: Vec<MessageRow>,
    limit: usize,
    names: Names,
    photos: Photos,
    transcript: Rc<Transcript>,
    list: ListState,
    composer: Option<Entity<Composer>>,
    reloading: bool,
    reload_again: bool,
    wants_older: bool,
    /// Read the cache again on the next render (from places without a
    /// window).
    pending_reload: bool,
    _subscriptions: Vec<Subscription>,
}

impl Conversation {
    pub fn new(
        session: Entity<Session>,
        chat_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = ListState::new(0, ListAlignment::Bottom, px(1200.));
        list.set_follow_mode(FollowMode::Tail);
        let weak = cx.entity().downgrade();
        list.set_scroll_handler(move |event, _, cx| {
            if event.visible_range.start < LOAD_WITHIN {
                // Not now: the list is still borrowed while it scrolls.
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| this.load_older(cx));
                });
            }
        });
        let subscriptions = vec![cx.subscribe_in(
            &session,
            window,
            move |this, _, event: &SessionEvent, window, cx| match event {
                SessionEvent::Changed(changes) if changes.chats.contains(&this.chat_id) => {
                    this.reload(window, cx)
                }
                SessionEvent::ContactsChanged => this.reload(window, cx),
                SessionEvent::Typing { chat_id } if *chat_id == this.chat_id => this.rebuild(cx),
                _ => {}
            },
        )];
        let mut this = Self {
            session,
            chat_id,
            chat: None,
            rows: Vec::new(),
            limit: PAGE,
            names: Names::default(),
            photos: Photos::default(),
            transcript: Rc::new(Transcript {
                items: Vec::new(),
                keys: Vec::new(),
            }),
            list,
            composer: None,
            reloading: false,
            reload_again: false,
            wants_older: false,
            pending_reload: false,
            _subscriptions: subscriptions,
        };
        this.reload(window, cx);
        this
    }

    pub fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        if let Some(composer) = &self.composer {
            Composer::focus(composer, window, cx);
        }
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reloading {
            self.reload_again = true;
            return;
        }
        self.reloading = true;
        let session = self.session.read(cx);
        let store = session.store.clone();
        let region = session.region;
        let chat_id = self.chat_id;
        let limit = self.limit;
        let photos = self.photos.clone();
        let task = cx.background_spawn(async move {
            let chat = store.chat(chat_id)?;
            let rows = store.messages_before(chat_id, None, limit as u32)?;
            let mut addresses: Vec<String> =
                chat.iter().flat_map(|c| c.participants.clone()).collect();
            addresses.extend(rows.iter().filter_map(|m| m.sender.clone()));
            addresses.extend(rows.iter().filter_map(|m| m.other_address.clone()));
            let names = Names::load(&store, region, addresses.iter().map(String::as_str))?;
            let photos = photos.load_more(&store, &names, addresses.iter().map(String::as_str));
            Ok::<_, bb_store::StoreError>((chat, rows, names, photos))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.reloading = false;
                match result {
                    Ok((chat, rows, names, photos)) => {
                        if this.composer.is_none()
                            && let Some(chat) = &chat
                        {
                            let session = this.session.clone();
                            this.composer =
                                Some(cx.new(|cx| Composer::new(session, chat, window, cx)));
                            this.focus_composer(window, cx);
                        }
                        this.chat = chat;
                        this.rows = rows;
                        this.names = names;
                        this.photos = photos;
                        this.rebuild(cx);
                    }
                    Err(e) => tracing::error!("reading messages: {e}"),
                }
                if std::mem::take(&mut this.reload_again) {
                    this.reload(window, cx);
                }
            });
        })
        .detach();
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = &self.chat else {
            return;
        };
        let typing = self.session.read(cx).is_typing(self.chat_id);
        let service = people::chat_service(chat);
        let now = chrono::Local::now();
        let to_local = |ms: i64| time::local(ms);
        let next = transcript::build(
            &self.rows,
            &transcript::Context {
                names: &self.names,
                is_group: chat.is_group(),
                chat_service: &service,
                now,
                to_local: &to_local,
                typing,
            },
        );
        self.splice(&next.keys);
        self.transcript = Rc::new(next);
        self.wants_older = false;
        cx.notify();
    }

    /// Updates the list's rows in place, so older history arriving above
    /// keeps the reading position, and new messages below follow the tail.
    fn splice(&mut self, new: &[Key]) {
        let old = &self.transcript.keys;
        if old.as_slice() == new {
            // Same rows, new content (receipts, edits): measure them again.
            self.list.remeasure();
            return;
        }
        let first_old = old.iter().find(|k| !matches!(k, Key::Separator(_)));
        let anchor = first_old.and_then(|f| new.iter().position(|k| k == f));
        let old_anchor = first_old.and_then(|f| old.iter().position(|k| k == f));
        match (anchor, old_anchor) {
            (Some(at), Some(old_at))
                if at >= old_at
                    && new[at - old_at..]
                        .starts_with(&old[..old.len().min(new.len() - (at - old_at))]) =>
            {
                let prepended = at - old_at;
                self.list.splice(0..0, prepended);
                let kept = old.len().min(new.len() - prepended);
                let tail_start = prepended + kept;
                self.list
                    .splice(tail_start..self.list.item_count(), new.len() - tail_start);
                self.list.remeasure();
            }
            _ => self.list.reset(new.len()),
        }
    }

    fn load_older(&mut self, cx: &mut Context<Self>) {
        if self.wants_older || self.reloading {
            return;
        }
        let Some(chat) = &self.chat else {
            return;
        };
        let have_more_locally = self.rows.len() >= self.limit;
        if !have_more_locally && chat.history_done {
            return;
        }
        self.wants_older = true;
        self.limit += PAGE;
        if have_more_locally {
            // The cache has more: read them.
            self.pending_reload = true;
            cx.notify();
        } else {
            self.session.read(cx).engine.load_older(self.chat_id);
        }
    }

    fn header(&self, cx: &App) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let (title, faces) = match &self.chat {
            Some(chat) => (
                self.names.chat_title(chat),
                people::chat_faces(chat, &self.names, &self.photos),
            ),
            None => (String::new(), vec![Face::Glyph]),
        };
        let avatar = if faces.len() > 1 {
            Avatar::group(faces, px(30.)).ring(colors.window)
        } else {
            Avatar::new(faces.into_iter().next().unwrap_or(Face::Glyph), px(30.))
        };
        window_controls::title_bar(
            div()
                .id("conversation-title")
                .h(px(64.))
                .flex_none()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(3.))
                .border_b_1()
                .border_color(colors.separator),
        )
        .child(avatar)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .text_size(m.caption())
                .text_color(colors.text)
                .child(title)
                .child(icon(Icon::ChevronDown, px(9.)).text_color(colors.text_secondary)),
        )
    }
}

impl Render for Conversation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.pending_reload) {
            self.reload(window, cx);
        }
        let colors = *cx.colors();
        let transcript = self.transcript.clone();
        let names = self.names.clone();
        let photos = self.photos.clone();
        let session = self.session.downgrade();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.window)
            .child(self.header(cx))
            .child(
                list(self.list.clone(), move |ix, window, cx| {
                    let Some(item) = transcript.items.get(ix) else {
                        return div().into_any_element();
                    };
                    let last = ix + 1 == transcript.items.len();
                    render_item(item, &names, &photos, &session, last, window, cx)
                })
                .flex_1()
                .px(px(14.)),
            )
            .children(self.composer.clone())
    }
}

fn render_item(
    item: &Item,
    names: &Names,
    photos: &Photos,
    session: &WeakEntity<Session>,
    last: bool,
    _: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let colors = *cx.colors();
    let m = cx.theme().metrics();
    let bottom = if last { px(10.) } else { px(0.) };
    match item {
        Item::Separator { day, time } => div()
            .w_full()
            .pt(px(16.))
            .pb(px(6.))
            .pb(bottom.max(px(6.)))
            .flex()
            .justify_center()
            .gap(px(4.))
            .text_size(m.caption())
            .text_color(colors.text_secondary)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(day.clone()))
            .child(time.clone())
            .into_any_element(),
        Item::Event(text) => div()
            .w_full()
            .py(px(8.))
            .pb(bottom.max(px(8.)))
            .flex()
            .justify_center()
            .text_size(m.caption())
            .text_color(colors.text_secondary)
            .text_center()
            .child(text.clone())
            .into_any_element(),
        Item::Typing { avatar_slot } => div()
            .pt(px(GROUP_GAP))
            .pb(px(10.))
            .flex()
            .items_end()
            .when(*avatar_slot, |d| d.child(div().w(px(SMALL_AVATAR + 6.))))
            .child(typing_bubble(cx))
            .into_any_element(),
        Item::Bubble(b) => bubble(b, names, photos, session, bottom, cx),
    }
}

fn bubble(
    b: &Item_,
    names: &Names,
    photos: &Photos,
    session: &WeakEntity<Session>,
    bottom: Pixels,
    cx: &mut App,
) -> AnyElement {
    let colors = *cx.colors();
    let m = cx.theme().metrics();
    let from_me = b.row.is_from_me;
    let (fill, fg) = match b.kind {
        Kind::IMessage => (colors.imessage, colors.on_bubble),
        Kind::Sms => (colors.sms, colors.on_bubble),
        Kind::Received => (colors.received, colors.on_received),
    };
    let tail = match (b.tail, from_me) {
        (false, _) => Tail::None,
        (true, true) => Tail::Right,
        (true, false) => Tail::Left,
    };
    let pending = b.row.is_pending();
    let failed = matches!(b.receipt, Some(Receipt::Failed));
    let id = b.row.id;

    let mut content = div().flex().flex_col().gap(px(4.));
    for a in b.row.attachments.iter().filter(|a| !a.hidden) {
        content = content.child(attachment_card(a, fill, fg, cx));
    }
    if let Some(text) = &b.text {
        if b.jumbo {
            content = content.child(
                div()
                    .text_size(m.jumbo_emoji())
                    .line_height(m.jumbo_emoji() * 1.15)
                    .child(text.clone()),
            );
        } else {
            let mut body = div()
                .px(m.bubble_pad_x())
                .py(m.bubble_pad_y())
                .text_size(m.message_text())
                .line_height(m.message_line())
                .text_color(fg);
            if let Some(subject) = b.row.subject.as_deref().filter(|s| !s.trim().is_empty()) {
                body = body.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(subject.to_owned()),
                );
            }
            body = body.child(text.clone());
            content = content.child(Bubble::new(fill, m.bubble_radius(), tail).child(body));
        }
    }

    let column = div()
        .max_w(relative(0.72))
        .flex()
        .flex_col()
        .when(from_me, |d| d.items_end())
        .when(!from_me, |d| d.items_start())
        .when_some(b.sender_name.clone(), |d, name| {
            d.child(
                div()
                    .pl(px(12.))
                    .pb(px(2.))
                    .text_size(m.caption())
                    .text_color(colors.text_secondary)
                    .child(name),
            )
        })
        .child(content.when(pending, |d| d.opacity(0.65)))
        .when(
            b.row.date_edited.is_some() && b.row.date_retracted.is_none(),
            |d| {
                d.child(
                    div()
                        .px(px(4.))
                        .pt(px(2.))
                        .text_size(m.caption())
                        .text_color(colors.text_secondary)
                        .child("Edited"),
                )
            },
        )
        .when_some(b.receipt.clone(), |d, receipt| {
            d.child(match receipt {
                Receipt::Status(text) => div()
                    .pr(px(4.))
                    .pt(px(2.))
                    .text_size(m.caption())
                    .text_color(colors.text_secondary)
                    .child(text)
                    .into_any_element(),
                Receipt::Failed => div()
                    .pr(px(4.))
                    .pt(px(2.))
                    .text_size(m.caption())
                    .text_color(colors.error)
                    .child("Not Delivered")
                    .into_any_element(),
            })
        });

    let avatar = b.avatar_slot.then(|| {
        let face = match (&b.row.sender, b.tail) {
            (Some(sender), true) => Some(people::face(names, photos, sender)),
            _ => None,
        };
        div()
            .w(px(SMALL_AVATAR))
            .mr(px(6.))
            .flex_none()
            .when_some(face, |d, face| d.child(Avatar::new(face, px(SMALL_AVATAR))))
    });

    let session = session.clone();
    div()
        .id(("message", id as u64))
        .w_full()
        .pt(px(if b.first { GROUP_GAP } else { GROUPED_GAP }))
        .pb(bottom)
        .flex()
        .items_end()
        .when(from_me, |d| d.justify_end().pr(px(TAIL)))
        .when(!from_me, |d| d.pl(px(TAIL)))
        .children(avatar)
        .child(column)
        .when(failed, |d| {
            d.child(
                div()
                    .id(("retry", id as u64))
                    .ml(px(6.))
                    .mb(px(20.))
                    .cursor_pointer()
                    .text_color(colors.error)
                    .child(icon(Icon::Alert, px(20.)).text_color(colors.error))
                    .on_click(move |_, _, cx| {
                        if let Some(session) = session.upgrade() {
                            session.read(cx).engine.retry(id);
                        }
                    }),
            )
        })
        .into_any_element()
}

/// Until Phase 3 draws them, attachments show as a labelled card.
fn attachment_card(a: &bb_store::AttachmentRow, fill: Hsla, fg: Hsla, cx: &App) -> AnyElement {
    let m = cx.theme().metrics();
    let mime = a.mime_type.as_deref().unwrap_or_default();
    let label = if mime.starts_with("image/") {
        "Photo"
    } else if mime.starts_with("video/") {
        "Video"
    } else if mime.starts_with("audio/") {
        "Audio"
    } else {
        "File"
    };
    let name = a.transfer_name.clone().unwrap_or_default();
    div()
        .rounded(m.bubble_radius())
        .bg(fill)
        .text_color(fg)
        .px(m.bubble_pad_x())
        .py(px(10.))
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(m.message_text())
                .font_weight(FontWeight::MEDIUM)
                .child(label),
        )
        .child(div().text_size(m.caption()).opacity(0.75).child(name))
        .into_any_element()
}

/// The received bubble with three pulsing dots.
fn typing_bubble(cx: &App) -> impl IntoElement {
    let colors = *cx.colors();
    let m = cx.theme().metrics();
    let dot = |i: usize| {
        div()
            .size(px(8.))
            .rounded_full()
            .bg(colors.text_secondary)
            .with_animation(
                ("typing-dot", i),
                Animation::new(Duration::from_millis(1200)).repeat(),
                move |d, t| {
                    // Each dot brightens in turn.
                    let phase = (t - i as f32 * 0.18).rem_euclid(1.0);
                    let lift = (phase * std::f32::consts::TAU).sin().max(0.0);
                    d.opacity(0.35 + 0.65 * lift)
                },
            )
    };
    div().pl(px(TAIL)).child(
        Bubble::new(colors.received, m.bubble_radius(), Tail::Left).child(
            div()
                .h(m.message_line() + m.bubble_pad_y() * 2.)
                .px(px(14.))
                .flex()
                .items_center()
                .gap(px(4.))
                .child(dot(0))
                .child(dot(1))
                .child(dot(2)),
        ),
    )
}
