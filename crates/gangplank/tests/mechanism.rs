//! Which mechanism actually protects the value: task cancellation, or the key
//! guard? This counts how far superseded work gets, so the answer is measured
//! rather than argued.

use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use gangplank::{ResourceState, use_resource};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const SLOW: Duration = Duration::from_millis(1500);

#[derive(Clone, Default)]
struct Counts {
    started: Arc<AtomicUsize>,
    finished: Arc<AtomicUsize>,
}

struct Probe {
    key: u32,
    counts: Counts,
    seen: Option<Entity<ResourceState<u32, u32>>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.key;
        let executor = cx.background_executor().clone();
        let counts = self.counts.clone();

        let resource = use_resource(window, cx, key, move |k: u32| async move {
            counts.started.fetch_add(1, Ordering::SeqCst);
            executor.timer(SLOW).await;
            counts.finished.fetch_add(1, Ordering::SeqCst);
            k
        });

        self.seen = Some(resource);
        div()
    }
}

#[gpui::test]
fn superseded_work_is_cancelled_before_it_finishes(cx: &mut TestAppContext) {
    let counts = Counts::default();
    let window = cx.add_window({
        let counts = counts.clone();
        |_, _| Probe {
            key: 0,
            counts,
            seen: None,
        }
    });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();

    // Supersede key 0 four times while its work is still in flight.
    for key in 1..=4 {
        probe.update(cx, |probe, cx| {
            probe.key = key;
            cx.notify();
        });
        cx.run_until_parked();
    }

    cx.executor().advance_clock(SLOW * 4);
    cx.run_until_parked();

    let started = counts.started.load(Ordering::SeqCst);
    let finished = counts.finished.load(Ordering::SeqCst);
    assert_eq!(started, 5, "one task per key");
    assert_eq!(
        finished, 1,
        "expected only the surviving task to finish, got {finished} of {started}"
    );
}
