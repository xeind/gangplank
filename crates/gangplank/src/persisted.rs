//! A value that survives relaunch: one JSON file per value, read on first
//! render, written on every `set`. Window position, recent folders, the
//! sidebar's width — the small things an app forgets between runs.
//!
//! Writes are synchronous. The files are a few hundred bytes; a queue would be
//! more code than the problem.

use gpui::{App, Context, ElementId, Entity, Window};
use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::panic::Location;
use std::path::{Path, PathBuf};

/// The platform's per-app data folder for `identifier`, created if missing.
/// `~/Library/Application Support/<identifier>` on macOS.
pub fn data_dir(identifier: &str) -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(identifier);
    fs::create_dir_all(&dir).ok();
    dir
}

/// The entity `use_persisted` hands back.
pub struct Persisted<T> {
    path: PathBuf,
    value: T,
}

impl<T: Serialize + 'static> Persisted<T> {
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Replace the value, write it to disk, and notify observers. A write
    /// failure keeps the in-memory value; the next `set` tries again.
    pub fn set(&mut self, value: T, cx: &mut Context<Self>) {
        self.value = value;
        if let Ok(json) = serde_json::to_vec_pretty(&self.value) {
            fs::write(&self.path, json).ok();
        }
        cx.notify();
    }
}

/// `default`, unless `path` holds a value from a previous run.
///
/// Identified by the caller's source location; call only during render. Two
/// logical instances at one source line share state — use
/// [`use_keyed_persisted`] for lists.
#[track_caller]
pub fn use_persisted<T>(
    window: &mut Window,
    cx: &mut App,
    path: impl AsRef<Path>,
    default: impl FnOnce() -> T,
) -> Entity<Persisted<T>>
where
    T: Serialize + DeserializeOwned + 'static,
{
    use_keyed_persisted(
        ElementId::CodeLocation(*Location::caller()),
        window,
        cx,
        path,
        default,
    )
}

/// [`use_persisted`] with an explicit id.
pub fn use_keyed_persisted<T>(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    path: impl AsRef<Path>,
    default: impl FnOnce() -> T,
) -> Entity<Persisted<T>>
where
    T: Serialize + DeserializeOwned + 'static,
{
    let path = path.as_ref().to_path_buf();
    window.use_keyed_state(id, cx, |_, _| {
        // A missing or unreadable file is the first run; a file that no
        // longer parses is an old schema. Both start from `default`.
        let value = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(default);
        Persisted { path, value }
    })
}
