//! Keyboard shortcuts without the `actions!` + keymap ceremony: bind
//! `"cmd-k"` to a handler, attach to the view's root element.
//!
//! gpui delivers key events along the focus path only. With nothing focused,
//! that path is the window root, which the view's element is not on. So the
//! hook owns a focus handle, tracks it on the root, and takes focus on first
//! render if nothing else has it. Once a text input takes focus the root is
//! still an ancestor, so the shortcuts keep working.

use gpui::{
    App, Div, ElementId, FocusHandle, InteractiveElement, KeyDownEvent, Keystroke, Window,
};
use std::panic::Location;
use std::rc::Rc;

type Handler = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>;

/// The bindings for one render. Call [`Shortcuts::attach`] on the root element.
pub struct Shortcuts {
    focus: FocusHandle,
    bindings: Vec<(Keystroke, Handler)>,
}

impl Shortcuts {
    /// Bind a gpui keystroke string (`"cmd-k"`, `"escape"`, `"ctrl-shift-p"`)
    /// to `handler`; `cx.listener(...)` produces one. A string that does not
    /// parse panics, since it is a typo in source, not a runtime condition.
    pub fn bind(
        mut self,
        keystroke: &str,
        handler: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let parsed = Keystroke::parse(keystroke)
            .unwrap_or_else(|_| panic!("use_keyboard: cannot parse keystroke {keystroke:?}"));
        self.bindings.push((parsed, Rc::new(handler)));
        self
    }

    /// Wire the bindings into `root`. Put this on the outermost element.
    pub fn attach(self, root: Div) -> Div {
        let bindings = self.bindings;
        root.track_focus(&self.focus)
            .on_key_down(move |event, window, cx| {
                for (keystroke, handler) in &bindings {
                    if matches(keystroke, &event.keystroke) {
                        handler(event, window, cx);
                        cx.stop_propagation();
                        return;
                    }
                }
            })
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }
}

/// The only difference between the parsed binding and the delivered event is
/// `key_char`, which depends on the keyboard layout. Ignore it.
fn matches(binding: &Keystroke, event: &Keystroke) -> bool {
    binding.modifiers == event.modifiers && binding.key == event.key
}

/// Shortcuts for this view, identified by the caller's source location.
/// Chain [`Shortcuts::bind`] for each key, then [`Shortcuts::attach`] on the
/// root element. Call only during render.
#[track_caller]
pub fn use_keyboard(window: &mut Window, cx: &mut App) -> Shortcuts {
    use_keyed_keyboard(ElementId::CodeLocation(*Location::caller()), window, cx)
}

/// [`use_keyboard`] with an explicit id.
pub fn use_keyed_keyboard(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Shortcuts {
    let focus = window
        .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone();
    if window.focused(cx).is_none() {
        window.focus(&focus);
    }
    Shortcuts { focus, bindings: Vec::new() }
}
