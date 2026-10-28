//! Message bubbles: a rounded rectangle, and for the last bubble of a run
//! from one sender, the Messages tail at its bottom outer corner.

use gpui_kit::{
    AnyElement, App, Bounds, Hsla, IntoElement, ParentElement, Path, PathBuilder, Pixels, Point,
    RenderOnce, Styled, Window, canvas, div, point, px,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    None,
    /// Received messages: tail at the bottom left.
    Left,
    /// Sent messages: tail at the bottom right.
    Right,
}

/// Approximates a quarter circle with a cubic Bézier.
const KAPPA: f32 = 0.552_284_8;

/// The bubble outline for `bounds`, the box the content sits in. The tail
/// is drawn outside it, reaching `radius * 4/14` past the side it is on
/// (under [`crate::theme::TAIL`] at the default radius).
pub fn bubble_path(bounds: Bounds<Pixels>, radius: Pixels, tail: Tail) -> Option<Path<Pixels>> {
    let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w <= 0. || h <= 0. {
        return None;
    }
    let (x1, y1) = (x0 + w, y0 + h);
    let r = f32::from(radius).min(h / 2.).min(w / 2.);
    let k = r * KAPPA;
    // Build the right-tailed shape, mirrored for a left tail.
    let mirror = tail == Tail::Left;
    let p = |x: f32, y: f32| -> Point<Pixels> {
        let x = if mirror { x0 + x1 - x } else { x };
        point(px(x), px(y))
    };
    let mut b = PathBuilder::fill();
    b.move_to(p(x0 + r, y0));
    b.line_to(p(x1 - r, y0));
    b.cubic_bezier_to(p(x1, y0 + r), p(x1 - r + k, y0), p(x1, y0 + r - k));
    if tail == Tail::None {
        b.line_to(p(x1, y1 - r));
        b.cubic_bezier_to(p(x1 - r, y1), p(x1, y1 - r + k), p(x1 - r + k, y1));
    } else {
        // The Messages hook: the outer edge runs down and sweeps out to a
        // tip below the bubble's corner; the underside rises into a small
        // notch and eases back into the bottom edge. Proportions scale
        // with the corner radius.
        let s = r / 14.;
        b.line_to(p(x1, y1 - 11. * s));
        b.cubic_bezier_to(p(x1 + 4. * s, y1), p(x1, y1 - s), p(x1 + 4. * s, y1));
        b.cubic_bezier_to(
            p(x1 - 7. * s, y1 - 4. * s),
            p(x1, y1 + 0.5 * s),
            p(x1 - 4. * s, y1 - s),
        );
        b.cubic_bezier_to(
            p(x1 - 21. * s, y1),
            p(x1 - 12. * s, y1),
            p(x1 - 16. * s, y1),
        );
    }
    b.line_to(p(x0 + r, y1));
    b.cubic_bezier_to(p(x0, y1 - r), p(x0 + r - k, y1), p(x0, y1 - r + k));
    b.line_to(p(x0, y0 + r));
    b.cubic_bezier_to(p(x0 + r, y0), p(x0, y0 + r - k), p(x0 + r - k, y0));
    b.close();
    b.build().ok()
}

/// A bubble filled with `color` around its children.
#[derive(IntoElement)]
pub struct Bubble {
    color: Hsla,
    radius: Pixels,
    tail: Tail,
    children: Vec<AnyElement>,
}

impl Bubble {
    pub fn new(color: Hsla, radius: Pixels, tail: Tail) -> Self {
        Self {
            color,
            radius,
            tail,
            children: Vec::new(),
        }
    }
}

impl ParentElement for Bubble {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Bubble {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Bubble {
            color,
            radius,
            tail,
            children,
        } = self;
        div()
            .relative()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| {
                        if let Some(path) = bubble_path(bounds, radius, tail) {
                            window.paint_path(path, color);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children(children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::size;

    #[test]
    fn builds_every_shape_and_skips_empty_bounds() {
        let bounds = Bounds::new(point(px(10.), px(10.)), size(px(120.), px(32.)));
        for tail in [Tail::None, Tail::Left, Tail::Right] {
            assert!(bubble_path(bounds, px(16.), tail).is_some());
        }
        let empty = Bounds::new(point(px(0.), px(0.)), size(px(0.), px(20.)));
        assert!(bubble_path(empty, px(16.), Tail::Right).is_none());
    }
}
