//! The Messages palette and type scale, light and dark, following the
//! system appearance unless the user picks one.

use gpui_kit::{
    App, BorrowAppContext, Global, Hsla, Pixels, Rgba, Window, WindowAppearance, px, rgb, rgba,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl From<WindowAppearance> for Appearance {
    fn from(a: WindowAppearance) -> Self {
        match a {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        }
    }
}

/// Colours named for where Messages uses them.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub window: Hsla,
    pub sidebar: Hsla,
    pub sidebar_selected: Hsla,
    pub sidebar_selected_text: Hsla,
    pub sidebar_hover: Hsla,
    pub separator: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub accent: Hsla,
    pub imessage: Hsla,
    pub sms: Hsla,
    pub on_bubble: Hsla,
    pub received: Hsla,
    pub on_received: Hsla,
    /// The thin line linking a reply to the message it answers.
    pub reply_line: Hsla,
    pub field: Hsla,
    pub field_border: Hsla,
    pub placeholder: Hsla,
    pub unread: Hsla,
    pub avatar_top: Hsla,
    pub avatar_bottom: Hsla,
    pub error: Hsla,
    pub popover: Hsla,
    pub popover_border: Hsla,
    pub shadow: Hsla,
    pub selection: Hsla,
    pub search_field: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex_rgba: u32) -> Hsla {
    let color: Rgba = rgba(hex_rgba);
    color.into()
}

impl Colors {
    pub fn light() -> Self {
        Self {
            window: c(0xFFFFFF),
            sidebar: c(0xF1F1F3),
            sidebar_selected: c(0x0A82FF),
            sidebar_selected_text: c(0xFFFFFF),
            sidebar_hover: ca(0x0000000D),
            separator: ca(0x3C3C4329),
            text: c(0x1C1C1E),
            text_secondary: c(0x86868B),
            text_tertiary: c(0xAEAEB2),
            accent: c(0x0A82FF),
            imessage: c(0x0B84FE),
            sms: c(0x34C759),
            on_bubble: c(0xFFFFFF),
            received: c(0xE9E9EB),
            on_received: c(0x000000),
            reply_line: c(0xC7C7CC),
            field: c(0xFFFFFF),
            field_border: c(0xD1D1D6),
            placeholder: c(0xA1A1A6),
            unread: c(0x0A82FF),
            avatar_top: c(0xA9AEBB),
            avatar_bottom: c(0x878B96),
            error: c(0xFF3B30),
            popover: c(0xFFFFFF),
            popover_border: ca(0x0000001F),
            shadow: ca(0x00000033),
            selection: ca(0x0A82FF40),
            search_field: ca(0x7676801F),
        }
    }

    pub fn dark() -> Self {
        Self {
            window: c(0x1E1E1E),
            sidebar: c(0x262628),
            sidebar_selected: c(0x0A84FF),
            sidebar_selected_text: c(0xFFFFFF),
            sidebar_hover: ca(0xFFFFFF0F),
            separator: ca(0x54545899),
            text: c(0xF2F2F7),
            text_secondary: c(0x98989D),
            text_tertiary: c(0x636366),
            accent: c(0x0A84FF),
            imessage: c(0x0A84FF),
            sms: c(0x30D158),
            on_bubble: c(0xFFFFFF),
            received: c(0x3B3B3D),
            on_received: c(0xFFFFFF),
            reply_line: c(0x48484A),
            field: c(0x1E1E1E),
            field_border: c(0x3A3A3C),
            placeholder: c(0x6C6C70),
            unread: c(0x0A84FF),
            avatar_top: c(0x8E929D),
            avatar_bottom: c(0x6B6E77),
            error: c(0xFF453A),
            popover: c(0x2C2C2E),
            popover_border: ca(0xFFFFFF1F),
            shadow: ca(0x00000080),
            selection: ca(0x0A84FF59),
            search_field: ca(0x7676803D),
        }
    }
}

/// Sizes and spacing, as Messages on macOS lays them out at its default
/// text size. Text sizes scale with [`Theme::text_scale`].
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub scale: f32,
}

impl Metrics {
    fn s(&self, v: f32) -> Pixels {
        px((v * self.scale).round())
    }
    pub fn message_text(&self) -> Pixels {
        self.s(14.)
    }
    pub fn message_line(&self) -> Pixels {
        self.s(18.)
    }
    pub fn jumbo_emoji(&self) -> Pixels {
        self.s(42.)
    }
    pub fn bubble_pad_x(&self) -> Pixels {
        self.s(12.)
    }
    pub fn bubble_pad_y(&self) -> Pixels {
        self.s(7.)
    }
    pub fn bubble_radius(&self) -> Pixels {
        self.s(16.)
    }
    pub fn caption(&self) -> Pixels {
        self.s(11.)
    }
    pub fn small(&self) -> Pixels {
        self.s(12.)
    }
    pub fn body(&self) -> Pixels {
        self.s(13.)
    }
    pub fn title(&self) -> Pixels {
        self.s(14.)
    }
}

/// Space between bubbles from the same sender, and between groups.
pub const GROUPED_GAP: f32 = 2.;
pub const GROUP_GAP: f32 = 10.;
/// How far a tail reaches past its bubble's edge.
pub const TAIL: f32 = 6.;
pub const SIDEBAR_WIDTH: f32 = 300.;
pub const AVATAR: f32 = 40.;

pub struct Theme {
    pub preference: Preference,
    pub system: Appearance,
    pub appearance: Appearance,
    pub colors: Colors,
    pub text_scale: f32,
}

impl Global for Theme {}

impl Theme {
    pub fn init(cx: &mut App) {
        let system = Appearance::from(cx.window_appearance());
        cx.set_global(Theme {
            preference: Preference::System,
            system,
            appearance: system,
            colors: colors_for(system),
            text_scale: 1.0,
        });
    }

    pub fn get(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    pub fn metrics(&self) -> Metrics {
        Metrics {
            scale: self.text_scale,
        }
    }

    pub fn set_preference(preference: Preference, cx: &mut App) {
        cx.update_global::<Theme, _>(|t, _| {
            t.preference = preference;
            t.resolve();
        });
        crate::kit::sync(cx);
        cx.refresh_windows();
    }

    pub fn set_text_scale(scale: f32, cx: &mut App) {
        cx.update_global::<Theme, _>(|t, _| t.text_scale = scale.clamp(0.8, 1.6));
        crate::kit::sync(cx);
        cx.refresh_windows();
    }

    /// Call from a window's appearance observer.
    pub fn sync_system(window: &Window, cx: &mut App) {
        let system = Appearance::from(window.appearance());
        let changed = cx.update_global::<Theme, _>(|t, _| {
            let before = t.appearance;
            t.system = system;
            t.resolve();
            before != t.appearance
        });
        if changed {
            crate::kit::sync(cx);
            cx.refresh_windows();
        }
    }

    fn resolve(&mut self) {
        self.appearance = match self.preference {
            Preference::System => self.system,
            Preference::Light => Appearance::Light,
            Preference::Dark => Appearance::Dark,
        };
        self.colors = colors_for(self.appearance);
    }
}

fn colors_for(a: Appearance) -> Colors {
    match a {
        Appearance::Light => Colors::light(),
        Appearance::Dark => Colors::dark(),
    }
}

/// `cx.theme()` and `cx.colors()` on anything that derefs to [`App`].
pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
    fn colors(&self) -> &Colors {
        &self.theme().colors
    }
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        Theme::get(self)
    }
}
