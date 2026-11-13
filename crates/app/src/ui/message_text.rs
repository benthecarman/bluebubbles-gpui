//! A message's text, selectable with the mouse like text in Messages:
//! drag across it, double-click a word, and copy with Ctrl+C or the
//! bubble's Copy. Each bubble is its own selection. Links are underlined
//! and open on a click that doesn't select anything.
//!
//! This is `gpui_base::SelectableText` (which joins the window's text
//! selection, set up by the window's `Root`) with highlights and links,
//! which it doesn't have.

use gpui_kit::base::{
    TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun,
};
use gpui_kit::*;
use std::ops::Range;

pub struct MessageText {
    id: ElementId,
    text: SharedString,
    styled: StyledText,
    links: Vec<(Range<usize>, String)>,
    selection_color: Hsla,
}

impl MessageText {
    pub fn new(id: impl Into<ElementId>, text: SharedString, selection_color: Hsla) -> Self {
        Self {
            id: id.into(),
            styled: StyledText::new(text.clone()),
            text,
            links: Vec::new(),
            selection_color,
        }
    }

    /// Underlines `links` (byte ranges and the URL each opens) in `color`.
    pub fn links(mut self, links: Vec<(Range<usize>, String)>, color: Hsla) -> Self {
        let underline = HighlightStyle {
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(color),
                wavy: false,
            }),
            ..Default::default()
        };
        self.styled = StyledText::new(self.text.clone())
            .with_highlights(links.iter().map(|(r, _)| (r.clone(), underline)));
        self.links = links;
        self
    }

    /// The link under `position`, if any.
    fn link_at(
        layout: &TextLayout,
        links: &[(Range<usize>, String)],
        position: Point<Pixels>,
    ) -> Option<String> {
        let ix = layout.index_for_position(position).ok()?;
        links
            .iter()
            .find(|(r, _)| r.contains(&ix))
            .map(|(_, url)| url.clone())
    }
}

impl IntoElement for MessageText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MessageText {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // The selection belongs to the bubble, kept across frames.
        let handle = window.with_element_state(
            global_id.expect("MessageText has an id"),
            |kept: Option<TextSelectionHandle>, _| {
                let handle =
                    kept.unwrap_or_else(|| TextSelectionHandle::new(self.text.clone(), cx));
                (handle.clone(), handle)
            },
        );
        let (layout_id, ()) = self
            .styled
            .request_layout(global_id, inspector_id, window, cx);
        (layout_id, handle)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let before = TextSelection::selected_text(window, cx);
        let projection = handle.update_runs(
            &[TextSelectionRun::new(
                self.text.clone(),
                layout.clone(),
                bounds,
            )],
            cx,
        );
        if before != TextSelection::selected_text(window, cx) {
            window.refresh();
        }
        for range in projection.ranges().iter().flatten().cloned() {
            paint_selection(&layout, range, self.selection_color, window);
        }
        self.styled.paint(
            global_id,
            inspector_id,
            bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );

        let over_link = hitbox.is_hovered(window)
            && Self::link_at(&layout, &self.links, window.mouse_position()).is_some();
        window.set_cursor_style(
            if over_link {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            },
            hitbox,
        );
        if self.links.is_empty() {
            return;
        }
        // A click opens the link under it; a drag selects instead.
        let links = self.links.clone();
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !hitbox.is_hovered(window)
                || TextSelection::has_selection(window, cx)
            {
                return;
            }
            if let Some(url) = Self::link_at(&layout, &links, event.position) {
                cx.open_url(&url);
            }
        });
    }
}

/// The selection's highlight over `range`, line by line.
fn paint_selection(layout: &TextLayout, range: Range<usize>, color: Hsla, window: &mut Window) {
    let (Some(start), Some(end)) = (
        layout.position_for_index(range.start),
        layout.position_for_index(range.end),
    ) else {
        return;
    };
    let bounds = layout.bounds();
    let line = layout.line_height();
    let mut quads = Vec::new();
    if start.y == end.y {
        quads.push(Bounds::from_corners(start, point(end.x, end.y + line)));
    } else {
        quads.push(Bounds::from_corners(
            start,
            point(bounds.right(), start.y + line),
        ));
        if end.y > start.y + line {
            quads.push(Bounds::from_corners(
                point(bounds.left(), start.y + line),
                point(bounds.right(), end.y),
            ));
        }
        quads.push(Bounds::from_corners(
            point(bounds.left(), end.y),
            point(end.x, end.y + line),
        ));
    }
    for q in quads {
        window.paint_quad(fill(q, color));
    }
}
