//! A value that settles: the input is echoed only after it has stopped
//! changing for `delay`. The shape of every `useDebounce` on the web.
//!
//! Feed it a text field's contents, then feed the settled value to
//! [`use_resource`](crate::use_resource) as its key. Typing restarts the
//! clock; the search runs once, for the final text.

use gpui::{App, ElementId, Entity, Task, Window};
use std::panic::Location;
use std::time::Duration;

/// The entity `use_debounce` hands back. Read it with `state.read(cx).value()`.
pub struct DebouncedState<T> {
    /// What the caller passed most recently. Compared on every render so an
    /// unchanged input is a single branch and no timer.
    latest: Option<T>,
    /// The last input that survived a full `delay` unchanged.
    value: T,
    /// Replacing it on an input change drops the pending timer, so only the
    /// last change in a burst ever commits.
    task: Option<Task<()>>,
}

impl<T> DebouncedState<T> {
    pub fn value(&self) -> &T {
        &self.value
    }

    /// `true` while an input change is waiting out its delay.
    pub fn pending(&self) -> bool {
        self.task.is_some()
    }
}

/// The settled form of `value`, identified by the caller's source location.
///
/// The first call commits `value` at once, so the view never renders a
/// placeholder. Call only during render. Two logical instances created at one
/// source line share state — use [`use_keyed_debounce`] for lists.
#[track_caller]
pub fn use_debounce<T>(
    window: &mut Window,
    cx: &mut App,
    value: T,
    delay: Duration,
) -> Entity<DebouncedState<T>>
where
    T: PartialEq + Clone + 'static,
{
    use_keyed_debounce(
        ElementId::CodeLocation(*Location::caller()),
        window,
        cx,
        value,
        delay,
    )
}

/// [`use_debounce`] with an explicit id, for when one source location renders
/// many instances — React's `key` prop.
pub fn use_keyed_debounce<T>(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    value: T,
    delay: Duration,
) -> Entity<DebouncedState<T>>
where
    T: PartialEq + Clone + 'static,
{
    let state = window.use_keyed_state(id, cx, |_, _| DebouncedState {
        latest: None,
        value: value.clone(),
        task: None,
    });

    if state.read(cx).latest.as_ref() == Some(&value) {
        return state;
    }

    state.update(cx, |this, cx| {
        let first = this.latest.is_none();
        this.latest = Some(value.clone());
        if first {
            // Already committed by the initializer; nothing to wait for.
            return;
        }
        let timer = cx.background_executor().timer(delay);
        this.task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                this.value = value;
                this.task = None;
                cx.notify();
            })
            .ok();
        }));
    });

    state
}
