//! Text you can select with the mouse and copy. gpui paints text but owns no
//! selection outside Zed's editor, so a log line, a chat message, or an API
//! response cannot be copied. This is the missing piece for read-only text.
//!
//! One [`Selection`] entity per view, from [`use_selection`]; any number of
//! [`selectable_text`] elements share it, so only one holds a selection at a
//! time. Drag to select, double-click for a word. Bind ⌘C to
//! `selection.read(cx).text()`.

use gpui::{
    App, Bounds, DispatchPhase, Element, ElementId, Entity, GlobalElementId, HighlightStyle, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, SharedString, StyledText, Window, rgb,
};
use std::ops::Range;
use std::panic::Location;

/// Which element holds the selection, and where in its text.
pub struct Selection {
    owner: Option<ElementId>,
    anchor: usize,
    head: usize,
    dragging: bool,
    /// The selected text, kept here so a copy needs no access to the element.
    text: String,
    highlight: HighlightStyle,
}

impl Selection {
    /// The selected text, if any. Empty selections read as `None`.
    pub fn text(&self) -> Option<&str> {
        (!self.text.is_empty()).then_some(self.text.as_str())
    }

    /// Drop the selection.
    pub fn clear(&mut self) {
        self.owner = None;
        self.text.clear();
    }

    /// Byte range within `owner`'s text, normalized.
    fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn range_in(&self, id: &ElementId) -> Option<Range<usize>> {
        (self.owner.as_ref() == Some(id) && self.anchor != self.head).then(|| self.range())
    }
}

/// The shared selection for this view, identified by the caller's source
/// location. Call only during render.
#[track_caller]
pub fn use_selection(window: &mut Window, cx: &mut App) -> Entity<Selection> {
    use_keyed_selection(ElementId::CodeLocation(*Location::caller()), window, cx)
}

/// [`use_selection`] with an explicit id.
pub fn use_keyed_selection(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<Selection> {
    window.use_keyed_state(id, cx, |_, _| Selection {
        owner: None,
        anchor: 0,
        head: 0,
        dragging: false,
        text: String::new(),
        highlight: HighlightStyle {
            background_color: Some(rgb(0x3a5f9e).into()),
            ..Default::default()
        },
    })
}

/// A line of text that takes part in `selection`. `id` must be unique among
/// the elements sharing one selection; in a list, use the row index.
pub fn selectable_text(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    selection: &Entity<Selection>,
) -> SelectableText {
    SelectableText {
        id: id.into(),
        text: text.into(),
        highlights: Vec::new(),
        selection: selection.clone(),
    }
}

pub struct SelectableText {
    id: ElementId,
    text: SharedString,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    selection: Entity<Selection>,
}

impl SelectableText {
    /// Extra styling by byte range (search matches, say), under the selection.
    pub fn with_highlights(mut self, highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>) -> Self {
        self.highlights.extend(highlights);
        self
    }
}

/// Word boundaries around `at`, for double-click.
fn word_at(text: &str, at: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start = text[..at].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(at, |(i, _)| i);
    let end = text[at..].char_indices().take_while(|(_, c)| is_word(*c)).last().map_or(at, |(i, c)| at + i + c.len_utf8());
    start..end
}

impl IntoElement for SelectableText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = StyledText;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, StyledText) {
        let mut highlights = self.highlights.clone();
        let selection = self.selection.read(cx);
        if let Some(range) = selection.range_in(&self.id) {
            highlights.push((range, selection.highlight));
        }
        let mut styled = StyledText::new(self.text.clone()).with_highlights(highlights);
        let (layout, ()) = styled.request_layout(None, inspector_id, window, cx);
        (layout, styled)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        styled: &mut StyledText,
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        styled.prepaint(None, inspector_id, bounds, &mut (), window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        styled: &mut StyledText,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        styled.paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);
        let layout = styled.layout().clone();
        let view = window.current_view();
        let id = self.id.clone();
        let text = self.text.clone();
        let selection = self.selection.clone();

        let index_at = move |p| layout.index_for_position(p).unwrap_or_else(|i| i);

        window.on_mouse_event({
            let hitbox = hitbox.clone();
            let (id, text, selection, index_at) = (id.clone(), text.clone(), selection.clone(), index_at.clone());
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                let at = index_at(event.position);
                let (anchor, head) = if event.click_count >= 2 {
                    let w = word_at(&text, at);
                    (w.start, w.end)
                } else {
                    (at, at)
                };
                selection.update(cx, |s, _| {
                    s.owner = Some(id.clone());
                    s.anchor = anchor;
                    s.head = head;
                    s.dragging = event.click_count < 2;
                    s.text = text[s.range()].to_string();
                });
                cx.notify(view);
            }
        });

        window.on_mouse_event({
            let (id, text, selection, index_at) = (id.clone(), text.clone(), selection.clone(), index_at.clone());
            move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let s = selection.read(cx);
                if !s.dragging || s.owner.as_ref() != Some(&id) {
                    return;
                }
                let head = index_at(event.position);
                if head == s.head {
                    return;
                }
                selection.update(cx, |s, _| {
                    s.head = head;
                    s.text = text[s.range()].to_string();
                });
                cx.notify(view);
            }
        });

        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && selection.read(cx).dragging {
                selection.update(cx, |s, _| s.dragging = false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::word_at;

    #[test]
    fn word_at_finds_boundaries() {
        assert_eq!(word_at("INFO tick_1 done", 7), 5..11);
        assert_eq!(word_at("INFO tick_1 done", 0), 0..4);
        assert_eq!(word_at("INFO tick_1 done", 4), 0..4);
        assert_eq!(word_at("a  b", 2), 2..2);
    }
}
