//! The icon in the top bar (a StatusNotifierItem, which Ubuntu's GNOME,
//! KDE and most other desktops show): Open, Settings and Quit, Mark All
//! as Read and a red dot while conversations are unread. Plain GNOME
//! shows it with the AppIndicator extension.

pub use crate::tray_art::TrayEvent;
use crate::tray_art::{ICON, badge, unread_label};
use futures::channel::mpsc::UnboundedSender;
use gpui_kit::{App, AppContext, Task};
use image::imageops::FilterType;
use ksni::TrayMethods;

/// Sizes handed over; the desktop picks the closest to its bar.
const SIZES: [u32; 4] = [22, 32, 48, 64];

/// The running icon.
#[derive(Clone)]
pub struct Handle(ksni::Handle<AppTray>);

impl Handle {
    /// Shows the red dot, and the count in the menu, while `unread`.
    pub fn set_unread(&self, unread: usize, cx: &App) {
        let handle = self.0.clone();
        cx.background_spawn(async move {
            handle.update(|t| t.set_unread(unread)).await;
        })
        .detach();
    }
}

pub struct AppTray {
    events: UnboundedSender<TrayEvent>,
    unread: usize,
    plain: Vec<ksni::Icon>,
    badged: Vec<ksni::Icon>,
}

impl AppTray {
    pub fn set_unread(&mut self, unread: usize) {
        self.unread = unread;
    }

    fn send(&self, event: TrayEvent) {
        let _ = self.events.unbounded_send(event);
    }
}

/// Registers the icon with the desktop's tray host. Fails when there is
/// none, as on GNOME without the AppIndicator extension.
pub fn start(events: UnboundedSender<TrayEvent>, cx: &App) -> Task<Result<Handle, String>> {
    cx.background_spawn(async move {
        let (plain, badged) = icons();
        AppTray {
            events,
            unread: 0,
            plain,
            badged,
        }
        .spawn()
        .await
        .map(Handle)
        .map_err(|e| e.to_string())
    })
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        crate::config::APP_ID.into()
    }

    fn title(&self) -> String {
        "BlueBubbles".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Communications
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        if self.unread > 0 {
            self.badged.clone()
        } else {
            self.plain.clone()
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "BlueBubbles".into(),
            description: unread_label(self.unread).unwrap_or_default(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let mut items = Vec::new();
        if let Some(label) = unread_label(self.unread) {
            items.push(
                StandardItem {
                    label,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            items.push(
                StandardItem {
                    label: "Mark All as Read".into(),
                    activate: Box::new(|t: &mut Self| t.send(TrayEvent::MarkAllRead)),
                    ..Default::default()
                }
                .into(),
            );
            items.push(ksni::MenuItem::Separator);
        }
        items.extend([
            StandardItem {
                label: "Open BlueBubbles".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Open)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Settings…".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Settings)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Quit)),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }

    fn watcher_online(&self) {
        self.send(TrayEvent::Available(true));
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.send(TrayEvent::Available(false));
        // Keep the service, so the icon comes back with the host.
        true
    }
}

/// The app icon at each size, without and with the unread dot.
fn icons() -> (Vec<ksni::Icon>, Vec<ksni::Icon>) {
    let Ok(icon) = image::load_from_memory(ICON) else {
        return (Vec::new(), Vec::new());
    };
    SIZES
        .iter()
        .map(|&size| {
            let plain = icon
                .resize_exact(size, size, FilterType::Lanczos3)
                .to_rgba8();
            let mut badged = plain.clone();
            badge(&mut badged);
            (pixmap(&plain), pixmap(&badged))
        })
        .unzip()
}

/// StatusNotifierItem pixmaps are ARGB32 in network byte order.
fn pixmap(image: &image::RgbaImage) -> ksni::Icon {
    let data = image
        .pixels()
        .flat_map(|p| [p.0[3], p.0[0], p.0[1], p.0[2]])
        .collect();
    ksni::Icon {
        width: image.width() as i32,
        height: image.height() as i32,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixmaps_are_argb_with_a_dot_when_unread() {
        let (plain, badged) = icons();
        assert_eq!(plain.len(), SIZES.len());
        let p = &plain[1];
        assert_eq!((p.width, p.height), (32, 32));
        assert_eq!(p.data.len(), 32 * 32 * 4);
        // The top-right pixel near the dot's centre is red in the badged one.
        let at = |icon: &ksni::Icon, x: usize, y: usize| {
            let i = (y * 32 + x) * 4;
            [
                icon.data[i],
                icon.data[i + 1],
                icon.data[i + 2],
                icon.data[i + 3],
            ]
        };
        assert_eq!(at(&badged[1], 24, 7), [255, 255, 59, 48]);
        assert_ne!(at(&plain[1], 24, 7), at(&badged[1], 24, 7));
        assert_eq!(unread_label(0), None);
        assert_eq!(unread_label(3).as_deref(), Some("3 unread conversations"));
    }
}
