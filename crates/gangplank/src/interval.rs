//! A tick every `period`, for as long as the view renders. Poll a folder,
//! refresh an index, advance a clock. The shape of every `useInterval`.
//!
//! The view reads `.ticks()` and renders it; each tick notifies, so the view
//! re-renders, so any work keyed on the tick count runs again. Pair with
//! [`use_resource`](crate::use_resource) keyed on `ticks` for periodic async
//! work that still cancels cleanly.

use gpui::{App, Context, ElementId, Entity, Task, Window};
use std::panic::Location;
use std::time::Duration;

/// The entity `use_interval` hands back.
pub struct Interval {
    ticks: u64,
    /// Dropping the entity drops the task, which ends the loop.
    _task: Task<()>,
}

impl Interval {
    /// How many periods have elapsed since the first render.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
}

/// A counter that increments every `period`, identified by the caller's
/// source location. The period is fixed at first render; a later change is
/// ignored. Call only during render.
#[track_caller]
pub fn use_interval(window: &mut Window, cx: &mut App, period: Duration) -> Entity<Interval> {
    use_keyed_interval(ElementId::CodeLocation(*Location::caller()), window, cx, period)
}

/// [`use_interval`] with an explicit id.
pub fn use_keyed_interval(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    period: Duration,
) -> Entity<Interval> {
    window.use_keyed_state(id, cx, |_, cx: &mut Context<Interval>| Interval {
        ticks: 0,
        _task: cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(period).await;
                let alive = this
                    .update(cx, |this, cx| {
                        this.ticks += 1;
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }),
    })
}
