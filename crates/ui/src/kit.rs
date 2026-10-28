//! gpui-kit's components (text fields, scrollbars) in our palette.

use crate::fonts;
use crate::theme::{Appearance, Theme};
use gpui_kit::component::theme::{Theme as KitTheme, ThemeMode};
use gpui_kit::{App, px};

pub fn sync(cx: &mut App) {
    let theme = Theme::get(cx);
    let colors = theme.colors;
    let mode = match theme.appearance {
        Appearance::Light => ThemeMode::Light,
        Appearance::Dark => ThemeMode::Dark,
    };
    let scale = theme.text_scale;
    KitTheme::change(mode, None, cx);
    let kit = KitTheme::global_mut(cx);
    kit.font_family = fonts::SANS.into();
    kit.font_size = px(13. * scale);
    kit.background = colors.window;
    kit.foreground = colors.text;
    kit.border = colors.field_border;
    kit.input = colors.field_border;
    kit.ring = colors.accent.opacity(0.5);
    kit.caret = colors.accent;
    kit.selection = colors.selection;
    kit.muted_foreground = colors.placeholder;
    kit.popover = colors.popover;
    kit.popover_foreground = colors.text;
    kit.primary = colors.accent;
    kit.primary_foreground = colors.on_bubble;
    kit.scrollbar = gpui_kit::transparent_black();
    kit.scrollbar_thumb = colors.text_tertiary.opacity(0.6);
    kit.scrollbar_thumb_hover = colors.text_secondary;
    kit.radius = px(6.);
    kit.shadow = false;
}
