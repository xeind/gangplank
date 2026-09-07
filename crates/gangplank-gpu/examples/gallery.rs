//! Four effects in one layout. Run with `cargo run --example gallery`.
//!
//! Each tile is one fragment function. Move the pointer over the tiles.

use gangplank_gpu::Effect;
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::time::Instant;

/// Rings spread out from wherever the pointer is; idle, they come from the centre.
const RIPPLE: &str = r#"
float4 effect(float2 uv, constant EffectUniforms &u) {
    float2 p = uv * u.resolution;
    float2 c = u.has_pointer > 0.5 ? u.pointer : u.resolution * 0.5;
    float d = distance(p, c) / (u.scale * 40.0);
    float ring = 0.5 + 0.5 * sin(d * 6.0 - u.time * 4.0);
    ring *= exp(-d * 0.35);
    float3 base = float3(0.05, 0.08, 0.15);
    return float4(base + ring * float3(0.2, 0.7, 1.0), 1.0);
}
"#;

/// Clouds: fractal noise scrolling. No textures, all math.
const CLOUDS: &str = r#"
float hash(float2 p) { return fract(sin(dot(p, float2(127.1, 311.7))) * 43758.5453); }
float noise(float2 p) {
    float2 i = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + float2(1, 0)), f.x),
               mix(hash(i + float2(0, 1)), hash(i + float2(1, 1)), f.x), f.y);
}
float4 effect(float2 uv, constant EffectUniforms &u) {
    float2 p = uv * float2(u.resolution.x / u.resolution.y, 1.0) * 3.0;
    p.x += u.time * 0.15;
    float v = 0.0, a = 0.5;
    for (int i = 0; i < 5; i++) { v += a * noise(p); p *= 2.1; a *= 0.5; }
    float3 sky = mix(float3(0.25, 0.45, 0.85), float3(0.9, 0.95, 1.0), uv.y * 0.6);
    float cloud = smoothstep(0.45, 0.75, v);
    return float4(mix(sky, float3(1.0), cloud), 1.0);
}
"#;

/// A loading ring: signed-distance ring with an animated sweep, anti-aliased.
const RING: &str = r#"
float4 effect(float2 uv, constant EffectUniforms &u) {
    float2 p = (uv - 0.5) * u.resolution;
    float r = min(u.resolution.x, u.resolution.y) * 0.32;
    float w = 8.0 * u.scale;
    float d = abs(length(p) - r) - w * 0.5;
    float aa = 1.0 * u.scale;
    float ring = 1.0 - smoothstep(-aa, aa, d);
    float ang = atan2(p.y, p.x);
    float sweep = fract((ang / 6.2831853) - u.time * 0.5);
    float3 track = float3(0.16);
    float3 arc = mix(float3(0.3, 0.5, 1.0), float3(0.9, 0.3, 0.8), sweep);
    float3 col = mix(track, arc, smoothstep(0.55, 1.0, sweep));
    float3 bg = float3(0.10);
    return float4(mix(bg, col, ring), 1.0);
}
"#;

/// Metaballs: five blobs merge as they drift; the pointer is a sixth.
const METABALLS: &str = r#"
float4 effect(float2 uv, constant EffectUniforms &u) {
    float2 p = uv * u.resolution / u.scale;
    float2 s = u.resolution / u.scale;
    float field = 0.0;
    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        float2 c = s * 0.5 + float2(sin(u.time * 0.7 + fi * 1.7), cos(u.time * 0.9 + fi * 2.3)) * s * 0.3;
        field += 900.0 / dot(p - c, p - c);
    }
    if (u.has_pointer > 0.5) {
        float2 c = u.pointer / u.scale;
        field += 1400.0 / dot(p - c, p - c);
    }
    float edge = smoothstep(0.9, 1.1, field);
    float3 col = mix(float3(0.08, 0.02, 0.12), float3(1.0, 0.45, 0.2) * (0.6 + 0.4 * sin(field)), edge);
    return float4(col, 1.0);
}
"#;

struct Gallery {
    started: Instant,
    tiles: Vec<(&'static str, Effect)>,
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let t = self.started.elapsed();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .size_full()
            .bg(rgb(0x161616))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .text_lg()
                    .child("Four fragment functions, laid out by GPUI"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .children(self.tiles.iter().map(|(label, effect)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(300.))
                                    .h(px(200.))
                                    .rounded_lg()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(rgb(0x333333))
                                    .child(effect.element(t).size_full()),
                            )
                            .child(div().text_sm().text_color(rgb(0x999999)).child(*label))
                    })),
            )
    }
}

/// Print shader compile errors from the crate's `log::error!` to stderr.
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
        let bounds = Bounds::centered(None, size(px(680.), px(560.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Gallery {
                    started: Instant::now(),
                    tiles: vec![
                        ("ripple: follows the pointer", Effect::new(RIPPLE)),
                        ("clouds: fractal noise, no texture", Effect::new(CLOUDS)),
                        ("ring: anti-aliased loader", Effect::new(RING)),
                        ("metaballs: pointer is a blob", Effect::new(METABALLS)),
                    ],
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
