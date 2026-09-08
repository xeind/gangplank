//! One WGSL fragment shader in a GPUI layout, translated by naga. Run with
//! `cargo run --example gradient`. The shader reads the uniforms and the
//! app's `tint` param as WGSL globals.

use gangplank_gpu::{Effect, Param};
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::time::Instant;

const GRADIENT: &str = r#"
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let sweep = sin(uv.x * 4.0 + u.time) * 0.5 + 0.5;
    let base = mix(vec3(0.05, 0.1, 0.3), p.tint.rgb, sweep * uv.y);
    var color = base + 0.15 * cos(u.time + uv.yxy * 6.0 + vec3(0.0, 2.0, 4.0));
    if (u.has_pointer > 0.5) {
        let d = distance(uv * u.resolution, u.pointer) / (u.scale * 60.0);
        color += vec3(exp(-d * d));
    }
    return vec4(color, 1.0);
}
"#;

struct Demo {
    started: Instant,
    gradient: Effect,
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let elapsed = self.started.elapsed();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(div().text_xl().child("gangplank-gpu: WGSL through naga"))
            .child(
                div()
                    .w(px(420.))
                    .h(px(260.))
                    .rounded_lg()
                    .overflow_hidden()
                    .border_2()
                    .border_color(rgb(0xff88aa))
                    .child(
                        self.gradient
                            .element(elapsed)
                            .animate()
                            .follow_pointer()
                            // The parent's 8 px radius minus its 2 px
                            // border, so the fade hugs the border's inside.
                            .rounded(px(6.))
                            .size_full(),
                    ),
            )
            .child(div().text_sm().text_color(rgb(0xaaaaaa)).child(format!(
                "t = {:.1}s. Move the pointer over it.",
                elapsed.as_secs_f32()
            )))
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
    log::set_max_level(log::LevelFilter::Warn);
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(400.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                let gradient = Effect::wgsl(GRADIENT)
                    .params(&[("tint", Param::Float4)])
                    .expect("params");
                gradient.set("tint", [1.0, 0.45, 0.6, 1.0]).expect("tint");
                cx.new(|_| Demo {
                    started: Instant::now(),
                    gradient,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
