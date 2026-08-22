//! A bound keystroke reaches its handler with nothing focused; an unbound one
//! does nothing; the binding still fires after a child takes focus.

use gangplank::use_keyboard;
use gpui::{Context, FocusHandle, Render, TestAppContext, Window, div, prelude::*};

struct Probe {
    child: FocusHandle,
    hits: Vec<&'static str>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let keys = use_keyboard(window, cx)
            .bind("cmd-k", cx.listener(|this, _, _, _| this.hits.push("palette")))
            .bind("escape", cx.listener(|this, _, _, _| this.hits.push("escape")));
        keys.attach(div()).child(div().track_focus(&self.child))
    }
}

#[gpui::test]
fn shortcuts_fire_with_and_without_child_focus(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, cx| Probe {
        child: cx.focus_handle(),
        hits: vec![],
    });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();

    cx.simulate_keystrokes(window.into(), "cmd-k");
    cx.simulate_keystrokes(window.into(), "x");
    cx.run_until_parked();
    assert_eq!(probe.read_with(cx, |p, _| p.hits.clone()), ["palette"]);

    window
        .update(cx, |probe, window, cx| {
            window.focus(&probe.child, cx);
        })
        .unwrap();
    cx.run_until_parked();

    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    assert_eq!(
        probe.read_with(cx, |p, _| p.hits.clone()),
        ["palette", "escape"],
        "root binding still fires when a child has focus"
    );
}
