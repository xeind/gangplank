//! Files handed to the app from outside: Finder "Open With", a drop on the
//! dock icon, `open -a`, a URL scheme, and `argv`. One stream, delivered to a
//! view's handler.
//!
//! macOS can deliver the file before the first window exists, so the handler
//! is registered on [`Application`] before `run`; anything that arrives early
//! is held until a view calls [`use_open_files`]. URLs arrive
//! percent-encoded (`my%20data.csv`); they are decoded here.
//!
//! ```ignore
//! let files = OpenFiles::install(&app);
//! app.run(move |cx| { files.ready(cx); /* open windows */ });
//! // in render:
//! use_open_files(window, cx, cx.listener(|this, paths, _, cx| this.open(&paths[0], cx)));
//! ```

use gpui::{App, Application, AsyncApp, ElementId, Global, Window};
use std::cell::RefCell;
use std::panic::Location;
use std::path::PathBuf;
use std::rc::Rc;

type Handler = Rc<dyn Fn(&[PathBuf], &mut App)>;

#[derive(Default)]
struct Inner {
    /// Arrived before any handler existed.
    buffered: Vec<PathBuf>,
    /// Set once `ready` has run, so the callback can reach the app.
    app: Option<AsyncApp>,
    handler: Option<Handler>,
}

/// The registration handle. Clone is cheap; all clones share one inbox.
#[derive(Clone, Default)]
pub struct OpenFiles(Rc<RefCell<Inner>>);

impl Global for OpenFiles {}

impl OpenFiles {
    /// Register with the platform before `run`. Paths in `argv` count as
    /// opened files too, so `my-app file.csv` and double-click behave alike.
    pub fn install(app: &Application) -> Self {
        let this = Self::default();
        this.0.borrow_mut().buffered = std::env::args().skip(1).map(PathBuf::from).filter(|p| p.exists()).collect();
        app.on_open_urls({
            let this = this.clone();
            move |urls| this.deliver(urls.iter().filter_map(|u| url_to_path(u)).collect())
        });
        this
    }

    /// Call once inside `run`, before opening windows.
    pub fn ready(&self, cx: &mut App) {
        self.0.borrow_mut().app = Some(cx.to_async());
        cx.set_global(self.clone());
    }

    /// Hand `paths` to the handler, or hold them until one exists.
    /// Public so tests and custom sources (a drop target, say) can feed it.
    pub fn deliver(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let (handler, app) = {
            let inner = self.0.borrow();
            (inner.handler.clone(), inner.app.clone())
        };
        match (handler, app) {
            (Some(handler), Some(app)) => {
                app.update(|cx| handler(&paths, cx)).ok();
            }
            _ => self.0.borrow_mut().buffered.extend(paths),
        }
    }

    fn set_handler(&self, handler: Handler, cx: &mut App) {
        let buffered = {
            let mut inner = self.0.borrow_mut();
            inner.handler = Some(handler.clone());
            std::mem::take(&mut inner.buffered)
        };
        // Called from render, where the window is already borrowed; deliver
        // once the render finishes.
        if !buffered.is_empty() {
            cx.defer(move |cx| handler(&buffered, cx));
        }
    }
}

/// `file:///Users/x/my%20data.csv` → `/Users/x/my data.csv`. Other schemes
/// pass through as-is so a custom URL scheme still reaches the handler.
pub fn url_to_path(url: &str) -> Option<PathBuf> {
    let Some(rest) = url.strip_prefix("file://") else {
        return Some(PathBuf::from(url));
    };
    let mut out = Vec::with_capacity(rest.len());
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8(out).ok()?))
}

/// Receive opened files in this view. `handler` is replaced on every render
/// so it can capture fresh state; `cx.listener(...)` produces one. Files that
/// arrived before the first call are delivered during it. Call only during
/// render, after [`OpenFiles::ready`].
#[track_caller]
pub fn use_open_files(window: &mut Window, cx: &mut App, handler: impl Fn(&[PathBuf], &mut Window, &mut App) + 'static) {
    use_keyed_open_files(ElementId::CodeLocation(*Location::caller()), window, cx, handler)
}

/// [`use_open_files`] with an explicit id.
pub fn use_keyed_open_files(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    handler: impl Fn(&[PathBuf], &mut Window, &mut App) + 'static,
) {
    let files = cx
        .try_global::<OpenFiles>()
        .cloned()
        .expect("use_open_files: call OpenFiles::ready(cx) inside Application::run first");
    let window_handle = window.window_handle();
    let handler: Handler = Rc::new(move |paths, cx| {
        window_handle.update(cx, |_, window, cx| handler(paths, window, cx)).ok();
    });
    // The id keeps one registration per call site across renders.
    let _ = window.use_keyed_state(id, cx, |_, _| ());
    files.set_handler(handler, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_urls() {
        assert_eq!(url_to_path("file:///Users/x/my%20data.csv").unwrap(), PathBuf::from("/Users/x/my data.csv"));
        assert_eq!(url_to_path("file:///a/b.csv").unwrap(), PathBuf::from("/a/b.csv"));
        assert_eq!(url_to_path("csvgrid://open").unwrap(), PathBuf::from("csvgrid://open"));
        assert_eq!(url_to_path("file:///100%25.csv").unwrap(), PathBuf::from("/100%.csv"));
    }

    #[test]
    fn buffers_until_a_handler_exists() {
        let files = OpenFiles::default();
        files.deliver(vec![PathBuf::from("/a")]);
        files.deliver(vec![PathBuf::from("/b")]);
        assert_eq!(files.0.borrow().buffered.len(), 2);
    }
}
