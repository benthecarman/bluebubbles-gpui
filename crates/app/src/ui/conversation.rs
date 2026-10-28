//! One conversation: the header, the transcript and the composer.
//!
//! Photos download as they come into the transcript and are drawn from
//! small decoded copies; other files download when clicked and open in
//! the system's app for them.

use crate::session::{Session, SessionEvent};
use crate::ui::composer::{Composer, ReplyTo};
use crate::ui::media;
use crate::ui::people::{self, Photos};
use crate::ui::time;
use crate::ui::transcript::{
    self, Bubble as Item_, Item, Key, Kind, Link, Media, MediaKind, Quote, Receipt, Transcript,
};
use crate::ui::window_controls::{self, Side};
use bb_core::{Names, Reaction, Tapback, tapback};
use bb_store::{AttachmentRow, ChatRow, MessageRow};
use bb_ui::theme::{GROUP_GAP, GROUPED_GAP, TAIL};
use bb_ui::{ActiveTheme, Avatar, Bubble, Face, Icon, Tail, icon};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Messages read from the cache at a time.
const PAGE: usize = 60;
/// Load older messages when the top is this close.
const LOAD_WITHIN: usize = 6;
const SMALL_AVATAR: f32 = 28.;
/// Photos up to this size download as soon as they're in the transcript.
const AUTO_DOWNLOAD_MAX: i64 = 40_000_000;
/// Room above a bubble for its tapback badge.
const BADGE_ROOM: f32 = 16.;

