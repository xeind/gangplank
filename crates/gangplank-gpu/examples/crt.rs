//! A CRT overlay on top of ordinary GPUI widgets. Run with `cargo run --example crt`.
//!
//! The overlay is `examples/shaders/crt.glsl`, a Shadertoy style shader that
//! returns alpha, drawn absolutely over a fake terminal-ish app. The widgets
//! below it stay interactive: hover the rows.

use gangplank_gpu::Effect;
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::time::Instant;

const CRT: &str = include_str!("shaders/crt.glsl");

struct Crt {
    started: Instant,
    overlay: Effect,
    enabled: bool,
}

impl Render for Crt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.started.elapsed();
        let lines = [
            "$ cargo run --example crt",
            "   Compiling gangplank-gpu v0.1.0",
            "    Finished dev profile in 2.1s",
            "     Running target/debug/examples/crt",
            "gpui: window opened, scale 2.0",
            "effect: crt.glsl compiled",
            "$ █",
        ];

        div()
            .relative()
            .size_full()
            .bg(rgb(0x0b0f0c))
            .font_family("Menlo")
            .text_color(rgb(0x9ee6a5))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p_5()
                    .gap_1()
                    .child(
                        div()
                            .id("toggle")
                            .mb_3()
                            .px_2()
                            .py_1()
                            .w(px(180.))
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(0x3d7a45))
                            .hover(|s| s.bg(rgb(0x163a1c)))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.enabled = !this.enabled;
                                cx.notify();
                            }))
                            .child(if self.enabled {
                                "CRT: on (click)"
                            } else {
                                "CRT: off (click)"
                            }),
                    )
                    .children(
                        lines.iter().enumerate().map(|(i, l)| {
                            div().id(i).px_1().hover(|s| s.bg(rgb(0x1a2f1e))).child(*l)
                        }),
                    ),
            )
            .children(self.enabled.then(|| {
                div()
                    .absolute()
                    .inset_0()
                    .child(self.overlay.element(t).animate().size_full())
            }))
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
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(420.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Crt {
                    started: Instant::now(),
                    overlay: Effect::shadertoy(CRT),
                    enabled: true,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
