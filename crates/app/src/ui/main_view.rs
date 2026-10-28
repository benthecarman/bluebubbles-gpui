//! The signed-in window: the sidebar beside the open conversation.

use crate::session::{Session, SessionEvent};
use crate::ui::conversation::Conversation;
use crate::ui::sidebar::{Sidebar, SidebarEvent};
use crate::ui::window_controls::{self, Side};
use bb_ui::ActiveTheme;
use bb_ui::theme::SIDEBAR_WIDTH;
use gpui_kit::*;

pub enum MainEvent {
    /// The server refused the password: back to the sign-in screen.
    SignedOut,
}

pub struct MainView {
    session: Entity<Session>,
    sidebar: Entity<Sidebar>,
    conversation: Option<Entity<Conversation>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MainEvent> for MainView {}

impl MainView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar = cx.new(|cx| Sidebar::new(session.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &sidebar,
                window,
                |this, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::Open(chat_id) => this.open(*chat_id, window, cx),
                    SidebarEvent::Loaded => this.open_requested(window, cx),
                },
            ),
            cx.observe_window_activation(window, |this, window, cx| this.report_view(window, cx)),
            cx.subscribe(&session, |_, _, event: &SessionEvent, cx| {
                if let SessionEvent::Status = event {
                    cx.notify();
                }
            }),
        ];
        Self {
            session,
            sidebar,
            conversation: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn open(&mut self, chat_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .conversation
            .as_ref()
            .is_some_and(|c| c.read(cx).chat_id == chat_id)
        {
            return;
        }
        let session = self.session.clone();
        let conversation = cx.new(|cx| Conversation::new(session, chat_id, window, cx));
        self.conversation = Some(conversation);
        self.sidebar.update(cx, |s, cx| s.select(Some(chat_id), cx));
        self.report_view(window, cx);
        cx.notify();
    }

    /// `BB_OPEN=<n>` opens the n-th conversation (from 1) at start, for
    /// screenshots.
    fn open_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation.is_some() {
            return;
        }
        let Some(n) = std::env::var("BB_OPEN")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
        else {
            return;
        };
        let chat = self
            .sidebar
            .read(cx)
            .chats()
            .get(n.saturating_sub(1))
            .map(|c| c.id);
        if let Some(chat) = chat {
            self.open(chat, window, cx);
        }
    }

    /// Tells the engine what is on screen: the open chat is marked read
    /// and doesn't notify while the window is focused.
    fn report_view(&self, window: &Window, cx: &App) {
        let chat = self.conversation.as_ref().map(|c| c.read(cx).chat_id);
        self.session
            .read(cx)
            .engine
            .view(chat, window.is_window_active());
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        if self.session.read(cx).status.connection == bb_core::Connection::Unauthorized {
            cx.emit(MainEvent::SignedOut);
        }
        let detail = match &self.conversation {
            Some(c) => c.clone().into_any_element(),
            None => window_controls::title_bar(div().id("empty-title").size_full())
                .relative()
                .children(
                    window_controls::buttons(Side::Right, window, cx)
                        .map(|b| div().absolute().top(px(14.)).right(px(12.)).child(b)),
                )
                .flex()
                .items_center()
                .justify_center()
                .text_size(m.title())
                .text_color(colors.text_tertiary)
                .child(if self.sidebar.read(cx).chats().is_empty() {
                    ""
                } else {
                    "No conversation selected"
                })
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(px(SIDEBAR_WIDTH))
                    .h_full()
                    .flex_none()
                    .border_r_1()
                    .border_color(colors.separator)
                    .child(self.sidebar.clone()),
            )
            .child(div().flex_1().min_w_0().h_full().child(detail))
    }
}
