//! Keys go through the window's focus path and the IME input handler, the
//! same two routes a real keyboard takes. Indices in asserts are chars.

use gangplank::{TextInput, text_input, use_text_input};
use gpui::{
    AnyWindowHandle, Context, Entity, EntityInputHandler, FocusHandle, Render, TestAppContext,
    Window, WindowHandle, div, prelude::*,
};
use std::time::Duration;

struct Probe {
    focus: FocusHandle,
    seen: Option<Entity<TextInput>>,
    submitted: Vec<String>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = use_text_input(window, cx);
        self.seen = Some(state.clone());
        let probe = cx.entity();
        div().child(text_input(&state, &self.focus).on_submit(move |text, _, cx| {
            let text = text.to_string();
            probe.update(cx, |p, _| p.submitted.push(text));
        }))
    }
}

struct Rig {
    window: WindowHandle<Probe>,
    probe: Entity<Probe>,
    state: Entity<TextInput>,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    let window = cx.add_window(|_, cx| Probe { focus: cx.focus_handle(), seen: None, submitted: Vec::new() });
    let probe = window.root(cx).unwrap();
    window.update(cx, |p, window, _| window.focus(&p.focus)).unwrap();
    cx.run_until_parked();
    let state = probe.read_with(cx, |p, _| p.seen.clone().unwrap());
    Rig { window, probe, state }
}

impl Rig {
    fn any(&self) -> AnyWindowHandle {
        self.window.into()
    }
    fn type_text(&self, cx: &mut TestAppContext, s: &str) {
        cx.simulate_input(self.any(), s);
    }
    fn keys(&self, cx: &mut TestAppContext, s: &str) {
        cx.simulate_keystrokes(self.any(), s);
    }
    fn text(&self, cx: &TestAppContext) -> String {
        self.state.read_with(cx, |t, _| t.text().to_string())
    }
    fn cursor(&self, cx: &TestAppContext) -> usize {
        self.state.read_with(cx, |t, _| t.cursor())
    }
}

#[gpui::test]
fn insert_and_delete_at_both_ends(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_text(cx, "héllo");
    assert_eq!(r.text(cx), "héllo");
    assert_eq!(r.cursor(cx), 5);

    r.keys(cx, "delete");
    assert_eq!(r.text(cx), "héllo", "delete at end is a no-op");
    r.keys(cx, "backspace");
    assert_eq!(r.text(cx), "héll");

    r.keys(cx, "home backspace");
    assert_eq!(r.text(cx), "héll", "backspace at start is a no-op");
    r.keys(cx, "delete");
    assert_eq!(r.text(cx), "éll");
    assert_eq!(r.cursor(cx), 0);
}

#[gpui::test]
fn typing_replaces_the_selection(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_text(cx, "abcdef");
    r.keys(cx, "left left shift-left shift-left");
    assert_eq!(r.state.read_with(cx, |t, _| t.selection()), Some(2..4));
    r.type_text(cx, "X");
    assert_eq!(r.text(cx), "abXef");
    assert_eq!(r.cursor(cx), 3);
    assert_eq!(r.state.read_with(cx, |t, _| t.selection()), None);
}

#[gpui::test]
fn home_end_and_cmd_arrows(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_text(cx, "abc");
    r.keys(cx, "home");
    assert_eq!(r.cursor(cx), 0);
    r.keys(cx, "end");
    assert_eq!(r.cursor(cx), 3);
    r.keys(cx, "cmd-left");
    assert_eq!(r.cursor(cx), 0);
    r.keys(cx, "cmd-shift-right");
    assert_eq!(r.state.read_with(cx, |t, _| t.selection()), Some(0..3));
    r.keys(cx, "left");
    assert_eq!(r.cursor(cx), 0, "left collapses to the selection start");
}

#[gpui::test]
fn select_all_copy_cut_paste(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_text(cx, "copy me");
    r.keys(cx, "cmd-a cmd-c");
    assert_eq!(cx.read_from_clipboard().and_then(|i| i.text()), Some("copy me".into()));
    r.keys(cx, "cmd-x");
    assert_eq!(r.text(cx), "");
    r.keys(cx, "cmd-v cmd-v");
    assert_eq!(r.text(cx), "copy mecopy me");
    assert_eq!(r.cursor(cx), 14);
}

#[gpui::test]
fn ime_marks_then_commits(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_text(cx, "ab");
    r.keys(cx, "left");
    // Pinyin-style: composition grows, then the candidate replaces it.
    r.window
        .update(cx, |_, window, cx| {
            r.state.update(cx, |t, cx| {
                t.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
                t.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                assert_eq!(t.text(), "anib");
                assert_eq!(t.marked_text_range(window, cx), Some(1..3));
                t.replace_text_in_range(None, "你", window, cx);
                assert_eq!(t.marked_text_range(window, cx), None);
            })
        })
        .unwrap();
    assert_eq!(r.text(cx), "a你b");
    assert_eq!(r.cursor(cx), 2);

    // Ranges the IME sends are UTF-16; 你 is one unit, 𝄞 is two.
    r.keys(cx, "end");
    r.window
        .update(cx, |_, window, cx| {
            r.state.update(cx, |t, cx| {
                t.replace_text_in_range(None, "𝄞", window, cx);
                let mut adjusted = None;
                assert_eq!(t.text_for_range(1..5, &mut adjusted, window, cx).as_deref(), Some("你b𝄞"));
                assert_eq!(t.selected_text_range(false, window, cx).map(|s| s.range), Some(5..5));
            })
        })
        .unwrap();
}

#[gpui::test]
fn enter_goes_to_on_submit_and_newlines_stay_out(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_text(cx, "go");
    r.keys(cx, "enter");
    assert_eq!(r.probe.read_with(cx, |p, _| p.submitted.clone()), vec!["go".to_string()]);
    assert_eq!(r.text(cx), "go");
}

#[gpui::test]
fn cursor_blinks_on_the_clock(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_text(cx, "a");
    let visible = |cx: &TestAppContext| r.state.read_with(cx, |t, _| t.cursor_visible());
    assert!(visible(cx));
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(!visible(cx), "off after half a second");
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(visible(cx), "on again");

    // An edit shows the cursor and restarts the clock.
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    r.type_text(cx, "b");
    assert!(visible(cx));
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(visible(cx), "the old deadline must not fire");
}
