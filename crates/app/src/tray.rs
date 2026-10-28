//! The icon in the top bar (a StatusNotifierItem, which Ubuntu's GNOME,
//! KDE and most other desktops show): Open, Settings and Quit, Mark All
//! as Read and a red dot while conversations are unread. Plain GNOME
//! shows it with the AppIndicator extension.

use futures::channel::mpsc::UnboundedSender;
use image::imageops::FilterType;
use ksni::TrayMethods;

const ICON: &[u8] = include_bytes!("../../../assets/icons/bluebubbles-256.png");
/// Sizes handed over; the desktop picks the closest to its bar.
const SIZES: [u32; 4] = [22, 32, 48, 64];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    MarkAllRead,
    Settings,
    Quit,
    /// The desktop's tray host came or went.
    Available(bool),
}

pub type Handle = ksni::Handle<AppTray>;

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
pub async fn start(events: UnboundedSender<TrayEvent>) -> Result<Handle, ksni::Error> {
    let (plain, badged) = icons();
    AppTray {
        events,
        unread: 0,
        plain,
        badged,
    }
    .spawn()
    .await
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

fn unread_label(unread: usize) -> Option<String> {
    match unread {
        0 => None,
        1 => Some("1 unread conversation".into()),
        n => Some(format!("{n} unread conversations")),
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

/// A red dot in the top-right corner, ringed in white.
fn badge(image: &mut image::RgbaImage) {
    let size = image.width() as f32;
    let r = size * 0.22;
    let (cx, cy) = (size - r - 0.5, r + 0.5);
    for (x, y, p) in image.enumerate_pixels_mut() {
        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
        let ring = (r + 1.0 - d).clamp(0.0, 1.0);
        let dot = (r - 0.5 - d).clamp(0.0, 1.0);
        if ring > 0.0 {
            let (red, white) = ([255.0, 59.0, 48.0], [255.0, 255.0, 255.0]);
            for i in 0..3 {
                let over = red[i] * dot + white[i] * (1.0 - dot);
                p.0[i] = (over * ring + p.0[i] as f32 * (1.0 - ring)) as u8;
            }
            p.0[3] = (255.0 * ring + p.0[3] as f32 * (1.0 - ring)) as u8;
        }
    }
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
