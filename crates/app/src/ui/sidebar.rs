//! The conversation list: avatar, name, time, a two-line preview and the
//! unread dot, newest first, with search.

use crate::session::{Session, SessionEvent};
use crate::shell::{self, OpenSettings};
use crate::ui::people::{self, Photos};
use crate::ui::time;
use crate::ui::window_controls::{self, Side};
use bb_core::{Connection, Names};
use bb_store::ChatRow;
use bb_ui::{ActiveTheme, Avatar, Colors, Icon, icon};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// The More menu at the top of the list, where iOS keeps Read All
/// behind Edit.
fn more_menu(any_unread: bool, private_api: bool, colors: Colors) -> impl IntoElement {
    let menu = Button::new("more")
        .ghost()
        .compact()
        .tooltip("More")
        .child(icon(Icon::More, px(18.)).text_color(colors.text_secondary))
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.item(
                PopupMenuItem::new("Mark All as Read")
                    .disabled(!any_unread)
                    .on_click(|_, _, cx| shell::mark_all_read(cx)),
            );
            if !private_api {
                // The server can't mark chats read on the Mac without it.
                menu = menu.item(PopupMenuItem::label(
                    "Only on this computer without the Private API",
                ));
            }
            menu.separator().item(
                PopupMenuItem::new("Settings…")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            )
        });
    // Around the popover, so its trigger sees the press first.
    window_controls::not_title_bar(div().id("more-menu").child(menu))
}

pub enum SidebarEvent {
    Open(i64),
    /// The list was read from the cache.
    Loaded,
}