/// Decoded photos by attachment GUID; `None` when the file isn't an
/// image we can decode (it shows as a file instead).
type Thumbs = HashMap<String, Option<Arc<RenderImage>>>;

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
    associated: HashMap<String, Vec<MessageRow>>,
    originals: HashMap<String, MessageRow>,
    thumbs: Rc<Thumbs>,
    decoding: HashSet<String>,
    /// Downloads asked of the engine, so each is asked once.
    requested: HashSet<String>,
    /// Files clicked before they were downloaded; they open on arrival.
    opening: Rc<HashSet<String>>,
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
            associated: HashMap::new(),
            originals: HashMap::new(),
            thumbs: Rc::default(),
            decoding: HashSet::new(),
            requested: HashSet::new(),
            opening: Rc::default(),
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
            let guids: Vec<String> = rows.iter().map(|m| m.guid.clone()).collect();
            let mut associated: HashMap<String, Vec<MessageRow>> = HashMap::new();
            for a in store.associated(&guids)? {
                if let Some(on) = a.associated_guid.clone() {
                    associated.entry(on).or_default().push(a);
                }
            }
            let mut originals = HashMap::new();
            for guid in rows
                .iter()
                .filter_map(|m| m.thread_originator_guid.as_ref())
            {
                if originals.contains_key(guid) {
                    continue;
                }
                let found = match rows.iter().find(|r| &r.guid == guid) {
                    Some(r) => Some(r.clone()),
                    None => store.message_by_guid(guid)?,
                };
                if let Some(original) = found {
                    originals.insert(guid.clone(), original);
                }
            }
            let mut addresses: Vec<String> =
                chat.iter().flat_map(|c| c.participants.clone()).collect();
            addresses.extend(rows.iter().filter_map(|m| m.sender.clone()));
            addresses.extend(rows.iter().filter_map(|m| m.other_address.clone()));
            addresses.extend(originals.values().filter_map(|m| m.sender.clone()));
            let names = Names::load(&store, region, addresses.iter().map(String::as_str))?;
            let photos = photos.load_more(&store, &names, addresses.iter().map(String::as_str));
            Ok::<_, bb_store::StoreError>((chat, rows, associated, originals, names, photos))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.reloading = false;
                match result {
                    Ok((chat, rows, associated, originals, names, photos)) => {
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
                        this.associated = associated;
                        this.originals = originals;
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
                associated: &self.associated,
                originals: &self.originals,
            },
        );
        self.splice(&next.keys);
        self.transcript = Rc::new(next);
        self.wants_older = false;
        self.fetch_media(cx);
        cx.notify();
    }

    /// Downloads the photos in the transcript, decodes the ones on disk,
    /// and opens files that were clicked before they arrived.
    fn fetch_media(&mut self, cx: &mut Context<Self>) {
        let engine = self.session.read(cx).engine.clone();
        let transcript = self.transcript.clone();
        for item in &transcript.items {
            let Item::Bubble(b) = item else {
                continue;
            };
            let photos = b
                .media
                .iter()
                .filter(|m| m.kind == MediaKind::Image)
                .map(|m| &m.attachment)
                .chain(b.link.as_ref().and_then(|l| l.image.as_ref()));
            for a in photos {
                match on_disk(a) {
                    Some(path) => {
                        if !self.thumbs.contains_key(&a.guid)
                            && self.decoding.insert(a.guid.clone())
                        {
                            self.decode(a.guid.clone(), path, cx);
                        }
                    }
                    None => {
                        if a.total_bytes.unwrap_or(0) <= AUTO_DOWNLOAD_MAX
                            && self.requested.insert(a.guid.clone())
                        {
                            engine.download(&a.guid);
                        }
                    }
                }
            }
            for m in &b.media {
                if let Some(path) = on_disk(&m.attachment)
                    && self.opening.contains(&m.attachment.guid)
                {
                    Rc::make_mut(&mut self.opening).remove(&m.attachment.guid);
                    cx.open_with_system(&path);
                }
            }
        }
    }

    fn decode(&mut self, guid: String, path: PathBuf, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { media::thumbnail(&path) });
        cx.spawn(async move |this, cx| {
            let thumb = task.await;
            let _ = this.update(cx, |this, cx| {
                this.decoding.remove(&guid);
                Rc::make_mut(&mut this.thumbs).insert(guid.clone(), thumb);
                // The photo's own aspect may differ from the reserved size.
                if let Some(ix) = this.transcript.items.iter().position(|item| {
                    matches!(item, Item::Bubble(b) if b.media.iter().any(|m| m.attachment.guid == guid)
                        || b.link.as_ref().and_then(|l| l.image.as_ref()).is_some_and(|a| a.guid == guid))
                }) {
                    this.list.remeasure_items(ix..ix + 1);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a file in the system's app for it, downloading it first.
    fn open_attachment(&mut self, a: &AttachmentRow, cx: &mut Context<Self>) {
        if let Some(path) = on_disk(a) {
            cx.open_with_system(&path);
            return;
        }
        Rc::make_mut(&mut self.opening).insert(a.guid.clone());
        self.requested.insert(a.guid.clone());
        self.session.read(cx).engine.download(&a.guid);
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

    fn header(&self, window: &Window, cx: &App) -> impl IntoElement {
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
                .relative()
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
        .children(window_controls::buttons(Side::Right, window, cx).map(|b| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(12.))
                .flex()
                .items_center()
                .child(b)
        }))
    }
}

impl Render for Conversation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.pending_reload) {
            self.reload(window, cx);
        }
        let colors = *cx.colors();
        let transcript = self.transcript.clone();
        let rcx = Rc::new(RenderCx {
            names: self.names.clone(),
            photos: self.photos.clone(),
            session: self.session.downgrade(),
            this: cx.entity().downgrade(),
            thumbs: self.thumbs.clone(),
            opening: self.opening.clone(),
            chat_guid: self
                .chat
                .as_ref()
                .map(|c| c.guid.clone())
                .unwrap_or_default(),
            private_api: self.session.read(cx).status.features.private_api,
            composer: self.composer.as_ref().map(|c| c.downgrade()),
        });
        div()
            .id("conversation")
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.window)
            // Files dropped anywhere on the conversation wait in the
            // composer.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(colors.sidebar_hover))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(composer) = this.composer.clone() {
                    composer.update(cx, |c, cx| c.stage(paths.paths().to_vec(), cx));
                    Composer::focus(&composer, window, cx);
                }
            }))
            .child(self.header(window, cx))
            .child(
                list(self.list.clone(), move |ix, window, cx| {
                    let Some(item) = transcript.items.get(ix) else {
                        return div().into_any_element();
                    };
                    let last = ix + 1 == transcript.items.len();
                    render_item(item, &rcx, last, window, cx)
                })
                .flex_1()
                .px(px(14.)),
            )
            .children(self.composer.clone())
    }
}

/// What drawing a transcript row needs from the conversation.
struct RenderCx {
    names: Names,
    photos: Photos,
    session: WeakEntity<Session>,
    this: WeakEntity<Conversation>,
    thumbs: Rc<Thumbs>,
    opening: Rc<HashSet<String>>,
    chat_guid: String,
    /// Tapbacks and replies can be sent (the server has the Private API).
    private_api: bool,
    composer: Option<WeakEntity<Composer>>,
}

fn on_disk(a: &AttachmentRow) -> Option<PathBuf> {
    a.local_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn render_item(
    item: &Item,
    rcx: &RenderCx,
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
        Item::Bubble(b) => bubble(b, rcx, bottom, cx),
    }
}

