//! A window that reopens where it was closed. Two calls: one before
//! `open_window` to read the saved bounds, one during render to save them.
//!
//! ```ignore
//! let file = data_dir("dev.example.app").join("window.json");
//! let bounds = saved_window_bounds(&file).unwrap_or_else(|| Bounds::centered(None, size(px(800.), px(600.)), cx));
//! // in render:
//! use_window_state(window, cx, &file);
//! ```

use crate::{use_keyed_debounce, use_keyed_persisted};
use gpui::{App, Bounds, ElementId, Pixels, Window, point, px, size};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use std::panic::Location;
use std::path::Path;
use std::time::Duration;

/// Whole pixels; a window never sits at a fraction.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
struct Saved {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Saved {
    fn from_bounds(b: Bounds<Pixels>) -> Self {
        Saved { x: f32::from(b.origin.x) as i32, y: f32::from(b.origin.y) as i32, w: f32::from(b.size.width) as i32, h: f32::from(b.size.height) as i32 }
    }
    fn to_bounds(self) -> Bounds<Pixels> {
        Bounds { origin: point(px(self.x as f32), px(self.y as f32)), size: size(px(self.w as f32), px(self.h as f32)) }
    }
}

/// The bounds saved by [`use_window_state`], if any. Call before `open_window`.
pub fn saved_window_bounds(file: impl AsRef<Path>) -> Option<Bounds<Pixels>> {
    let saved: Saved = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    (saved.w > 0 && saved.h > 0).then(|| saved.to_bounds())
}

/// Save this window's bounds to `file` once they have held still for half a
/// second. Identified by the caller's source location; call only during render.
#[track_caller]
pub fn use_window_state(window: &mut Window, cx: &mut App, file: impl AsRef<Path>) {
    use_keyed_window_state(ElementId::CodeLocation(*Location::caller()), window, cx, file)
}

/// [`use_window_state`] with an explicit id.
pub fn use_keyed_window_state(id: impl Into<ElementId>, window: &mut Window, cx: &mut App, file: impl AsRef<Path>) {
    let id = Arc::new(id.into());
    let child = |name: &'static str| ElementId::NamedChild(id.clone(), name.into());
    let now = Saved::from_bounds(window.bounds());
    let settled = *use_keyed_debounce(child("settled"), window, cx, now, Duration::from_millis(500))
        .read(cx)
        .value();
    let store = use_keyed_persisted(child("file"), window, cx, file, || settled);
    if *store.read(cx).value() != settled {
        store.update(cx, |s, cx| s.set(settled, cx));
    }
}
