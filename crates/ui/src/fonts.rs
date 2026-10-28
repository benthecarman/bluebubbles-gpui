//! Bundled fonts: static Inter (standing in for SF Pro, which can't be
//! redistributed) and Noto Color Emoji, both SIL OFL 1.1.
//!
//! GPUI's Linux text stack picks weights reliably only from static fonts,
//! draws colour glyphs only from a font with the PostScript name
//! `NotoColorEmoji`, and ignores a fallback family without a glyph for `m`.
//! `scripts/edit-fonts.py` prepares the files for all three.

use gpui_kit::{App, FontFallbacks};
use std::borrow::Cow;

pub const SANS: &str = "Inter";
pub const EMOJI: &str = "BlueBubbles Color Emoji";

const FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Italic.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Bold.ttf"),
    include_bytes!("../../../assets/fonts/emoji/BlueBubblesColorEmoji.ttf"),
];

pub fn register(cx: &mut App) {
    let fonts = FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("bb-ui: failed to register bundled fonts: {err:#}");
    }
}

/// Every text style falls back to colour emoji before any system font.
pub fn fallbacks() -> FontFallbacks {
    FontFallbacks::from_fonts(vec![EMOJI.to_string()])
}
