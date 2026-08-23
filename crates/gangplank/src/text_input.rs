//! A single-line text field. gpui-ce paints text and routes key events but
//! ships no input: every app writes its own cursor, selection, IME plumbing
//! and blink timer. This hook is that field, once.
//!
//! [`use_text_input`] hands back the [`TextInput`] entity; [`text_input`]
//! renders it against a focus handle the caller owns. Printable keys,
//! Backspace, Delete, ←/→, Home/End (and ⌘←/⌘→), Shift-extend, ⌘A, ⌘C, ⌘X,
//! ⌘V are handled here. Enter and Esc are the caller's: set `on_submit` and
//! `on_cancel` on the element, or let them propagate to [`crate::use_keyboard`].
//!
//! Indices in the public API are char indices. Byte offsets stay inside the
//! layout code; UTF-16 offsets stay inside the IME code.

use crate::clipboard::Clipboard;
use gpui::{
    App, AppContext, Bounds, ContentMask, Context, DispatchPhase, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, GlobalElementId, HighlightStyle, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, InteractiveElement, IntoElement, KeyDownEvent,
    LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, SharedString,
    StyledText, Task, TextLayout, UTF16Selection, UnderlineStyle, Window, div, fill, linear_color_stop,
    linear_gradient, px, rgb, size,
};
use std::ops::Range;
use std::panic::Location;
use std::rc::Rc;
use std::time::Duration;

const BLINK: Duration = Duration::from_millis(500);

/// The entity `use_text_input` hands back. Indices are chars.
pub struct TextInput {
    text: String,
    cursor: usize,
    /// Other end of the selection, when one exists. `cursor` is the head.
    anchor: Option<usize>,
    /// IME composition in progress, underlined, replaced on commit.
    marked: Option<Range<usize>>,
    clipboard: Entity<Clipboard>,
    blink: Option<Task<()>>,
    cursor_on: bool,
    dragging: bool,
    /// Last painted layout, for IME candidate-window placement and clicks.
    layout: Option<TextLayout>,
}

impl TextInput {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Cursor position in chars.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Selected char range, normalized; `None` when empty.
    pub fn selection(&self) -> Option<Range<usize>> {
        let a = self.anchor?;
        (a != self.cursor).then(|| a.min(self.cursor)..a.max(self.cursor))
    }

