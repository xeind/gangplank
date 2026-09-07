//! One fragment shader in a GPUI layout. Run with `cargo run --example plasma`.

use gangplank_gpu::Effect;
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::time::Instant;

const PLASMA: &str = r#"
float4 effect(float2 uv, constant EffectUniforms &u) {
    float wave = sin(uv.x * 6.0 + u.time) + cos(uv.y * 5.0 - u.time * 0.7);
    float3 color = 0.5 + 0.5 * cos(u.time * 0.5 + wave + uv.xyx * 2.0 + float3(0.0, 2.0, 4.0));
    if (u.has_pointer > 0.5) {
        float d = distance(uv * u.resolution, u.pointer) / (u.scale * 70.0);
        color += exp(-d * d) * 0.9;
    }
    return float4(color, 1.0);
}
"#;

struct Demo {
    started: Instant,
    plasma: Effect,
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
            .child(div().text_xl().child("gangplank-gpu: Effect"))
            .child(
                div()
                    .w(px(420.))
                    .h(px(260.))
                    .rounded_lg()
                    .overflow_hidden()
                    .border_2()
                    .border_color(rgb(0x8888ff))
                    .child(self.plasma.element(elapsed).animate().size_full()),
            )
            .child(div().text_sm().text_color(rgb(0xaaaaaa)).child(format!(
                "t = {:.1}s. Move the pointer over it.",
                elapsed.as_secs_f32()
            )))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(400.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Demo {
                    started: Instant::now(),
                    plasma: Effect::new(PLASMA),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
