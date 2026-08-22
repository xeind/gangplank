//! The value from the previous render. Diff against it to animate a change,
//! detect a direction, or log what moved.

use gpui::{App, ElementId, Entity, Window};
use std::panic::Location;

struct Slot<T> {
    previous: Option<T>,
    /// `None` only before the first render completes the swap below.
    current: Option<T>,
}

/// `None` on the first render, then whatever `value` was last time.
/// Identified by the caller's source location. Call only during render.
#[track_caller]
pub fn use_previous<T: Clone + 'static>(window: &mut Window, cx: &mut App, value: T) -> Option<T> {
    use_keyed_previous(ElementId::CodeLocation(*Location::caller()), window, cx, value)
}

/// [`use_previous`] with an explicit id.
pub fn use_keyed_previous<T: Clone + 'static>(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    value: T,
) -> Option<T> {
    let slot: Entity<Slot<T>> = window.use_keyed_state(id, cx, |_, _| Slot {
        previous: None,
        current: None,
    });
    // Mutate without notify: the caller is mid-render, and a notify here
    // would schedule a second render for nothing.
    slot.update(cx, |slot, _| {
        slot.previous = slot.current.replace(value);
        slot.previous.clone()
    })
}
