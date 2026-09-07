//! Filter Mixer: two bars drive named params on a shader. Run with `cargo run --example mixer`.
//!
//! gpui-ce 0.3 has no slider, so `Bar` is a div that maps pointer x to 0..1.

use gangplank_gpu::{Effect, Param};
use gpui::{
    App, Application, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Render,
    Window, WindowBounds, WindowOptions, canvas, div, prelude::*, px, relative, rgb, size,
};
use std::{cell::Cell, rc::Rc, time::Instant};

const PLASMA: &str = r#"
float4 effect(float2 uv, constant EffectUniforms &u, constant EffectParams &p) {
    uv += p.distortion * 0.2 * sin(uv.yx * 10.0 + u.time);
    float wave = sin(uv.x * 6.0 + u.time) + cos(uv.y * 5.0 - u.time * 0.7);
    float3 color = 0.5 + 0.5 * cos(u.time * 0.5 + wave + uv.xyx * 2.0 + float3(0.0, 2.0, 4.0));
    return float4(color, 1.0) * p.tint;
}
"#;

/// Hue 0..1 at full saturation and value, alpha 1.
fn hue_to_rgba(h: f32) -> [f32; 4] {
    let k = |n: f32| {
        let t = (n + h * 6.0) % 6.0;
        1.0 - t.min(4.0 - t).clamp(0.0, 1.0)
    };
    [k(5.0), k(3.0), k(1.0), 1.0]
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
    started: Instant,
    plasma: Effect,
    bars: Vec<gpui::Entity<Bar>>,
}

impl Demo {
    fn new(cx: &mut Context<Self>) -> Self {
        let plasma = Effect::new(PLASMA)
            .params(&[("tint", Param::Float4), ("distortion", Param::Float)])
            .expect("params");
        let bar = |label, value: f32, cx: &mut Context<Self>| {
            let weak = cx.weak_entity();
            let apply = move |v: f32, cx: &mut App| {
                weak.update(cx, |this: &mut Demo, cx| {
                    match label {
                        "tint" => this.plasma.set("tint", hue_to_rgba(v)),
                        _ => this.plasma.set("distortion", v),
                    }
                    .expect("set param");
                    cx.notify();
                })
                .ok();
            };
            cx.new(|_| Bar {
                label,
                value,
                bounds: Rc::default(),
                on_change: Box::new(apply),
            })
        };
        let bars = vec![bar("tint", 0.6, cx), bar("distortion", 0.0, cx)];
        plasma.set("tint", hue_to_rgba(0.6)).expect("set tint");
        plasma.set("distortion", 0.0).expect("set distortion");
        Demo {
            started: Instant::now(),
            plasma,
            bars,
        }
    }
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(div().text_xl().child("gangplank-gpu: Filter Mixer"))
            .child(
                div().flex_1().rounded_lg().overflow_hidden().child(
                    self.plasma
                        .element(self.started.elapsed())
                        .animate()
                        .size_full(),
                ),
            )
            .children(self.bars.iter().cloned())
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
        let bounds = Bounds::centered(None, size(px(520.), px(460.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(Demo::new),
        )
        .unwrap();
        cx.activate(true);
    });
}
