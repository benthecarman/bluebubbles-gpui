//! The window's own title bar: Linux compositors (GNOME on Wayland in
//! particular) draw none for us, so the sidebar's top edge is the title
//! bar and carries Messages' three window buttons.

use bb_ui::{Icon, icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// Which buttons the session supports: Hyprland has no minimize, and
/// Omarchy refuses maximize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub minimize: bool,
    pub maximize: bool,
}

impl Policy {
    pub fn current() -> Self {
        static POLICY: std::sync::LazyLock<Policy> = std::sync::LazyLock::new(|| {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| std::env::var("XDG_SESSION_DESKTOP").ok())
                .unwrap_or_default();
            Policy::for_session(&desktop, &std::env::var("OMARCHY_PATH").unwrap_or_default())
        });
        *POLICY
    }

    fn for_session(desktop: &str, omarchy_path: &str) -> Self {
        let hyprland = desktop
            .split(':')
            .any(|name| name.trim().eq_ignore_ascii_case("Hyprland"));
        Self {
            minimize: !hyprland,
            maximize: !hyprland || omarchy_path.trim().is_empty(),
        }
    }
}

const RED: u32 = 0xFF5F57;
const YELLOW: u32 = 0xFEBC2E;
const GREEN: u32 = 0x28C840;
const SIZE: f32 = 12.;

/// Close, minimize and zoom, as coloured circles whose glyphs show while
/// the pointer is over the group.
pub fn traffic_lights(window: &Window) -> impl IntoElement {
    let policy = Policy::current();
    let supported = window.window_controls();
    let active = window.is_window_active();
    let light = |id: &'static str, color: u32, glyph: Icon| {
        let fill: Hsla = if active {
            rgb(color).into()
        } else {
            hsla(0., 0., 0.5, 0.35)
        };
        div()
            .id(id)
            .size(px(SIZE))
            .rounded_full()
            .bg(fill)
            .flex()
            .items_center()
            .justify_center()
            .child(
                icon(glyph, px(8.))
                    .text_color(hsla(0., 0., 0., 0.55))
                    .invisible()
                    .group_hover("traffic-lights", |s| s.visible()),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };
    div()
        .group("traffic-lights")
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            light("window-close", RED, Icon::Close).on_click(|_, window, _| window.remove_window()),
        )
        .when(policy.minimize && supported.minimize, |row| {
            row.child(
                light("window-minimize", YELLOW, Icon::Minimize)
                    .on_click(|_, window, _| window.minimize_window()),
            )
        })
        .when(policy.maximize && supported.maximize, |row| {
            row.child(
                light("window-zoom", GREEN, Icon::Maximize)
                    .on_click(|_, window, _| window.zoom_window()),
            )
        })
}

/// Makes an element a title bar: drag to move, double-click to maximize,
/// right-click for the window menu.
pub fn title_bar(element: Stateful<Div>) -> Stateful<Div> {
    element
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .on_mouse_down(MouseButton::Right, |event, window, _| {
            if window.window_controls().window_menu {
                window.show_window_menu(event.position);
            }
        })
}

/// A control inside a title bar: keeps its clicks from moving the window.
pub fn not_title_bar<E: InteractiveElement>(element: E) -> E {
    element
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

#[cfg(test)]
mod tests {
    use super::Policy;

    #[test]
    fn hyprland_has_no_minimize() {
        assert_eq!(
            Policy::for_session("Hyprland", ""),
            Policy {
                minimize: false,
                maximize: true
            }
        );
        assert_eq!(
            Policy::for_session("ubuntu:GNOME", "/x"),
            Policy {
                minimize: true,
                maximize: true
            }
        );
    }
}
