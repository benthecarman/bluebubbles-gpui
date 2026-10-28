//! The window's own title bar: Linux compositors (GNOME on Wayland in
//! particular) draw none for us, so the top edges of the sidebar and the
//! conversation are the title bar and carry GNOME's window buttons.

use bb_ui::{ActiveTheme, Icon, icon};
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

/// Which end of the title bar a group of buttons sits at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// The buttons for one side, in the order GNOME's `button-layout` setting
/// gives (minimize, maximize and close on the right unless changed), drawn
/// as libadwaita draws them. `None` when that side has none.
pub fn buttons(side: Side, window: &Window, cx: &App) -> Option<impl IntoElement + use<>> {
    let layout = cx
        .button_layout()
        .unwrap_or_else(WindowButtonLayout::linux_default);
    let slots = match side {
        Side::Left => layout.left,
        Side::Right => layout.right,
    };
    let policy = Policy::current();
    let supported = window.window_controls();
    let shown: Vec<WindowButton> = slots
        .into_iter()
        .flatten()
        .filter(|b| match b {
            WindowButton::Minimize => policy.minimize && supported.minimize,
            WindowButton::Maximize => policy.maximize && supported.maximize,
            WindowButton::Close => true,
        })
        .collect();
    if shown.is_empty() {
        return None;
    }
    let colors = *cx.colors();
    let fg = if window.is_window_active() {
        colors.text
    } else {
        colors.text.opacity(0.5)
    };
    let maximized = window.is_maximized();
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .children(shown.into_iter().map(move |button| {
                let glyph = match button {
                    WindowButton::Minimize => Icon::WindowMinimize,
                    WindowButton::Maximize if maximized => Icon::WindowRestore,
                    WindowButton::Maximize => Icon::WindowMaximize,
                    WindowButton::Close => Icon::WindowClose,
                };
                not_title_bar(
                    div()
                        .id(button.id())
                        .size(px(24.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(colors.text.opacity(0.1))
                        .hover(|s| s.bg(colors.text.opacity(0.15)))
                        .active(|s| s.bg(colors.text.opacity(0.3))),
                )
                .child(icon(glyph, px(16.)).text_color(fg))
                .on_click(move |_, window, _| match button {
                    WindowButton::Minimize => window.minimize_window(),
                    WindowButton::Maximize => window.zoom_window(),
                    WindowButton::Close => window.remove_window(),
                })
            })),
    )
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
