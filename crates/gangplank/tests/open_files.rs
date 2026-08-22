//! A path that arrives before the view exists reaches the view's handler.

use gangplank::{OpenFiles, use_open_files};
use gpui::{Context, Render, TestAppContext, Window, div, prelude::*};
use std::path::PathBuf;

struct Probe {
    opened: Vec<PathBuf>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let on_open = cx.listener(|this, paths: &[PathBuf], _, cx| {
            this.opened.extend_from_slice(paths);
            cx.notify();
        });
        use_open_files(window, cx, on_open);
        div()
    }
}

#[gpui::test]
fn early_paths_reach_the_view(cx: &mut TestAppContext) {
    let files = OpenFiles::default();
    files.deliver(vec![PathBuf::from("/early.csv")]);
    cx.update(|cx| files.ready(cx));

    let window = cx.add_window(|_, _| Probe { opened: Vec::new() });
    cx.run_until_parked();
    let probe = window.root(cx).unwrap();
    assert_eq!(probe.read_with(cx, |p, _| p.opened.clone()), vec![PathBuf::from("/early.csv")]);

    files.deliver(vec![PathBuf::from("/late.csv")]);
    cx.run_until_parked();
    assert_eq!(probe.read_with(cx, |p, _| p.opened.len()), 2, "live delivery after the view exists");
}
