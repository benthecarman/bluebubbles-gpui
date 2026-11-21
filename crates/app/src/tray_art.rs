//! What the tray icon looks like and does, on every platform: the app
//! icon, with a red dot while conversations are unread.

/// What was picked in the tray icon's menu, or happened to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    MarkAllRead,
    Settings,
    Quit,
    /// The desktop's tray host came or went. (Windows always has one.)
    #[cfg_attr(windows, allow(dead_code))]
    Available(bool),
}

pub const ICON: &[u8] = include_bytes!("../../../assets/icons/bluebubbles-256.png");

/// "3 unread conversations", for the tooltip and menu.
pub fn unread_label(unread: usize) -> Option<String> {
    match unread {
        0 => None,
        1 => Some("1 unread conversation".into()),
        n => Some(format!("{n} unread conversations")),
    }
}

/// A red dot in the top-right corner, ringed in white.
pub fn badge(image: &mut image::RgbaImage) {
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
