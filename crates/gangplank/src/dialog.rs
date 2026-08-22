//! Native open and save panels as one call with a callback. gpui returns a
//! oneshot channel inside a `Result` inside an `Option`; this unwraps that and
//! treats cancel and failure alike, since the view does the same thing for
//! both: nothing.

use gpui::{App, PathPromptOptions, Task};
use std::path::{Path, PathBuf};

/// Show the open panel. `on_pick` runs with the chosen paths, or not at all.
pub fn pick_files(
    cx: &mut App,
    options: PathPromptOptions,
    on_pick: impl FnOnce(Vec<PathBuf>, &mut App) + 'static,
) -> Task<()> {
    let rx = cx.prompt_for_paths(options);
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = rx.await {
            cx.update(|cx| on_pick(paths, cx)).ok();
        }
    })
}

/// [`pick_files`] for one file.
pub fn pick_file(cx: &mut App, on_pick: impl FnOnce(PathBuf, &mut App) + 'static) -> Task<()> {
    let options = PathPromptOptions { files: true, directories: false, multiple: false, prompt: None };
    pick_files(cx, options, move |mut paths, cx| {
        if let Some(path) = paths.pop() {
            on_pick(path, cx)
        }
    })
}

/// Show the save panel, starting in `directory` with `suggested_name` filled in.
pub fn pick_save_path(
    cx: &mut App,
    directory: &Path,
    suggested_name: Option<&str>,
    on_pick: impl FnOnce(PathBuf, &mut App) + 'static,
) -> Task<()> {
    let rx = cx.prompt_for_new_path(directory, suggested_name);
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(path))) = rx.await {
            cx.update(|cx| on_pick(path, cx)).ok();
        }
    })
}
