//! Stage 1 gate: a superseded task's result is dropped, never written.
//!
//! The test clock makes the race deterministic. Every case starts slow work,
//! supersedes it with fast work, then advances past the slow deadline and
//! asserts nothing overwrote the newer result.

use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use gangplank::{Resource, ResourceState, use_resource};
use std::time::Duration;

const SLOW: Duration = Duration::from_millis(1500);
const FAST: Duration = Duration::from_millis(200);

struct Probe {
    key: u32,
    /// Published during render so the test can read the hook's state.
    seen: Option<Entity<ResourceState<u32, u32>>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.key;
        let executor = cx.background_executor().clone();

        let resource = use_resource(window, cx, key, move |k: u32| async move {
            executor.timer(if k.is_multiple_of(2) { SLOW } else { FAST }).await;
            k
        });

        self.seen = Some(resource);
        div()
    }
}

fn value(probe: &Entity<Probe>, cx: &mut TestAppContext) -> Resource<u32> {
    probe.read_with(cx, |probe, cx| {
        probe
            .seen
            .as_ref()
            .expect("render must have run")
            .read(cx)
            .value()
            .clone()
    })
}

fn set_key(probe: &Entity<Probe>, key: u32, cx: &mut TestAppContext) {
    probe.update(cx, |probe, cx| {
        probe.key = key;
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn slow_result_never_overwrites_the_fast_one_that_superseded_it(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe { key: 0, seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();

    // Key 0 is slow and now in flight.
    assert_eq!(value(&probe, cx), Resource::Loading);

    // Key 1 supersedes it and is fast.
    set_key(&probe, 1, cx);
    assert_eq!(value(&probe, cx), Resource::Loading, "key change resets");

    cx.executor().advance_clock(FAST * 2);
    cx.run_until_parked();
    assert_eq!(value(&probe, cx), Resource::Ready(1), "fast result lands");

    // The gate: key 0's task now finishes. It must not be written.
    cx.executor().advance_clock(SLOW * 2);
    cx.run_until_parked();
    assert_eq!(
        value(&probe, cx),
        Resource::Ready(1),
        "superseded slow result overwrote the newer one"
    );
}

#[gpui::test]
fn final_state_matches_the_final_key_after_rapid_changes(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe { key: 0, seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();

    for key in 1..=5 {
        set_key(&probe, key, cx);
    }

    cx.executor().advance_clock(SLOW * 4);
    cx.run_until_parked();
    assert_eq!(value(&probe, cx), Resource::Ready(5));
}

#[gpui::test]
fn an_unchanged_key_does_not_restart_work(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe { key: 1, seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();

    cx.executor().advance_clock(FAST * 2);
    cx.run_until_parked();
    assert_eq!(value(&probe, cx), Resource::Ready(1));

    // A re-render with the same key must keep the value, not drop to Loading.
    probe.update(cx, |_: &mut Probe, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(value(&probe, cx), Resource::Ready(1));
}