fn bubble(b: &Item_, rcx: &RenderCx, bottom: Pixels, cx: &mut App) -> AnyElement {
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

    let mut content = div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(4.))
        .when(from_me, |d| d.items_end())
        .when(!from_me, |d| d.items_start());
    for m in &b.media {
        content = content.child(media_view(m, rcx, cx));
    }
    if let Some(link) = &b.link {
        content = content.child(link_card(link, rcx, cx));
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
        // Above the bubble, outside it: the tapback badge sits on the
        // bubble, not on the quote.
        .when_some(b.quote.as_ref(), |d, quote| {
            d.child(div().pb(px(4.)).child(quote_view(quote, cx)))
        })
        .child(with_menu(
            content
                .id(("content", id as u64))
                .when(pending, |d| d.opacity(0.65))
                .when(!b.reactions.is_empty(), |d| {
                    // Room for the badge, between the bubble and whatever
                    // is above it.
                    d.mt(px(BADGE_ROOM))
                        .child(badge(&b.reactions, from_me, &rcx.names, cx))
                }),
            b,
            rcx,
        ))
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
            (Some(sender), true) => Some(people::face(&rcx.names, &rcx.photos, sender)),
            _ => None,
        };
        div()
            .w(px(SMALL_AVATAR))
            .mr(px(6.))
            .flex_none()
            .when_some(face, |d, face| d.child(Avatar::new(face, px(SMALL_AVATAR))))
    });

    let session = rcx.session.clone();
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

/// The bubble's right-click menu: Copy, and where the server has the
/// Private API, the tapbacks and Reply. Without anything to offer, no
/// menu.
fn with_menu(content: Stateful<Div>, b: &Item_, rcx: &RenderCx) -> AnyElement {
    let can_act = rcx.private_api && !b.row.is_pending() && !b.row.is_failed();
    let text = b.text.clone();
    if !can_act && text.is_none() {
        return content.into_any_element();
    }
    let session = rcx.session.clone();
    let composer = rcx.composer.clone();
    let chat_guid = rcx.chat_guid.clone();
    let guid = b.row.guid.clone();
    let mine = tapback::mine(&b.reactions).cloned();
    let reply = ReplyTo {
        guid: guid.clone(),
        part: 0,
        sender: match &b.row.sender {
            _ if b.row.is_from_me => "You".into(),
            Some(s) => rcx.names.short(s),
            None => String::new(),
        },
        text: rcx.names.preview(&b.row),
    };
    content
        .context_menu(move |mut menu, _, _| {
            if can_act {
                for t in Tapback::CLASSIC {
                    let on = mine.as_ref() == Some(&t);
                    let (session, chat_guid, guid) =
                        (session.clone(), chat_guid.clone(), guid.clone());
                    let label = format!("{}  {}", glyph_text(&t), tapback_name(&t));
                    menu = menu.item(PopupMenuItem::new(label).checked(on).on_click(
                        move |_, _, cx| {
                            if let Some(session) = session.upgrade() {
                                // Choosing your own tapback again takes it off.
                                session
                                    .read(cx)
                                    .engine
                                    .react(&chat_guid, &guid, 0, t.clone(), on);
                            }
                        },
                    ));
                }
                let (composer, reply) = (composer.clone(), reply.clone());
                menu = menu.separator().item(PopupMenuItem::new("Reply").on_click(
                    move |_, window, cx| {
                        if let Some(composer) = composer.as_ref().and_then(|c| c.upgrade()) {
                            Composer::reply(&composer, reply.clone(), window, cx);
                        }
                    },
                ));
            }
            if let Some(text) = text.clone() {
                if can_act {
                    menu = menu.separator();
                }
                menu = menu.item(PopupMenuItem::new("Copy").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                }));
            }
            menu
        })
        .into_any_element()
}

fn tapback_name(t: &Tapback) -> String {
    match t {
        Tapback::Love => "Love".into(),
        Tapback::Like => "Like".into(),
        Tapback::Dislike => "Dislike".into(),
        Tapback::Laugh => "Laugh".into(),
        Tapback::Emphasize => "Emphasize".into(),
        Tapback::Question => "Question".into(),
        Tapback::Emoji(e) => e.clone(),
    }
}

fn media_view(m: &Media, rcx: &RenderCx, cx: &App) -> AnyElement {
    let a = &m.attachment;
    let thumb = rcx.thumbs.get(&a.guid);
    if m.kind == MediaKind::Image && !matches!(thumb, Some(None)) {
        return photo(a, thumb.cloned().flatten(), rcx, cx);
    }
    file_card(m, rcx, cx)
}