    /// False during the off half of a blink. Resets to true on any edit.
    pub fn cursor_visible(&self) -> bool {
        self.cursor_on
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|r| self.slice(r))
    }

    /// Replace the contents and put the cursor at the end.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = text.into();
        self.cursor = self.len();
        self.anchor = None;
        self.marked = None;
        self.touched(cx);
    }

    /// Insert at the cursor, replacing the selection if one exists.
    pub fn insert(&mut self, s: &str, cx: &mut Context<Self>) {
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        self.replace(range, s);
        self.touched(cx);
    }

    pub fn backspace(&mut self, cx: &mut Context<Self>) {
        match self.selection() {
            Some(r) => self.replace(r, ""),
            None if self.cursor > 0 => self.replace(self.cursor - 1..self.cursor, ""),
            None => {}
        }
        self.touched(cx);
    }

    pub fn delete(&mut self, cx: &mut Context<Self>) {
        match self.selection() {
            Some(r) => self.replace(r, ""),
            None if self.cursor < self.len() => self.replace(self.cursor..self.cursor + 1, ""),
            None => {}
        }
        self.touched(cx);
    }

    /// Move the cursor to `to`. With `extend`, the selection grows from where
    /// it was; without, a selection collapses.
    pub fn move_to(&mut self, to: usize, extend: bool, cx: &mut Context<Self>) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = to.min(self.len());
        self.touched(cx);
    }

    pub fn left(&mut self, extend: bool, cx: &mut Context<Self>) {
        match (extend, self.selection()) {
            (false, Some(r)) => self.move_to(r.start, false, cx),
            _ => self.move_to(self.cursor.saturating_sub(1), extend, cx),
        }
    }

    pub fn right(&mut self, extend: bool, cx: &mut Context<Self>) {
        match (extend, self.selection()) {
            (false, Some(r)) => self.move_to(r.end, false, cx),
            _ => self.move_to(self.cursor + 1, extend, cx),
        }
    }

    pub fn home(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.move_to(0, extend, cx);
    }

    pub fn end(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.move_to(self.len(), extend, cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.anchor = Some(0);
        self.cursor = self.len();
        self.touched(cx);
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = self.selected_text() {
            self.clipboard.update(cx, |c, cx| c.copy(s, cx));
        }
    }

    pub fn cut(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = self.selection() {
            self.copy(cx);
            self.replace(r, "");
            self.touched(cx);
        }
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = self.clipboard.read(cx).read(cx) {
            self.insert(&s, cx);
        }
    }

    /// Apply one key event. `true` means the field consumed it; Enter, Esc
    /// and printable keys return `false` (text arrives through the IME path).
    pub fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let k = &event.keystroke;
        let (cmd, shift) = (k.modifiers.platform, k.modifiers.shift);
        if k.modifiers.control || k.modifiers.alt {
            return false;
        }
        match (cmd, k.key.as_str()) {
            (false, "backspace") => self.backspace(cx),
            (false, "delete") => self.delete(cx),
            (false, "left") => self.left(shift, cx),
            (false, "right") => self.right(shift, cx),
            (_, "home") | (true, "left") => self.home(shift, cx),
            (_, "end") | (true, "right") => self.end(shift, cx),
            (true, "a") => self.select_all(cx),
            (true, "c") => self.copy(cx),
            (true, "x") => self.cut(cx),
            (true, "v") => self.paste(cx),
            _ => return false,
        }
        true
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn slice(&self, r: Range<usize>) -> String {
        self.text.chars().skip(r.start).take(r.end - r.start).collect()
    }

    fn byte(&self, ch: usize) -> usize {
        self.text.char_indices().nth(ch).map_or(self.text.len(), |(b, _)| b)
    }

    fn char_at_byte(&self, b: usize) -> usize {
        self.text[..b].chars().count()
    }

    fn utf16(&self, ch: usize) -> usize {
        self.text.chars().take(ch).map(char::len_utf16).sum()
    }

    fn char_at_utf16(&self, u: usize) -> usize {
        let mut seen = 0;
        for (i, c) in self.text.chars().enumerate() {
            if seen >= u {
                return i;
            }
            seen += c.len_utf16();
        }
        self.len()
    }

    /// Splice `s` over the char range `r`; cursor lands after `s`. Newlines
    /// never enter a single-line field.
    fn replace(&mut self, r: Range<usize>, s: &str) {
        let s: String = s.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        let (start, end) = (self.byte(r.start), self.byte(r.end));
        self.text.replace_range(start..end, &s);
        self.cursor = r.start + s.chars().count();
        self.anchor = None;
        self.marked = None;
    }

    /// After any edit or move: show the cursor, restart the blink, redraw.
    fn touched(&mut self, cx: &mut Context<Self>) {
        self.cursor_on = true;
        self.start_blink(cx);
        cx.notify();
    }

    fn start_blink(&mut self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        self.blink = Some(cx.spawn(async move |this, cx| {
            loop {
                executor.timer(BLINK).await;
                let alive = this.update(cx, |this, cx| {
                    this.cursor_on = !this.cursor_on;
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    fn stop_blink(&mut self) {
        self.blink = None;
        self.cursor_on = true;
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.char_at_utf16(range.start)..self.char_at_utf16(range.end);
        *adjusted = Some(self.utf16(r.start)..self.utf16(r.end));
        Some(self.slice(r))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let r = self.selection().unwrap_or(self.cursor..self.cursor);
        Some(UTF16Selection {
            range: self.utf16(r.start)..self.utf16(r.end),
            reversed: self.anchor.is_some_and(|a| a > self.cursor),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let m = self.marked.clone()?;
        Some(self.utf16(m.start)..self.utf16(m.end))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let r = range
            .map(|r| self.char_at_utf16(r.start)..self.char_at_utf16(r.end))
            .or_else(|| self.marked.clone())
            .or_else(|| self.selection())
            .unwrap_or(self.cursor..self.cursor);
        self.replace(r, text);
        self.touched(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range
            .map(|r| self.char_at_utf16(r.start)..self.char_at_utf16(r.end))
            .or_else(|| self.marked.clone())
            .or_else(|| self.selection())
            .unwrap_or(self.cursor..self.cursor);
        self.replace(r.clone(), text);
        let end = self.cursor;
        self.marked = Some(r.start..end);
        if let Some(s) = selected {
            // `selected` is relative to the marked text, in UTF-16.
            let marked: String = self.slice(r.start..end);
            let to_char = |u: usize| {
                let mut seen = 0;
                for (i, c) in marked.chars().enumerate() {
                    if seen >= u {
                        return i;
                    }
                    seen += c.len_utf16();
                }
                marked.chars().count()
            };
            self.anchor = (s.start != s.end).then(|| r.start + to_char(s.start));
            self.cursor = r.start + to_char(s.end);
        }
        self.touched(cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let start = self.byte(self.char_at_utf16(range.start));
        let end = self.byte(self.char_at_utf16(range.end));
        let origin = layout.position_for_index(start).unwrap_or(element_bounds.origin);
        let right = layout.position_for_index(end).map_or(origin.x, |p| p.x);
        Some(Bounds::new(origin, size(right - origin.x, layout.line_height())))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let b = layout.index_for_position(p).unwrap_or_else(|i| i);
        Some(self.utf16(self.char_at_byte(b)))
    }
}

/// The field's state, identified by the caller's source location. Call only
/// during render; render it with [`text_input`].
#[track_caller]
pub fn use_text_input(window: &mut Window, cx: &mut App) -> Entity<TextInput> {
    use_keyed_text_input(ElementId::CodeLocation(*Location::caller()), window, cx)
}

/// [`use_text_input`] with an explicit id.
pub fn use_keyed_text_input(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<TextInput> {
    window.use_keyed_state(id, cx, |_, cx| TextInput {
        text: String::new(),
        cursor: 0,
        anchor: None,
        marked: None,
        clipboard: cx.new(|_| Clipboard::new(Duration::from_secs(1))),
        blink: None,
        cursor_on: true,
        dragging: false,
        layout: None,
    })
}

type Submit = Rc<dyn Fn(&str, &mut Window, &mut App)>;
type Cancel = Rc<dyn Fn(&mut Window, &mut App)>;

/// The element for one field. Font and text colour come from the enclosing
/// text style; wrap it in a `div` for padding, border and width.
pub struct TextInputElement {
    state: Entity<TextInput>,
    focus: FocusHandle,
    placeholder: Option<SharedString>,
    placeholder_color: Hsla,
    selection_color: Hsla,
    cursor_color: Option<Hsla>,
    /// Background to fade clipped text into. None: a hard clip.
    fade_color: Option<Hsla>,
    on_submit: Option<Submit>,
    on_cancel: Option<Cancel>,
}

/// Render `state` as a focusable field. Clicking focuses `focus`; keys reach
/// the field only while it is focused.
pub fn text_input(state: &Entity<TextInput>, focus: &FocusHandle) -> TextInputElement {
    TextInputElement {
        state: state.clone(),
        focus: focus.clone(),
        placeholder: None,
        placeholder_color: rgb(0x808080).into(),
        selection_color: rgb(0x3a5f9e).into(),
        cursor_color: None,
        fade_color: None,
        on_submit: None,
        on_cancel: None,
    }
}

impl TextInputElement {
    /// Shown, muted, while the field is empty.
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn placeholder_color(mut self, color: impl Into<Hsla>) -> Self {
        self.placeholder_color = color.into();
        self
    }

    pub fn selection_color(mut self, color: impl Into<Hsla>) -> Self {
        self.selection_color = color.into();
        self
    }

    /// Defaults to the text colour.
    pub fn cursor_color(mut self, color: impl Into<Hsla>) -> Self {
        self.cursor_color = Some(color.into());
        self
    }

    /// Text that scrolls under either edge fades into this colour, the
    /// field's background, instead of stopping at a hard clip.
    pub fn fade_color(mut self, color: impl Into<Hsla>) -> Self {
        self.fade_color = Some(color.into());
        self
    }

    /// Enter. Without this, Enter propagates to the parent.
    pub fn on_submit(mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }

    /// Esc. Without this, Esc propagates to the parent.
    pub fn on_cancel(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(f));
        self
    }
}

impl IntoElement for TextInputElement {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        let state = self.state.clone();
        let (on_submit, on_cancel) = (self.on_submit.clone(), self.on_cancel.clone());
        div()
            .track_focus(&self.focus)
            .on_key_down(move |event, window, cx| {
                let key = event.keystroke.key.as_str();
                let plain = !event.keystroke.modifiers.modified();
                match key {
                    "enter" if plain && on_submit.is_some() => {
                        let text = state.read(cx).text.clone();
                        on_submit.as_ref().unwrap()(&text, window, cx);
                    }
                    "escape" if plain && on_cancel.is_some() => on_cancel.as_ref().unwrap()(window, cx),
                    _ => {
                        if !state.update(cx, |t, cx| t.handle_key(event, cx)) {
                            return;
                        }
                    }
                }
                cx.stop_propagation();
            })
            .child(Field {
                state: self.state,
                focus: self.focus,
                placeholder: self.placeholder,
                placeholder_color: self.placeholder_color,
                selection_color: self.selection_color,
                cursor_color: self.cursor_color,
                fade_color: self.fade_color,
            })
    }
}

/// The painted part: text, selection, marked-text underline, cursor.
struct Field {
    state: Entity<TextInput>,
    focus: FocusHandle,
    placeholder: Option<SharedString>,
    placeholder_color: Hsla,
    selection_color: Hsla,
    cursor_color: Option<Hsla>,
    fade_color: Option<Hsla>,
}

/// Width of the fade over a clipped edge.
const FADE: Pixels = px(14.);

impl IntoElement for Field {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Field {
    type RequestLayoutState = StyledText;
    /// The hitbox, and the bounds the text was laid out at: shifted left
    /// when the caret would otherwise fall past the field's right edge.
    type PrepaintState = (Hitbox, Bounds<Pixels>);

    fn id(&self) -> Option<ElementId> {
        None
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
        let t = self.state.read(cx);
        let mut highlights = Vec::new();
        let mut text: SharedString = t.text.clone().into();
        if t.text.is_empty() {
            if let Some(p) = &self.placeholder {
                text = p.clone();
                highlights.push((0..p.len(), HighlightStyle { color: Some(self.placeholder_color), ..Default::default() }));
            }
        } else {
            if let Some(r) = t.selection() {
                highlights.push((
                    t.byte(r.start)..t.byte(r.end),
                    HighlightStyle { background_color: Some(self.selection_color), ..Default::default() },
                ));
            }
            if let Some(m) = &t.marked {
                highlights.push((
                    t.byte(m.start)..t.byte(m.end),
                    HighlightStyle {
                        underline: Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                        ..Default::default()
                    },
                ));
            }
        }
        let mut styled = StyledText::new(text).with_highlights(highlights);
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
    ) -> (Hitbox, Bounds<Pixels>) {
        styled.prepaint(None, inspector_id, bounds, &mut (), window, cx);
        // Text longer than the field scrolls under its left edge so the
        // caret stays in view, like every native single-line field.
        let t = self.state.read(cx);
        let caret = styled.layout().position_for_index(t.byte(t.cursor)).map_or(px(0.), |p| p.x - bounds.origin.x);
        let overflow = (caret + px(1.) - bounds.size.width).max(px(0.));
        let mut text_bounds = bounds;
        if overflow > px(0.) {
            text_bounds.origin.x -= overflow;
            styled.prepaint(None, inspector_id, text_bounds, &mut (), window, cx);
        }
        (window.insert_hitbox(bounds, HitboxBehavior::Normal), text_bounds)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        styled: &mut StyledText,
        (hitbox, text_bounds): &mut (Hitbox, Bounds<Pixels>),
        window: &mut Window,
        cx: &mut App,
    ) {
        let text_bounds = *text_bounds;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            styled.paint(None, inspector_id, text_bounds, &mut (), &mut (), window, cx);
        });
        let layout = styled.layout().clone();
        let focused = self.focus.is_focused(window);

        // Book-keeping only: no notify during paint.
        let (cursor_byte, show_cursor, empty) = self.state.update(cx, |t, cx| {
            t.layout = Some(layout.clone());
            match (focused, t.blink.is_some()) {
                (true, false) => t.start_blink(cx),
                (false, true) => t.stop_blink(),
                _ => {}
            }
            (t.byte(t.cursor), focused && t.cursor_on, t.text.is_empty())
        });

        if show_cursor {
            let origin = if empty { None } else { layout.position_for_index(cursor_byte) }.unwrap_or(text_bounds.origin);
            let color = self.cursor_color.unwrap_or_else(|| window.text_style().color);
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_quad(fill(Bounds::new(origin, size(px(1.), layout.line_height())), color));
            });
        }

        if let Some(bg) = self.fade_color {
            let text_right = text_bounds.origin.x + layout.bounds().size.width;
            // Where the selection is painted, in window x; the fade over a
            // selected edge must sink into the selection colour, not the
            // field's, or it reads as a grey smear on blue.
            let selected = self.state.read(cx).selection().and_then(|r| {
                let t = self.state.read(cx);
                let a = layout.position_for_index(t.byte(r.start))?.x;
                let b = layout.position_for_index(t.byte(r.end))?.x;
                Some(a..b)
            });
            let sel = self.selection_color;
            // Paint the band in pieces split at the selection's ends, so each
            // piece sinks into the colour under it. Opacity runs from 1 at
            // `solid` to 0 at the far side of the band.
            let fade = |x: Pixels, angle: f32, window: &mut Window| {
                let solid = if angle == 90. { x } else { x + FADE };
                let opacity = |p: Pixels| 1. - ((p - solid).abs() / FADE).clamp(0., 1.);
                let mut cuts = vec![x, x + FADE];
                if let Some(r) = &selected {
                    cuts.extend([r.start, r.end].into_iter().filter(|p| x < *p && *p < x + FADE));
                }
                cuts.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for w in cuts.windows(2) {
                    let (from, to) = (w[0], w[1]);
                    let mid = (from + to) / 2.;
                    let color = if selected.as_ref().is_some_and(|r| r.contains(&mid)) { sel } else { bg };
                    let b = Bounds::new(Point::new(from, bounds.origin.y), size(to - from, bounds.size.height));
                    let (a0, a1) = (opacity(from), opacity(to));
                    window.paint_quad(fill(
                        b,
                        linear_gradient(
                            90.,
                            linear_color_stop(Hsla { a: a0, ..color }, 0.),
                            linear_color_stop(Hsla { a: a1, ..color }, 1.),
                        ),
                    ));
                }
            };
            if text_bounds.origin.x < bounds.origin.x {
                fade(bounds.origin.x, 90., window);
            }
            if text_right > bounds.origin.x + bounds.size.width {
                fade(bounds.origin.x + bounds.size.width - FADE, 270., window);
            }
        }

        window.handle_input(&self.focus, ElementInputHandler::new(bounds, self.state.clone()), cx);

        let index_at = move |p: Point<Pixels>| layout.index_for_position(p).unwrap_or_else(|i| i);
        let state = self.state.clone();
        let focus = self.focus.clone();
        window.on_mouse_event({
            let hitbox = hitbox.clone();
            let (state, index_at) = (state.clone(), index_at.clone());
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                #[cfg(feature = "gpui-git")]
                window.focus(&focus, cx);
                #[cfg(not(feature = "gpui-git"))]
                window.focus(&focus);
                let at = index_at(event.position);
                state.update(cx, |t, cx| {
                    let at = if t.text.is_empty() { 0 } else { t.char_at_byte(at) };
                    t.move_to(at, event.modifiers.shift, cx);
                    t.dragging = true;
                });
            }
        });
        window.on_mouse_event({
            let state = state.clone();
            move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || !state.read(cx).dragging {
                    return;
                }
                let at = index_at(event.position);
                state.update(cx, |t, cx| {
                    let at = if t.text.is_empty() { 0 } else { t.char_at_byte(at) };
                    if at != t.cursor {
                        t.move_to(at, true, cx);
                    }
                });
            }
        });
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && state.read(cx).dragging {
                state.update(cx, |t, _| t.dragging = false);
            }
        });
    }
}
