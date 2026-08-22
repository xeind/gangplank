//! One tick per period, none before the first, and the loop dies with the view.

use gangplank::{Interval, use_interval};
use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use std::time::Duration;

const PERIOD: Duration = Duration::from_secs(1);

struct Probe {
    seen: Option<Entity<Interval>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.seen = Some(use_interval(window, cx, PERIOD));
        div()
    }
}

#[gpui::test]
fn ticks_once_per_period(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe { seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();
    let interval = probe.read_with(cx, |p, _| p.seen.clone().unwrap());

    let ticks = |cx: &mut TestAppContext| interval.read_with(cx, |i, _| i.ticks());
    assert_eq!(ticks(cx), 0);

    cx.executor().advance_clock(PERIOD / 2);
    cx.run_until_parked();
    assert_eq!(ticks(cx), 0, "no tick before the first period");

    cx.executor().advance_clock(PERIOD * 3);
    cx.run_until_parked();
    assert_eq!(ticks(cx), 3);
}
