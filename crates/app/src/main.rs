use bb_ui::{ActiveTheme, Bubble, Tail, TypeStyled};
use gpui_kit::*;

struct Demo;

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.colors();
        let m = cx.theme().metrics();
        let bubble = |text: &str, color: Hsla, fg: Hsla, tail: Tail| {
            let row = div().flex().w_full().px(px(16.));
            let row = if tail == Tail::Left || color == colors.received {
                row.justify_start()
            } else {
                row.justify_end()
            };
            row.child(
                div().max_w(px(360.)).child(
                    Bubble::new(color, m.bubble_radius(), tail).child(
                        div()
                            .px(m.bubble_pad_x())
                            .py(m.bubble_pad_y())
                            .text_size(m.message_text())
                            .line_height(m.message_line())
                            .text_color(fg)
                            .child(text.to_string()),
                    ),
                ),
            )
        };
        div()
            .messages_type()
            .size_full()
            .flex()
            .bg(colors.window)
            .text_color(colors.text)
            .child(div().w(px(300.)).h_full().bg(colors.sidebar))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .gap(px(2.))
                    .pb(px(20.))
                    .child(bubble("Hey! Are we still on for tonight?", colors.received, colors.on_received, Tail::None))
                    .child(bubble("I can bring dessert 🍰", colors.received, colors.on_received, Tail::Left))
                    .child(div().h(px(8.)))
                    .child(bubble("Yes! 7 PM works", colors.imessage, colors.on_bubble, Tail::None))
                    .child(bubble("A longer message that should wrap onto a second line so we can see how the bubble grows with its text ❤️", colors.imessage, colors.on_bubble, Tail::Right))
                    .child(div().h(px(8.)))
                    .child(bubble("Sent as SMS", colors.sms, colors.on_bubble, Tail::Right)),
            )
    }
}

fn main() {
    application().run(|cx| {
        gpui_kit::init(cx);
        bb_ui::init(cx);
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("bluebubbles-gpui".into()),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| Demo),
        )
        .expect("open the window");
    });
}
