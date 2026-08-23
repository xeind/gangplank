//! One text field. Run with `cargo run --example text_input`.
//!
//! Type, select with Shift+arrows or the mouse, ⌘A/⌘C/⌘X/⌘V. Enter submits
//! the line to the list below and clears the field; Esc clears it. To check
//! IME, switch the macOS input source to Japanese or Pinyin and type: the
//! composition shows underlined, the candidate window sits under the cursor,
//! and the chosen text commits in place.

use gangplank::{text_input, use_text_input};
use gpui::{
    App, Application, Bounds, Context, FocusHandle, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, rgb, size,
};

struct Demo {
    focus: FocusHandle,
    submitted: Vec<String>,
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = use_text_input(window, cx);
        let this = cx.entity();
        let field = text_input(&input, &self.focus)
            .placeholder("type a command, Enter to run")
            .on_submit({
                let input = input.clone();
                move |text, _, cx| {
                    let text = text.to_string();
                    this.update(cx, |d, cx| {
                        d.submitted.push(text);
                        cx.notify();
                    });
                    input.update(cx, |t, cx| t.set_text("", cx));
                }
            })
            .on_cancel({
                let input = input.clone();
                move |_, cx| input.update(cx, |t, cx| t.set_text("", cx))
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xe0e0e0))
            .font_family("Menlo")
            .text_size(px(14.))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(rgb(0x555555))
                    .rounded_sm()
                    .overflow_hidden()
                    .child(":")
                    .child(div().flex_1().child(field)),
            )
            .children(self.submitted.iter().rev().map(|s| div().text_color(rgb(0x9a9a9a)).child(format!("ran: {s}"))))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(300.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |window, cx| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                cx.new(|_| Demo { focus, submitted: Vec::new() })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
