//! Copy text and know, for a moment, that you did. The "Copied!" label every
//! app draws after a copy button: `copied()` is true for two seconds after
//! `copy`, then the view re-renders with it false.

use gpui::{App, ClipboardItem, Context, ElementId, Entity, Task, Window};
use std::panic::Location;
use std::time::Duration;

const SHOW_FOR: Duration = Duration::from_secs(2);

/// The entity `use_clipboard` hands back.
pub struct Clipboard {
    /// Replacing it on the next copy restarts the two seconds.
    reset: Option<Task<()>>,
}

impl Clipboard {
    /// Write `text` to the system clipboard and raise the `copied` flag.
    pub fn copy(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
        let timer = cx.background_executor().timer(SHOW_FOR);
        self.reset = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                this.reset = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// True for two seconds after `copy`.
    pub fn copied(&self) -> bool {
        self.reset.is_some()
    }

    /// The clipboard's current text, if it holds any.
    pub fn read(&self, cx: &App) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }
}

/// Identified by the caller's source location. Call only during render.
#[track_caller]
pub fn use_clipboard(window: &mut Window, cx: &mut App) -> Entity<Clipboard> {
    use_keyed_clipboard(ElementId::CodeLocation(*Location::caller()), window, cx)
}

/// [`use_clipboard`] with an explicit id.
pub fn use_keyed_clipboard(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Clipboard> {
    window.use_keyed_state(id, cx, |_, _| Clipboard { reset: None })
}
