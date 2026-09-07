//! The effect element takes the size it is given and registers a hitbox
//! there, so pointer input reaches the shader. The Metal side needs a real
//! device and is checked by running the `plasma` example.

use gangplank_gpu::Effect;
use gpui::{Bounds, Context, Render, TestAppContext, Window, div, point, prelude::*, px, size};
use std::time::Duration;

struct Probe;

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui::test]
fn element_lays_out_at_requested_size(cx: &mut TestAppContext) {
    let (_probe, cx) = cx.add_window_view(|_, _| Probe);
    let effect = Effect::new(
        "float4 effect(float2 uv, constant EffectUniforms &u) { return float4(uv, 0, 1); }",
    );

    let (_, hitbox) = cx.draw(point(px(10.), px(20.)), size(px(400.), px(400.)), |_, _| {
        effect
            .element(Duration::from_secs(1))
            .w(px(200.))
            .h(px(100.))
    });

    assert_eq!(
        hitbox.bounds,
        Bounds::new(point(px(10.), px(20.)), size(px(200.), px(100.)))
    );
}

#[test]
fn uniform_block_is_sixteen_byte_aligned() {
    // Metal pads float2/float3 members; the Rust struct must match the MSL layout.
    assert_eq!(std::mem::size_of::<gangplank_gpu::EffectUniforms>(), 64);
}
