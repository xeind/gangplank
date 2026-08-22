//! A burst of changes commits once, with the last value, after the delay.
//! Measured on the test clock, so timing is exact.

use gangplank::{DebouncedState, use_debounce};
use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use std::time::Duration;

const DELAY: Duration = Duration::from_millis(300);

struct Probe {
    input: String,
    seen: Option<Entity<DebouncedState<String>>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.seen = Some(use_debounce(window, cx, self.input.clone(), DELAY));
        div()
    }
}

#[gpui::test]
fn burst_commits_only_the_last_value(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe {
        input: String::new(),
        seen: None,
    });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();

    let settled = |cx: &mut TestAppContext| {
        probe.read_with(cx, |p, cx| p.seen.as_ref().unwrap().read(cx).value().clone())
    };
    assert_eq!(settled(cx), "", "first value commits at once");

    // Type "abc" one key every 100 ms: each change lands inside the previous delay.
    for text in ["a", "ab", "abc"] {
        probe.update(cx, |p, cx| {
            p.input = text.to_string();
            cx.notify();
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert_eq!(settled(cx), "", "nothing commits while typing");
    }

    cx.executor().advance_clock(DELAY);
    cx.run_until_parked();
    assert_eq!(settled(cx), "abc", "only the final value commits");

    let pending = probe.read_with(cx, |p, cx| p.seen.as_ref().unwrap().read(cx).pending());
    assert!(!pending);
}
