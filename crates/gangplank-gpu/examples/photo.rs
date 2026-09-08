//! Photo Lab: a fragment shader filtering an app-supplied image.
//! Run with `cargo run --example photo [path/to/image]`. Without a path it
//! filters a procedural colour wheel. Click the image to cycle the filter.
//!
//! gpui-ce 0.3 has no slider, so `Bar` is a div that maps pointer x to 0..1.

use gangplank_gpu::{Effect, Param};
use gpui::{
    App, Application, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Render,
    Window, WindowBounds, WindowOptions, canvas, div, prelude::*, px, relative, rgb, size,
};
use std::{cell::Cell, rc::Rc, time::Duration};

const FILTER: &str = r#"
float4 effect(float2 uv, constant EffectUniforms &u, constant EffectParams &p, EffectImages images) {
    // Fit the image inside the element, preserving aspect ratio.
    float2 img = images.size(0);
    float2 fit = min(u.resolution / img, min(u.resolution.x / img.x, u.resolution.y / img.y));
    float2 shown = img * fit;
    float2 iuv = (uv * u.resolution - (u.resolution - shown) * 0.5) / shown;
    if (any(iuv < 0.0) || any(iuv > 1.0)) return float4(0.08, 0.08, 0.08, 1.0);

    if (p.mode == 3) {
        float cells = mix(400.0, 12.0, p.amount);
        iuv = (floor(iuv * cells) + 0.5) / cells;
    }
    float4 c = images.sample(0, iuv);
    if (p.mode == 0) {
        c.rgb = float3(dot(c.rgb, float3(0.299, 0.587, 0.114)));
    } else if (p.mode == 1) {
        c.rgb = c.a - c.rgb;
    } else if (p.mode == 2) {
        float2 off = float2(p.amount * 0.05, 0.0);
        c.r = images.sample(0, iuv - off).r;
        c.b = images.sample(0, iuv + off).b;
    }
    return c;
}
"#;

const MODES: [&str; 4] = ["grayscale", "invert", "aberration", "pixelate"];

/// Premultiplied BGRA8 from RGBA8.
fn premultiply_bgra(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|px| {
            let a = px[3] as u32;
            let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
            [m(px[2]), m(px[1]), m(px[0]), px[3]]
        })
        .collect()
}

/// A 512x512 colour wheel over a checkerboard, alpha 1, as RGBA8.
fn test_image() -> (u32, u32, Vec<u8>) {
    const N: u32 = 512;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 / N as f32 - 0.5, y as f32 / N as f32 - 0.5);
            let r = (dx * dx + dy * dy).sqrt() * 2.0;
            let h = dy.atan2(dx) / std::f32::consts::TAU + 0.5;
            let k = |n: f32| {
                let t = (n + h * 6.0) % 6.0;
                1.0 - t.min(4.0 - t).clamp(0.0, 1.0)
            };
            let check = if ((x / 32) + (y / 32)) % 2 == 0 {
                0.9
            } else {
                0.5
            };
            let s = if r < 1.0 { r } else { 0.0 };
            let mix = |c: f32| ((c * s + check * (1.0 - s)) * 255.0) as u8;
            rgba.extend([mix(k(5.0)), mix(k(3.0)), mix(k(1.0)), 255]);
        }
    }
    (N, N, rgba)
}

type OnChange = Box<dyn Fn(f32, &mut App)>;

struct Bar {
    label: &'static str,
    value: f32,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    on_change: OnChange,
}

impl Bar {
    fn set_from_x(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let b = self.bounds.get();
        self.value = (f32::from(x - b.left()) / f32::from(b.size.width)).clamp(0.0, 1.0);
        (self.on_change)(self.value, cx);
        cx.notify();
    }
}

impl Render for Bar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().w(px(80.)).text_sm().child(self.label))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h(px(24.))
                    .rounded_md()
                    .bg(rgb(0x333333))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, _, cx| {
                            this.set_from_x(e.position.x, cx)
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                        if e.pressed_button == Some(MouseButton::Left) {
                            this.set_from_x(e.position.x, cx);
                        }
                    }))
                    .child(
                        canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .child(div().h_full().w(relative(self.value)).bg(rgb(0x8888ff))),
            )
    }
}

struct Demo {
    filter: Effect,
    mode: i32,
    amount: gpui::Entity<Bar>,
}

impl Demo {
    fn new(image: (u32, u32, Vec<u8>), cx: &mut Context<Self>) -> Self {
        let filter = Effect::new(FILTER)
            .params(&[("amount", Param::Float), ("mode", Param::Int)])
            .expect("params");
        let (w, h, rgba) = image;
        filter
            .image(0, w, h, premultiply_bgra(&rgba))
            .expect("image");
        filter.set("amount", 0.5).expect("set amount");
        filter.set("mode", 0).expect("set mode");
        let weak = cx.weak_entity();
        let amount = cx.new(|_| Bar {
            label: "amount",
            value: 0.5,
            bounds: Rc::default(),
            on_change: Box::new(move |v, cx| {
                weak.update(cx, |this: &mut Demo, cx| {
                    this.filter.set("amount", v).expect("set amount");
                    cx.notify();
                })
                .ok();
            }),
        });
        Demo {
            filter,
            mode: 0,
            amount,
        }
    }

    fn cycle_mode(&mut self, cx: &mut Context<Self>) {
        self.mode = (self.mode + 1) % MODES.len() as i32;
        self.filter.set("mode", self.mode).expect("set mode");
        cx.notify();
    }
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(div().text_xl().child("gangplank-gpu: Photo Lab"))
            .child(
                div()
                    .flex_1()
                    .rounded_lg()
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| this.cycle_mode(cx)),
                    )
                    .child(self.filter.element(Duration::ZERO).size_full()),
            )
            .child(div().text_sm().text_color(rgb(0xaaaaaa)).child(format!(
                "mode: {}. Click the image to change it.",
                MODES[self.mode as usize]
            )))
            .child(self.amount.clone())
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
    let image = match std::env::args().nth(1) {
        Some(path) => {
            let img = image::open(&path).expect("decode image").into_rgba8();
            (img.width(), img.height(), img.into_raw())
        }
        None => test_image(),
    };
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(560.), px(560.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| Demo::new(image, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