/// A photo at its own aspect ratio, or a grey box of that size while it
/// downloads. Clicking opens the full file.
fn photo(
    a: &AttachmentRow,
    thumb: Option<Arc<RenderImage>>,
    rcx: &RenderCx,
    cx: &App,
) -> AnyElement {
    let colors = *cx.colors();
    let radius = cx.theme().metrics().bubble_radius();
    let (w, h) = match &thumb {
        Some(t) => {
            let size = t.size(0);
            media::display_size(Some(size.width.0 as i64), Some(size.height.0 as i64))
        }
        None => media::display_size(a.width, a.height),
    };
    let this = rcx.this.clone();
    let attachment = a.clone();
    div()
        .id(SharedString::from(format!("photo-{}", a.guid)))
        .w(px(w))
        .h(px(h))
        .rounded(radius)
        .bg(colors.received)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .map(|d| match thumb {
            Some(t) => d.child(
                img(t)
                    .w(px(w))
                    .h(px(h))
                    .rounded(radius)
                    .object_fit(ObjectFit::Cover),
            ),
            None => d.child(icon(Icon::Photo, px(28.)).text_color(colors.text_tertiary)),
        })
        .on_click(move |_, _, cx| {
            let _ = this.update(cx, |this, cx| this.open_attachment(&attachment, cx));
        })
        .into_any_element()
}

/// Videos, audio and other files: a card that downloads the file when
/// clicked and opens it in the system's app (no player runs in here).
fn file_card(m: &Media, rcx: &RenderCx, cx: &App) -> AnyElement {
    let colors = *cx.colors();
    let metrics = cx.theme().metrics();
    let a = &m.attachment;
    let (glyph, title) = match m.kind {
        MediaKind::Video => (Icon::Play, "Video".to_owned()),
        MediaKind::Audio => (Icon::Waveform, "Audio Message".to_owned()),
        MediaKind::Image => (Icon::Photo, "Photo".to_owned()),
        MediaKind::File => (
            Icon::Document,
            a.transfer_name.clone().unwrap_or_else(|| "File".into()),
        ),
    };
    let detail = if rcx.opening.contains(&a.guid) {
        "Downloading…".to_owned()
    } else {
        let size = a.total_bytes.filter(|b| *b > 0).map(media::file_size);
        let name = (m.kind != MediaKind::File)
            .then(|| a.transfer_name.clone())
            .flatten();
        [name, size]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let this = rcx.this.clone();
    let attachment = a.clone();
    div()
        .id(SharedString::from(format!("file-{}", a.guid)))
        .w(px(240.))
        .px(px(12.))
        .py(px(10.))
        .rounded(metrics.bubble_radius())
        .bg(colors.received)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .size(px(32.))
                .flex_none()
                .rounded_full()
                .bg(colors.window)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(glyph, px(16.)).text_color(colors.accent)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_size(metrics.message_text())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.on_received)
                        .child(title),
                )
                .when(!detail.is_empty(), |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(metrics.caption())
                            .text_color(colors.text_secondary)
                            .child(detail),
                    )
                }),
        )
        .on_click(move |_, _, cx| {
            let _ = this.update(cx, |this, cx| this.open_attachment(&attachment, cx));
        })
        .into_any_element()
}

/// A link preview: the sender's picture of the page, then its title and
/// site. Nothing is fetched from the site itself.
fn link_card(link: &Link, rcx: &RenderCx, cx: &App) -> AnyElement {
    let colors = *cx.colors();
    let metrics = cx.theme().metrics();
    let radius = metrics.bubble_radius();
    let width = media::MAX_W;
    let thumb = link
        .image
        .as_ref()
        .and_then(|a| rcx.thumbs.get(&a.guid).cloned().flatten());
    let title = match (&link.title, link.loading) {
        (Some(t), _) => t.clone(),
        (None, true) => "Loading preview…".into(),
        (None, false) => link.host.clone(),
    };
    let has_image = thumb.is_some();
    let url = link.url.clone();
    div()
        .id(SharedString::from(format!("link-{}", link.url)))
        .w(px(width))
        .flex()
        .flex_col()
        .cursor_pointer()
        .when_some(thumb, |d, t| {
            let size = t.size(0);
            let aspect = size.height.0 as f32 / size.width.0.max(1) as f32;
            let h = (width * aspect).clamp(80., 200.);
            d.child(
                img(t)
                    .w(px(width))
                    .h(px(h))
                    .rounded_t(radius)
                    .object_fit(ObjectFit::Cover),
            )
        })
        .child(
            div()
                .w_full()
                .px(px(12.))
                .py(px(9.))
                .bg(colors.received)
                .rounded_b(radius)
                .when(!has_image, |d| d.rounded_t(radius))
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(metrics.message_text())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.on_received)
                        .line_clamp(2)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(metrics.caption())
                        .text_color(colors.text_secondary)
                        .truncate()
                        .child(link.host.clone()),
                ),
        )
        .on_click(move |_, _, cx| cx.open_url(&url))
        .into_any_element()
}

