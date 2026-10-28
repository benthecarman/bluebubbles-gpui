//! Contact pictures: a photo, initials on Messages' grey gradient, or a
//! person glyph. Groups show two smaller circles.

use crate::assets::{Icon, icon};
use crate::theme::ActiveTheme;
use gpui_kit::*;
use std::sync::Arc;

#[derive(Clone)]
pub enum Face {
    Image(Arc<Image>),
    Initials(SharedString),
    Glyph,
}

#[derive(IntoElement)]
pub struct Avatar {
    faces: Vec<Face>,
    size: Pixels,
    ring: Option<Hsla>,
}

impl Avatar {
    pub fn new(face: Face, size: Pixels) -> Self {
        Self {
            faces: vec![face],
            size,
            ring: None,
        }
    }

    /// A group: the first two members' faces.
    pub fn group(faces: Vec<Face>, size: Pixels) -> Self {
        Self {
            faces,
            size,
            ring: None,
        }
    }

    /// The background behind a group avatar, for the ring between its
    /// circles (the sidebar's colour by default).
    pub fn ring(mut self, color: Hsla) -> Self {
        self.ring = Some(color);
        self
    }
}

fn circle(face: &Face, size: Pixels, cx: &App) -> AnyElement {
    let colors = cx.colors();
    let base = div()
        .size(size)
        .flex_none()
        .rounded_full()
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center();
    match face {
        // The image is rounded itself: overflow_hidden clips to a
        // rectangle, not the circle.
        Face::Image(image) => base
            .child(
                img(ImageSource::Image(image.clone()))
                    .size_full()
                    .rounded_full()
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element(),
        Face::Initials(initials) => base
            .bg(linear_gradient(
                180.,
                linear_color_stop(colors.avatar_top, 0.),
                linear_color_stop(colors.avatar_bottom, 1.),
            ))
            .text_color(white())
            .text_size(size * if size < px(30.) { 0.4 } else { 0.42 })
            .font_weight(FontWeight::MEDIUM)
            .child(initials.clone())
            .into_any_element(),
        Face::Glyph => base
            .bg(linear_gradient(
                180.,
                linear_color_stop(colors.avatar_top, 0.),
                linear_color_stop(colors.avatar_bottom, 1.),
            ))
            .items_end()
            .child(icon(Icon::Person, size * 0.78).text_color(white()))
            .into_any_element(),
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = self.size;
        match self.faces.as_slice() {
            [] => circle(&Face::Glyph, size, cx),
            [one] => circle(one, size, cx),
            [a, b, ..] => {
                // Two overlapping circles, the front one ringed in the
                // background colour as Messages does.
                let small = size * 0.62;
                let ring = self.ring.unwrap_or(cx.colors().sidebar);
                div()
                    .size(size)
                    .flex_none()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .child(circle(b, small, cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .rounded_full()
                            .border_2()
                            .border_color(ring)
                            .child(circle(a, small, cx)),
                    )
                    .into_any_element()
            }
        }
    }
}
