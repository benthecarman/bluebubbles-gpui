//! The icon in the notification area: click it to open the window, or
//! right-click for Open, Settings and Quit, and Mark All as Read with a
//! red dot while conversations are unread.

pub use crate::tray_art::TrayEvent;
use crate::tray_art::{ICON, badge, unread_label};
use futures::channel::mpsc::UnboundedSender;
use gpui_kit::{App, Task};
use image::imageops::FilterType;
use std::rc::Rc;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Drawn at this size; Windows scales it to the notification area's
/// (16 at 100%, 32 at 200%).
const SIZE: u32 = 32;

const OPEN: &str = "open";
const MARK_ALL_READ: &str = "mark-all-read";
const SETTINGS: &str = "settings";
const QUIT: &str = "quit";

/// The running icon. It lives on the UI thread, whose message loop
/// delivers its clicks.
#[derive(Clone)]
pub struct Handle(Rc<Inner>);

struct Inner {
    icon: TrayIcon,
    plain: Icon,
    badged: Icon,
}

impl Handle {
    /// Shows the red dot, and the count in the menu, while `unread`.
    pub fn set_unread(&self, unread: usize, _cx: &App) {
        let inner = &self.0;
        let icon = if unread > 0 {
            &inner.badged
        } else {
            &inner.plain
        };
        if let Err(e) = inner.icon.set_icon(Some(icon.clone())) {
            tracing::debug!("tray icon: {e}");
        }
        let _ = inner.icon.set_tooltip(Some(tooltip(unread)));
        inner.icon.set_menu(Some(Box::new(menu(unread))));
    }
}

/// Adds the icon to the notification area, which Windows always has.
pub fn start(events: UnboundedSender<TrayEvent>, _cx: &App) -> Task<Result<Handle, String>> {
    Task::ready(build(events))
}

fn build(events: UnboundedSender<TrayEvent>) -> Result<Handle, String> {
    let (plain, badged) = icons().ok_or("the app icon didn't decode")?;
    let icon = TrayIconBuilder::new()
        .with_icon(plain.clone())
        .with_tooltip(tooltip(0))
        .with_menu(Box::new(menu(0)))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())?;
    let clicks = events.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = clicks.unbounded_send(TrayEvent::Open);
        }
    }));
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let picked = match event.id().as_ref() {
            OPEN => TrayEvent::Open,
            MARK_ALL_READ => TrayEvent::MarkAllRead,
            SETTINGS => TrayEvent::Settings,
            QUIT => TrayEvent::Quit,
            _ => return,
        };
        let _ = events.unbounded_send(picked);
    }));
    Ok(Handle(Rc::new(Inner {
        icon,
        plain,
        badged,
    })))
}

fn tooltip(unread: usize) -> String {
    match unread_label(unread) {
        Some(label) => format!("BlueBubbles\n{label}"),
        None => "BlueBubbles".into(),
    }
}

fn menu(unread: usize) -> Menu {
    let menu = Menu::new();
    let mut items: Vec<Box<dyn tray_icon::menu::IsMenuItem>> = Vec::new();
    if let Some(label) = unread_label(unread) {
        items.push(Box::new(MenuItem::new(label, false, None)));
        items.push(Box::new(MenuItem::with_id(
            MARK_ALL_READ,
            "Mark All as Read",
            true,
            None,
        )));
        items.push(Box::new(PredefinedMenuItem::separator()));
    }
    items.push(Box::new(MenuItem::with_id(
        OPEN,
        "Open BlueBubbles",
        true,
        None,
    )));
    items.push(Box::new(MenuItem::with_id(
        SETTINGS,
        "Settings…",
        true,
        None,
    )));
    items.push(Box::new(PredefinedMenuItem::separator()));
    items.push(Box::new(MenuItem::with_id(QUIT, "Quit", true, None)));
    for item in &items {
        let _ = menu.append(item.as_ref());
    }
    menu
}

/// The app icon, without and with the unread dot.
fn icons() -> Option<(Icon, Icon)> {
    let plain = image::load_from_memory(ICON)
        .ok()?
        .resize_exact(SIZE, SIZE, FilterType::Lanczos3)
        .to_rgba8();
    let mut badged = plain.clone();
    badge(&mut badged);
    let icon = |image: image::RgbaImage| Icon::from_rgba(image.into_raw(), SIZE, SIZE).ok();
    Some((icon(plain)?, icon(badged)?))
}