pub struct Sidebar {
    session: Entity<Session>,
    chats: Vec<ChatRow>,
    shown: Vec<usize>,
    names: Names,
    photos: Photos,
    selected: Option<i64>,
    search: Entity<InputState>,
    scroll: UniformListScrollHandle,
    reloading: bool,
    reload_again: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

const ROW: f32 = 68.;

impl Sidebar {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let subscriptions = vec![
            cx.subscribe(&session, |this, _, event: &SessionEvent, cx| match event {
                SessionEvent::Changed(changes) if !changes.chats.is_empty() => this.reload(cx),
                SessionEvent::ContactsChanged => this.reload(cx),
                SessionEvent::Status => cx.notify(),
                _ => {}
            }),
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.filter(cx);
                }
            }),
        ];
        let mut this = Self {
            session,
            chats: Vec::new(),
            shown: Vec::new(),
            names: Names::default(),
            photos: Photos::default(),
            selected: None,
            search,
            scroll: UniformListScrollHandle::new(),
            reloading: false,
            reload_again: false,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    pub fn select(&mut self, chat_id: Option<i64>, cx: &mut Context<Self>) {
        self.selected = chat_id;
        cx.notify();
    }

    pub fn chats(&self) -> &[ChatRow] {
        &self.chats
    }

    /// Re-reads the list off the UI thread; a change while reading reads
    /// once more afterwards.
    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.reloading {
            self.reload_again = true;
            return;
        }
        self.reloading = true;
        let session = self.session.read(cx);
        let store = session.store.clone();
        let region = session.region;
        let photos = self.photos.clone();
        let task = cx.background_spawn(async move {
            let chats = store.chats(false)?;
            let names = Names::for_chats(&store, region, &chats)?;
            let photos = photos.load_more(
                &store,
                &names,
                chats
                    .iter()
                    .flat_map(|c| c.participants.iter().take(2).map(String::as_str)),
            );
            Ok::<_, bb_store::StoreError>((chats, names, photos))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.reloading = false;
                match result {
                    Ok((chats, names, photos)) => {
                        this.chats = chats;
                        this.names = names;
                        this.photos = photos;
                        this.filter(cx);
                        cx.emit(SidebarEvent::Loaded);
                    }
                    Err(e) => tracing::error!("reading chats: {e}"),
                }
                if std::mem::take(&mut this.reload_again) {
                    this.reload(cx);
                }
            });
        })
        .detach();
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.shown = self
            .chats
            .iter()
            .enumerate()
            .filter(|(_, chat)| {
                query.is_empty()
                    || self.names.chat_title(chat).to_lowercase().contains(&query)
                    || chat
                        .participants
                        .iter()
                        .any(|p| p.to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
            .collect();
        cx.notify();
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self.shown.get(ix).and_then(|&i| self.chats.get(i)) else {
            return div().into_any_element();
        };
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let selected = self.selected == Some(chat.id);
        let (fg, secondary) = if selected {
            (
                colors.sidebar_selected_text,
                colors.sidebar_selected_text.opacity(0.8),
            )
        } else {
            (colors.text, colors.text_secondary)
        };
        let preview = match &chat.last_message {
            Some(m) => self.names.preview(m),
            None => String::new(),
        };
        let draft = chat.draft.clone().filter(|d| !d.trim().is_empty());
        let when = chat
            .last_message
            .as_ref()
            .map(|m| time::sidebar(&time::local(m.date_created), &chrono::Local::now()))
            .unwrap_or_default();
        let faces = people::chat_faces(chat, &self.names, &self.photos);
        let avatar = if faces.len() > 1 {
            Avatar::group(faces, px(bb_ui::theme::AVATAR)).ring(if selected {
                colors.sidebar_selected
            } else {
                colors.sidebar
            })
        } else {
            Avatar::new(
                faces.into_iter().next().unwrap_or(bb_ui::Face::Glyph),
                px(bb_ui::theme::AVATAR),
            )
        };
        let id = chat.id;
        let unread = chat.unread;
        let session = self.session.clone();
        div()
            .id(("chat", chat.id as u64))
            .h(px(ROW))
            .px(px(8.))
            .child(
                div()
                    .size_full()
                    .rounded(px(8.))
                    .when(selected, |d| d.bg(colors.sidebar_selected))
                    .when(!selected, |d| d.hover(|s| s.bg(colors.sidebar_hover)))
                    .flex()
                    .items_center()
                    .pr(px(10.))
                    .child(
                        // The unread dot sits in the margin left of the avatar.
                        div().w(px(18.)).flex().justify_center().child(
                            div().size(px(9.)).rounded_full().when(chat.unread, |d| {
                                d.bg(if selected {
                                    colors.sidebar_selected_text
                                } else {
                                    colors.unread
                                })
                            }),
                        ),
                    )
                    .child(avatar)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ml(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(m.body())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(fg)
                                            .child(self.names.chat_title(chat)),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(m.small())
                                            .text_color(secondary)
                                            .child(when),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(m.body())
                                    .line_height(px(16. * m.scale))
                                    .text_color(secondary)
                                    .line_clamp(2)
                                    .when_some(draft.clone(), |d, draft| {
                                        d.child(
                                            div().child(
                                                StyledText::new(format!("Draft: {draft}"))
                                                    .with_highlights([(
                                                        0.."Draft:".len(),
                                                        HighlightStyle {
                                                            color: Some(if selected {
                                                                fg
                                                            } else {
                                                                colors.error
                                                            }),
                                                            ..Default::default()
                                                        },
                                                    )]),
                                            ),
                                        )
                                    })
                                    .when(draft.is_none(), |d| d.child(preview)),
                            ),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id);
                cx.emit(SidebarEvent::Open(id));
                cx.notify();
            }))
            .context_menu(move |menu, _, _| {
                let session = session.clone();
                let label = if unread {
                    "Mark as Read"
                } else {
                    "Mark as Unread"
                };
                menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let engine = &session.read(cx).engine;
                    if unread {
                        engine.mark_read(id);
                    } else {
                        engine.mark_unread(id);
                    }
                }))
            })
            .into_any_element()
    }

    fn connection_note(&self, cx: &App) -> Option<(Icon, &'static str)> {
        let status = &self.session.read(cx).status;
        // Brief drops aren't worth a banner.
        let long = status
            .offline_since
            .is_some_and(|since| chrono::Utc::now().timestamp_millis() - since > 4_000);
        match status.connection {
            Connection::Unauthorized => Some((Icon::Alert, "The server refused the password")),
            Connection::Offline if long => Some((Icon::Offline, "Not connected to the server")),
            Connection::Connecting if !status.synced => Some((Icon::Offline, "Connecting…")),
            _ => None,
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let count = self.shown.len();
        let note = self.connection_note(cx);
        let syncing_first = !self.session.read(cx).status.synced && self.chats.is_empty();
        let any_unread = self.chats.iter().any(|c| c.unread);
        let private_api = self.session.read(cx).status.features.private_api;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .child(
                window_controls::title_bar(
                    div()
                        .id("sidebar-title")
                        .h(px(52.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(12.)),
                )
                .children(window_controls::buttons(Side::Left, window, cx))
                .child(div().flex_1())
                .child(more_menu(any_unread, private_api, colors))
                .child(
                    window_controls::not_title_bar(
                        div()
                            .id("compose")
                            .p(px(4.))
                            .rounded(px(5.))
                            .text_color(colors.text_secondary)
                            .hover(|s| s.bg(colors.sidebar_hover)),
                    )
                    .child(icon(Icon::Compose, px(18.)).text_color(colors.text_secondary)),
                ),
            )
            .child(
                div().px(px(12.)).pb(px(10.)).flex_none().child(
                    div()
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(7.))
                        .bg(colors.search_field)
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(colors.text_secondary)
                        .child(icon(Icon::Search, px(13.)).text_color(colors.text_secondary))
                        .child(
                            div().flex_1().child(
                                Input::new(&self.search)
                                    .appearance(false)
                                    .text_size(m.body()),
                            ),
                        ),
                ),
            )
            .when_some(note, |el, (glyph, text)| {
                el.child(
                    div()
                        .mx(px(12.))
                        .mb(px(8.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(7.))
                        .bg(colors.sidebar_hover)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(m.small())
                        .text_color(colors.text_secondary)
                        .child(icon(glyph, px(14.)).text_color(colors.text_secondary))
                        .child(text),
                )
            })
            .when(syncing_first, |el| {
                el.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(m.body())
                        .text_color(colors.text_secondary)
                        .child("Loading conversations…"),
                )
            })
            .when(!syncing_first, |el| {
                el.child(
                    uniform_list(
                        "chats",
                        count,
                        cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                            range.map(|ix| this.row(ix, cx)).collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .flex_1(),
                )
            })
    }
}
