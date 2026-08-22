//! A value set in one window is what the next window starts with.

use gangplank::{Persisted, use_persisted};
use gpui::{Context, Entity, Render, TestAppContext, Window, div, prelude::*};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Prefs {
    sidebar_width: f32,
    recent: Vec<String>,
}

struct Probe {
    path: PathBuf,
    seen: Option<Entity<Persisted<Prefs>>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.seen = Some(use_persisted(window, cx, &self.path, || Prefs {
            sidebar_width: 240.0,
            recent: vec![],
        }));
        div()
    }
}

fn open(cx: &mut TestAppContext, path: PathBuf) -> Entity<Persisted<Prefs>> {
    let window = cx.add_window(|_, _| Probe { path, seen: None });
    let probe = window.root(cx).unwrap();
    cx.run_until_parked();
    probe.read_with(cx, |p, _| p.seen.clone().unwrap())
}

#[gpui::test]
fn value_survives_a_relaunch(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");

    let first = open(cx, path.clone());
    assert_eq!(first.read_with(cx, |p, _| p.value().sidebar_width), 240.0, "default on first run");

    first.update(cx, |p, cx| {
        p.set(Prefs { sidebar_width: 300.0, recent: vec!["~/Documents".into()] }, cx)
    });

    // A second window is the closest a test gets to a relaunch: new entity,
    // same file.
    let second = open(cx, path);
    assert_eq!(
        second.read_with(cx, |p, _| p.value().clone()),
        Prefs { sidebar_width: 300.0, recent: vec!["~/Documents".into()] }
    );
}

#[gpui::test]
fn unparseable_file_falls_back_to_default(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");
    std::fs::write(&path, b"{ not json").unwrap();
    let state = open(cx, path);
    assert_eq!(state.read_with(cx, |p, _| p.value().sidebar_width), 240.0);
}
