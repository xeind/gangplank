//! Re-render when a file changes. Polls the modified time on a background
//! timer: a stat every `period` is cheap, needs no native watcher, and misses
//! nothing that matters at human timescales. Key a
//! [`use_resource`](crate::use_resource) on `.version()` to reload the file.

use gpui::{App, Context, ElementId, Entity, Task, Window};
use std::fs;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The entity `use_file_watch` hands back.
pub struct FileWatch {
    version: u64,
    _task: Task<()>,
}

impl FileWatch {
    /// Increments each time the file's modified time changes, including
    /// when the file appears or disappears.
    pub fn version(&self) -> u64 {
        self.version
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Identified by the caller's source location; the path and period are fixed
/// at first render. Call only during render.
#[track_caller]
pub fn use_file_watch(
    window: &mut Window,
    cx: &mut App,
    path: impl AsRef<Path>,
    period: Duration,
) -> Entity<FileWatch> {
    use_keyed_file_watch(
        ElementId::CodeLocation(*Location::caller()),
        window,
        cx,
        path,
        period,
    )
}

/// [`use_file_watch`] with an explicit id.
pub fn use_keyed_file_watch(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    path: impl AsRef<Path>,
    period: Duration,
) -> Entity<FileWatch> {
    let path: PathBuf = path.as_ref().to_path_buf();
    window.use_keyed_state(id, cx, |_, cx: &mut Context<FileWatch>| FileWatch {
        version: 0,
        _task: cx.spawn(async move |this, cx| {
            let mut last = modified(&path);
            loop {
                cx.background_executor().timer(period).await;
                let now = modified(&path);
                if now == last {
                    continue;
                }
                last = now;
                let alive = this
                    .update(cx, |this, cx| {
                        this.version += 1;
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }),
    })
}
