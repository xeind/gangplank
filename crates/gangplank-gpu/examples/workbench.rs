//! Shader workbench: edit a `.glsl` file, see it reload. Run with
//! `cargo run --example workbench -- path/to/shader.glsl`; with no path it
//! runs `examples/shaders/crt.glsl`.
//!
//! The reload path is two gangplank hooks: `use_file_watch` bumps a version
//! when the file's mtime changes, and a `use_resource` keyed on that version
//! reads the file on the background executor. When the read lands, the view
//! hands the source to `Effect::set_shadertoy` once per version. A shader that
//! fails to compile keeps the last one that worked; the status line shows the
//! error, captured from the crate's `log::error!`.

use gangplank::{Resource, use_file_watch, use_resource};
use gangplank_gpu::Effect;
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type LastError = Arc<Mutex<Option<String>>>;

const BLANK: &str = "void mainImage(out vec4 c, in vec2 f) { c = vec4(0.0); }";

struct Workbench {
    path: PathBuf,
    effect: Effect,
    started: Instant,
    last_applied: Option<u64>,
    last_reload: Option<Duration>,
    last_error: LastError,
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let watch = use_file_watch(window, cx, &self.path, Duration::from_millis(250));
        let version = watch.read(cx).version();
        let path = self.path.clone();
        let source = use_resource(window, cx, version, move |_| async move {
            std::fs::read_to_string(&path).map_err(|err| err.to_string())
        });

        if self.last_applied != Some(version)
            && let Resource::Ready(read) = source.read(cx).value()
        {
            self.last_applied = Some(version);
            self.last_reload = Some(self.started.elapsed());
            match read {
                Ok(glsl) => {
                    *self.last_error.lock().unwrap() = None;
                    self.effect.set_shadertoy(glsl);
                }
                Err(err) => *self.last_error.lock().unwrap() = Some(err.clone()),
            }
        }

        let reload = match self.last_reload {
            Some(at) => format!("reloaded at {:.1}s", at.as_secs_f32()),
            None => "waiting for first read".to_string(),
        };
        let health = match self.last_error.lock().unwrap().as_deref() {
            Some(err) => format!("compile failed, showing last good: {err}"),
            None => "ok".to_string(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x101010))
            .text_color(rgb(0xcccccc))
            .child(
                // Text under the effect, so shaders that read iChannel0
                // (crt.glsl does) have something to draw through.
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .p_6()
                    .text_xl()
                    .child("gangplank-gpu: workbench")
                    .child(div().text_sm().child("edit the shader and save"))
                    .child(
                        div().absolute().inset_0().child(
                            self.effect
                                .element(self.started.elapsed())
                                .animate()
                                .backdrop(px(4.))
                                .size_full(),
                        ),
                    ),
            )
            .child(
                div()
                    .p_2()
                    .text_sm()
                    .child(format!("{}  |  {reload}  |  {health}", self.path.display())),
            )
    }
}

/// Prints every record to stderr and keeps the last error for the status line.
struct Capture(LastError);

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        eprintln!("{}: {}", record.level(), record.args());
        if record.level() == log::Level::Error {
            *self.0.lock().unwrap() = Some(record.args().to_string());
        }
    }
    fn flush(&self) {}
}

fn main() {
    let last_error: LastError = Arc::default();
    log::set_logger(Box::leak(Box::new(Capture(last_error.clone())))).ok();
    log::set_max_level(log::LevelFilter::Error);

    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shaders/crt.glsl")
        });

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.), px(500.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Workbench {
                    path,
                    // Draws nothing until the first read lands.
                    effect: Effect::shadertoy(BLANK),
                    started: Instant::now(),
                    last_applied: None,
                    last_reload: None,
                    last_error,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
