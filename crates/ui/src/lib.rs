//! The Messages look for GPUI.

pub mod bubble;
pub mod fonts;
pub mod theme;

pub use bubble::{Bubble, Tail};
pub use theme::{ActiveTheme, Appearance, Colors, Metrics, Preference, Theme};

use gpui_kit::{App, Styled, TextRenderingMode};

/// Registers the fonts and theme. Call once at startup, after
/// `gpui_kit::init`.
pub fn init(cx: &mut App) {
    fonts::register(cx);
    // Grayscale antialiasing, as macOS draws text, and no colour fringes
    // on coloured bubbles.
    cx.set_text_rendering_mode(TextRenderingMode::Grayscale);
    Theme::init(cx);
}

pub trait TypeStyled: Styled + Sized {
    /// Inter with colour emoji fallback; set once at a root, children
    /// inherit it.
    fn messages_type(mut self) -> Self {
        self = self.font_family(fonts::SANS);
        self.text_style().font_fallbacks = Some(fonts::fallbacks());
        self
    }
}

impl<T: Styled> TypeStyled for T {}
