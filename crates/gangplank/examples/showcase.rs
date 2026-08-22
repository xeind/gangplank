//! Every hook in one window. Run with `cargo run --example showcase`.
//!
//! Type (any letter, backspace): the query debounces, then a fake search runs
//! for the settled text. ⌘K clears it. ⌘C copies the query and shows "copied"
//! for two seconds. The clock ticks once a second. The query survives a
//! relaunch. Save `/tmp/gangplank-watched.txt` and the file counter bumps.

use gangplank::{
    Resource, data_dir, use_clipboard, use_debounce, use_file_watch, use_interval, use_keyboard,
    use_persisted, use_previous, use_resource,
};
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::time::Duration;

const WATCHED: &str = "/tmp/gangplank-watched.txt";

struct Showcase {
    query: String,
}

impl Render for Showcase {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // use_persisted: the query comes back after relaunch.
        let saved = use_persisted(window, cx, data_dir("dev.gangplank.showcase").join("query.json"), String::new);
        if self.query.is_empty() && use_previous(window, cx, ()).is_none() {
            self.query = saved.read(cx).value().clone();
        }

        // use_debounce → use_resource: search once, for the settled text.
        let settled = use_debounce(window, cx, self.query.clone(), Duration::from_millis(300));
        let settled = settled.read(cx).value().clone();
        let executor = cx.background_executor().clone();
        let hits = use_resource(window, cx, settled.clone(), move |q: String| async move {
            executor.timer(Duration::from_millis(400)).await;
            if q.is_empty() { 0 } else { q.len() * 7 % 23 }
        });
        let hits = match hits.read(cx).value() {
            Resource::Loading => "searching…".to_string(),
            Resource::Ready(n) => format!("{n} results for {settled:?}"),
        };

        let clipboard = use_clipboard(window, cx, Duration::from_secs(2));
        let ticks = use_interval(window, cx, Duration::from_secs(1)).read(cx).ticks();
        let changes = use_file_watch(window, cx, WATCHED, Duration::from_millis(500)).read(cx).version();

        let saved_for_keys = saved.clone();
        let clip_for_keys = clipboard.clone();
        let keys = use_keyboard(window, cx)
            .bind("cmd-k", cx.listener(move |this, _, _, cx| {
                this.query.clear();
                saved_for_keys.update(cx, |s, cx| s.set(String::new(), cx));
                cx.notify();
            }))
            .bind("cmd-c", cx.listener(move |this, _, _, cx| {
                let q = this.query.clone();
                clip_for_keys.update(cx, |c, cx| c.copy(q, cx));
            }))
            .bind("backspace", cx.listener(move |this, _, _, cx| {
                this.query.pop();
                let q = this.query.clone();
                saved.update(cx, |s, cx| s.set(q, cx));
                cx.notify();
            }));
        // Letters a–z: one binding each, so typing works without a text input.
        let keys = (b'a'..=b'z').fold(keys, |keys, c| {
            let key: &'static str = Box::leak(((c as char).to_string()).into_boxed_str());
            keys.bind(key, cx.listener(move |this, _, _, cx| {
                this.query.push(c as char);
                cx.notify();
            }))
        });

        let row = |label: &str, value: String| {
            div().flex().gap_3().child(div().w(px(110.)).text_color(rgb(0x888888)).child(label.to_string())).child(value)
        };
        let copied = if clipboard.read(cx).copied() { "copied ✓" } else { "⌘C to copy" };

        keys.attach(div())
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_2()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xe0e0e0))
            .font_family("Menlo")
            .text_sm()
            .child(div().text_lg().mb_2().child("gangplank showcase — type letters, ⌘K clear, ⌘C copy"))
            .child(row("query", format!("{:?}", self.query)))
            .child(row("debounced", format!("{settled:?}")))
            .child(row("resource", hits))
            .child(row("clipboard", copied.to_string()))
            .child(row("interval", format!("{ticks} s")))
            .child(row("file_watch", format!("{changes} changes to {WATCHED}")))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(320.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Showcase { query: String::new() }),
        )
        .expect("failed to open window");
        cx.activate(true);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
