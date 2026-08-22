//! A write bumps the version once; a quiet file does not.

use gangplank::{FileWatch, use_file_watch};
use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use std::path::PathBuf;
use std::time::Duration;

const PERIOD: Duration = Duration::from_millis(500);

struct Probe {
    path: PathBuf,
    seen: Option<Entity<FileWatch>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.seen = Some(use_file_watch(window, cx, &self.path, PERIOD));
        div()
    }
}

#[gpui::test]
fn version_bumps_once_per_change(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("watched.txt");
    std::fs::write(&path, "one").unwrap();

    let window = cx.add_window(|_, _| Probe { path: path.clone(), seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();
    let watch = probe.read_with(cx, |p, _| p.seen.clone().unwrap());
    let version = |cx: &mut TestAppContext| watch.read_with(cx, |w, _| w.version());

    cx.executor().advance_clock(PERIOD * 2);
    cx.run_until_parked();
    assert_eq!(version(cx), 0, "quiet file");

    // Filesystems round mtime; set it explicitly so the change is visible.
    std::fs::write(&path, "two").unwrap();
    let later = std::time::SystemTime::now() + Duration::from_secs(5);
    std::fs::File::open(&path).unwrap().set_modified(later).unwrap();

    cx.executor().advance_clock(PERIOD * 2);
    cx.run_until_parked();
    assert_eq!(version(cx), 1, "one change, one bump");
}
