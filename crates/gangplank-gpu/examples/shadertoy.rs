//! Run a Shadertoy or Ghostty style `.glsl` file and hot-reload it on save.
//!
//! `cargo run --example shadertoy -- path/to/shader.glsl`
//! With no path it runs `examples/shaders/starfield.glsl`.
//! Edit the file in any editor; the window picks up the change within 250ms.
//! Compile errors print to stderr and the previous shader keeps running.

use gangplank_gpu::Effect;
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

struct Runner {
    path: PathBuf,
    modified: Option<SystemTime>,
    effect: Option<Effect>,
    started: Instant,
    status: String,
}

impl Runner {
    fn reload_if_changed(&mut self) {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            self.status = format!("cannot read {}", self.path.display());
            return;
        };
        let modified = meta.modified().ok();
        if modified == self.modified && self.effect.is_some() {
            return;
        }
        self.modified = modified;
        match std::fs::read_to_string(&self.path) {
            Ok(glsl) => {
                self.effect = Some(Effect::shadertoy(&glsl));
                self.started = Instant::now();
                self.status = format!("{}  (edit and save to reload)", self.path.display());
            }
            Err(err) => self.status = format!("{}: {err}", self.path.display()),
        }
    }
}

impl Render for Runner {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.started.elapsed();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x101010))
            .text_color(rgb(0xcccccc))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .children(self.effect.as_ref().map(|e| e.element(t).animate().size_full())),
            )
            .child(div().p_2().text_sm().child(self.status.clone()))
    }
}

struct Stderr;
impl log::Log for Stderr {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        eprintln!("{}: {}", record.level(), record.args());
    }
    fn flush(&self) {}
}

fn main() {
    log::set_logger(&Stderr).ok();
    log::set_max_level(log::LevelFilter::Error);
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shaders/starfield.glsl")
        });

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.), px(500.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| {
                        let mut runner = Runner {
                            path,
                            modified: None,
                            effect: None,
                            started: Instant::now(),
                            status: String::new(),
                        };
                        runner.reload_if_changed();
                        runner
                    })
                },
            )
            .unwrap();
        let runner = window.root(cx).unwrap();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if runner
                    .update(cx, |r, cx| {
                        r.reload_if_changed();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}