/// The message replied to, small and faded above the reply.
fn quote_view(quote: &Quote, cx: &App) -> AnyElement {
    let colors = *cx.colors();
    let metrics = cx.theme().metrics();
    div()
        .max_w(px(media::MAX_W))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(14.))
        .border_1()
        .border_color(colors.separator)
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(metrics.caption())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.text_secondary)
                .child(quote.sender.clone()),
        )
        .child(
            div()
                .text_size(metrics.small())
                .text_color(colors.text_secondary)
                .line_clamp(2)
                .child(quote.text.clone()),
        )
        .into_any_element()
}

/// The tapbacks on a message, as Messages draws them: a small bubble on
/// the bubble's top corner, blue when one of them is yours. Hovering
/// lists who reacted.
fn badge(reactions: &[Reaction], from_me: bool, names: &Names, cx: &App) -> AnyElement {
    let colors = *cx.colors();
    let mine = reactions.iter().any(|r| r.from_me);
    let (bg, fg) = if mine {
        (colors.imessage, colors.on_bubble)
    } else {
        (colors.received, colors.text_secondary)
    };
    // Newest first, each kind once, at most three.
    let mut kinds: Vec<&Tapback> = Vec::new();
    for r in reactions.iter().rev() {
        if !kinds.contains(&&r.tapback) && kinds.len() < 3 {
            kinds.push(&r.tapback);
        }
    }
    let who = reactions
        .iter()
        .map(|r| {
            let name = match &r.sender {
                _ if r.from_me => "You".to_owned(),
                Some(s) => names.short(s),
                None => String::new(),
            };
            format!("{} {name}", glyph_text(&r.tapback))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let count = reactions.len();
    div()
        .id("tapbacks")
        .absolute()
        .top(px(-BADGE_ROOM - 4.))
        .when(from_me, |d| d.left(px(-12.)))
        .when(!from_me, |d| d.right(px(-12.)))
        .h(px(30.))
        .min_w(px(30.))
        .px(px(7.))
        .rounded_full()
        .bg(bg)
        .border_2()
        .border_color(colors.window)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(3.))
        .children(kinds.into_iter().map(|t| tapback_glyph(t, fg)))
        .when(count > 3, |d| {
            d.child(
                div()
                    .text_size(px(10.))
                    .text_color(fg)
                    .child(count.to_string()),
            )
        })
        .tooltip(move |window, cx| Tooltip::new(who.clone()).build(window, cx))
        .into_any_element()
}

fn glyph_text(t: &Tapback) -> String {
    match t {
        Tapback::Love => "❤️".into(),
        Tapback::Like => "👍".into(),
        Tapback::Dislike => "👎".into(),
        Tapback::Laugh => "😂".into(),
        Tapback::Emphasize => "‼️".into(),
        Tapback::Question => "❓".into(),
        Tapback::Emoji(e) => e.clone(),
    }
}

fn tapback_glyph(t: &Tapback, fg: Hsla) -> AnyElement {
    let text = |s: &'static str, size: f32| {
        div()
            .text_size(px(size))
            .font_weight(FontWeight::BOLD)
            .text_color(fg)
            .child(s)
            .into_any_element()
    };
    match t {
        Tapback::Love => icon(Icon::Heart, px(15.)).text_color(fg).into_any_element(),
        Tapback::Like => icon(Icon::ThumbsUp, px(15.))
            .text_color(fg)
            .into_any_element(),
        Tapback::Dislike => icon(Icon::ThumbsDown, px(15.))
            .text_color(fg)
            .into_any_element(),
        Tapback::Laugh => text("HA HA", 9.),
        Tapback::Emphasize => text("!!", 14.),
        Tapback::Question => text("?", 15.),
        Tapback::Emoji(e) => div().text_size(px(15.)).child(e.clone()).into_any_element(),
    }
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
