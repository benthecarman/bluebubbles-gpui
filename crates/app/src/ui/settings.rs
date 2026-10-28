//! Settings, over the window: keeping the app in the tray, notifications
//! (with a test), and appearance. Opened with Ctrl+, the sidebar's gear or
//! the tray menu.

use crate::shell::{self, Shell};
use bb_ui::{ActiveTheme, Icon, Preference, Theme, icon};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

actions!(settings, [CloseSettings]);

pub enum SettingsEvent {
    Close,
}

#[derive(Clone)]
enum Test {
    Sending,
    Sent(String),
    Failed(String),
}

pub struct Settings {
    focus: FocusHandle,
    test: Option<Test>,
}

impl EventEmitter<SettingsEvent> for Settings {}

impl Focusable for Settings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseSettings, Some("Settings"))]);
}

impl Settings {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            test: None,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Close);
    }

    fn send_test(&mut self, cx: &mut Context<Self>) {
        let Some(session) = shell::session(cx) else {
            self.test = Some(Test::Failed("Sign in first".into()));
            cx.notify();
            return;
        };
        self.test = Some(Test::Sending);
        cx.notify();
        let sent = session.read(cx).test_notification(cx);
        cx.spawn(async move |this, cx| {
            let result = sent.await;
            let _ = this.update(cx, |this, cx| {
                this.test = Some(match result {
                    Ok(server) => Test::Sent(server),
                    Err(e) => Test::Failed(e),
                });
                cx.notify();
            });
        })
        .detach();
    }
}

fn set_appearance(preference: Preference, cx: &mut App) {
    let value = match preference {
        Preference::System => None,
        Preference::Light => Some("light".to_owned()),
        Preference::Dark => Some("dark".to_owned()),
    };
    shell::update_config(cx, |c| c.appearance = value);
    Theme::set_preference(preference, cx);
    cx.refresh_windows();
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let config = cx.global::<Shell>().config.clone();
        let tray = shell::tray_shown(cx);
        let preference = cx.theme().preference;

        let caption = |text: SharedString| {
            div()
                .text_size(m.caption())
                .text_color(colors.text_secondary)
                .child(text)
        };
        let heading = |text: &'static str| {
            div()
                .pt(px(18.))
                .pb(px(6.))
                .text_size(m.caption())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.text_secondary)
                .child(text)
        };
        let row = |label: &'static str, control: AnyElement, note: Option<SharedString>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .py(px(10.))
                .border_b_1()
                .border_color(colors.separator)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(16.))
                        .child(div().text_size(m.body()).child(label))
                        .child(control),
                )
                .children(note.map(caption))
        };

        let keep = Switch::new("keep-running")
            .color(colors.accent)
            .checked(config.keep_running())
            .on_click(|on, _, cx| {
                let on = *on;
                shell::update_config(cx, |c| c.keep_running = Some(on));
                cx.refresh_windows();
            });
        let keep_note: SharedString = if !config.keep_running() {
            "Closing the window quits BlueBubbles, and new messages don't notify until you open \
             it again."
                .into()
        } else if tray {
            "Closing the window leaves BlueBubbles in the top bar, so new messages still notify. \
             Quit from its menu or with Ctrl+Q."
                .into()
        } else {
            "Your desktop isn't showing tray icons, so closing the window quits. On GNOME, turn \
             on the AppIndicator extension (Ubuntu has it on by default)."
                .into()
        };
        let notify = Switch::new("notifications")
            .color(colors.accent)
            .checked(config.notifications())
            .on_click(|on, _, cx| {
                let on = *on;
                shell::update_config(cx, |c| c.notifications = Some(on));
                cx.refresh_windows();
            });

        let test_button = div()
            .id("test-notification")
            .px(px(12.))
            .py(px(5.))
            .rounded(px(7.))
            .bg(colors.field)
            .border_1()
            .border_color(colors.field_border)
            .text_size(m.body())
            .cursor_pointer()
            .hover(|d| d.bg(colors.sidebar_hover))
            .child("Send Test Notification")
            .on_click(cx.listener(|this, _, _, cx| this.send_test(cx)));
        let test_note: Option<SharedString> = match &self.test {
            None => None,
            Some(Test::Sending) => Some("Sending…".into()),
            Some(Test::Sent(server)) => Some(
                format!(
                    "Sent to {server}. If nothing appeared, check that BlueBubbles is allowed in \
                     the desktop's notification settings and that Do Not Disturb is off."
                )
                .into(),
            ),
            Some(Test::Failed(e)) => Some(e.clone().into()),
        };

        let segment = |label: &'static str, value: Preference| {
            let selected = preference == value;
            div()
                .id(label)
                .px(px(12.))
                .py(px(4.))
                .rounded(px(6.))
                .text_size(m.body())
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(colors.window).shadow_sm().text_color(colors.text)
                })
                .when(!selected, |d| d.text_color(colors.text_secondary))
                .child(label)
                .on_click(move |_, _, cx| set_appearance(value, cx))
        };
        let appearance = div()
            .flex()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(8.))
            .bg(colors.search_field)
            .child(segment("System", Preference::System))
            .child(segment("Light", Preference::Light))
            .child(segment("Dark", Preference::Dark));

        let panel = div()
            .id("settings-panel")
            .w(px(500.))
            .max_h(window.viewport_size().height - px(112.))
            .overflow_y_scroll()
            .px(px(24.))
            .pt(px(16.))
            .pb(px(24.))
            .rounded(px(14.))
            .bg(colors.popover)
            .border_1()
            .border_color(colors.popover_border)
            .shadow_lg()
            // Clicks inside don't reach the backdrop, which closes.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(m.title())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .id("close-settings")
                            .size(px(24.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(colors.text.opacity(0.08))
                            .hover(|d| d.bg(colors.text.opacity(0.15)))
                            .child(
                                icon(Icon::WindowClose, px(14.)).text_color(colors.text_secondary),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            )
            .child(heading("GENERAL"))
            .child(row(
                "Keep running when the window is closed",
                keep.into_any_element(),
                Some(keep_note),
            ))
            .child(heading("NOTIFICATIONS"))
            .child(row(
                "Show notifications for new messages",
                notify.into_any_element(),
                None,
            ))
            .child(row(
                "Check notifications",
                test_button.into_any_element(),
                test_note,
            ))
            .child(heading("APPEARANCE"))
            .child(row("Theme", appearance.into_any_element(), None).border_b(px(0.)));

        div()
            .id("settings")
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CloseSettings, _, cx| this.close(cx)))
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            // Hung from the top like a sheet, so it grows downward when a
            // note appears instead of shifting under the pointer.
            .pt(px(72.))
            .bg(gpui_kit::black().opacity(0.25))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close(cx)),
            )
            .child(panel)
    }
}
