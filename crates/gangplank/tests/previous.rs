//! None on the first render, then the last render's value.

use gangplank::use_previous;
use gpui::{Context, Render, TestAppContext, Window, div, prelude::*};

struct Probe {
    count: u32,
    log: Vec<Option<u32>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let previous = use_previous(window, cx, self.count);
        self.log.push(previous);
        div()
    }
}

#[gpui::test]
fn previous_lags_current_by_one_render(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe { count: 0, log: vec![] });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();

    for count in [5, 9] {
        probe.update(cx, |p, cx| {
            p.count = count;
            cx.notify();
        });
        cx.run_until_parked();
    }
    assert_eq!(probe.read_with(cx, |p, _| p.log.clone()), [None, Some(0), Some(5)]);
}
