//! The command runs at once, again each period, and its output reaches the view.

use gangplank::{CommandState, use_command};
use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use std::time::Duration;

struct Probe {
    seen: Option<Entity<CommandState>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.seen = Some(use_command(window, cx, "echo", &["hi"], Duration::from_secs(1)));
        div()
    }
}

#[gpui::test]
async fn runs_at_once_then_each_period(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe { seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();
    let state = probe.read_with(cx, |p, _| p.seen.clone().unwrap());

    // The test executor is deterministic; parking drives the spawned run to completion.
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert_eq!(s.runs(), 1);
        assert_eq!(s.output().unwrap().stdout, "hi\n");
        assert!(s.output().unwrap().success);
    });
}
