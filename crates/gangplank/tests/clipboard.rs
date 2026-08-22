//! Copy lands in the clipboard; the flag holds for two seconds, then drops.

use gangplank::{Clipboard, use_clipboard};
use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use std::time::Duration;

struct Probe {
    seen: Option<Entity<Clipboard>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.seen = Some(use_clipboard(window, cx));
        div()
    }
}

#[gpui::test]
fn copied_flag_clears_after_two_seconds(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Probe { seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();
    let clip = probe.read_with(cx, |p, _| p.seen.clone().unwrap());

    clip.update(cx, |c, cx| c.copy("hello", cx));
    assert!(clip.read_with(cx, |c, _| c.copied()));
    assert_eq!(clip.read_with(cx, |c, cx| c.read(cx)), Some("hello".to_string()));

    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(clip.read_with(cx, |c, _| c.copied()), "still shown at 1 s");

    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(!clip.read_with(cx, |c, _| c.copied()), "cleared at 2 s");
}
