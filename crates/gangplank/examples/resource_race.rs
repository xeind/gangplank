//! Stage 1 verification: a superseded result must never be written.
//!
//! Even keys load slowly, odd keys load fast, so advancing the key always
//! leaves a slower task in flight behind a faster one. Click "next key" and
//! watch for two seconds. The verdict line must never read MISMATCH.

use gpui::{
    App, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb,
    size,
};
use gangplank::{Resource, use_resource};
use std::time::Duration;

/// Chosen so a slow task always outlives the fast task that supersedes it.
const SLOW: Duration = Duration::from_millis(1500);
const FAST: Duration = Duration::from_millis(200);

struct RaceExample {
    key: u32,
}

impl Render for RaceExample {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.key;
        let executor = cx.background_executor().clone();

        let resource = use_resource(window, cx, key, move |k: u32| async move {
            executor.timer(if k.is_multiple_of(2) { SLOW } else { FAST }).await;
            k
        });

        let value = resource.read(cx).value().clone();

        let (verdict, verdict_color) = match &value {
            Resource::Loading => ("loading…".to_string(), rgb(0x9e9e9e)),
            Resource::Ready(got) if *got == key => (format!("match (key {got})"), rgb(0x4caf50)),
            Resource::Ready(got) => (
                format!("MISMATCH — showing {got}, current key is {key}"),
                rgb(0xf44336),
            ),
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .size_full()
            .p_8()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xe0e0e0))
            .child(div().text_2xl().child("use_resource — race check"))
            .child(div().text_sm().text_color(rgb(0x9e9e9e)).child(
                "Even keys take 1500ms, odd keys 200ms. Click fast, then wait 2s.",
            ))
            .child(div().text_xl().child(format!("current key: {key}")))
            .child(div().text_xl().text_color(verdict_color).child(verdict))
            .child(
                div()
                    .id("next-key")
                    .px_4()
                    .py_2()
                    .w(px(140.))
                    .rounded_md()
                    .bg(rgb(0x1976d2))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x2286e2)))
                    .child("next key")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.key += 1;
                        cx.notify();
                    })),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(360.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| RaceExample { key: 0 }),
        )
        .expect("failed to open window");

        cx.activate(true);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
