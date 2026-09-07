use gpui::{GpuCanvas, GpuCanvasFrame, GpuCanvasRenderer, gpu_canvas, metal};
use std::{cell::RefCell, rc::Rc, time::Duration};

/// The uniform block every effect receives as `constant EffectUniforms &u`.
///
/// Distances are device pixels. `pointer` is relative to the element's
/// top-left and only meaningful when `has_pointer` is `1.0`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectUniforms {
    /// Element size in device pixels.
    pub resolution: [f32; 2],
    /// Pointer position relative to the element, device pixels.
    pub pointer: [f32; 2],
    /// Element origin in the drawable, device pixels.
    pub origin: [f32; 2],
    /// Seconds, as handed to [`Effect::element`].
    pub time: f32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// `1.0` while the pointer is over the element, else `0.0`.
    pub has_pointer: f32,
    _pad: [f32; 3],
}

impl EffectUniforms {
    fn from_frame(frame: &GpuCanvasFrame<'_>) -> Self {
        let origin = [
            frame.bounds.origin.x.0 as f32,
            frame.bounds.origin.y.0 as f32,
        ];
        let pointer = frame
            .pointer_position
            .map(|p| [p.x.0 as f32 - origin[0], p.y.0 as f32 - origin[1]]);
        Self {
            resolution: [
                frame.bounds.size.width.0 as f32,
                frame.bounds.size.height.0 as f32,
            ],
            pointer: pointer.unwrap_or([0., 0.]),
            origin,
            time: frame.time.as_secs_f32(),
            scale: frame.scale_factor,
            has_pointer: if pointer.is_some() { 1. } else { 0. },
            _pad: [0.; 3],
        }
    }
}

/// Metal source that wraps the user's `effect` function. Must stay in step
/// with [`EffectUniforms`].
const PREAMBLE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct EffectUniforms {
    float2 resolution;
    float2 pointer;
    float2 origin;
    float time;
    float scale;
    float has_pointer;
    float3 _pad;
};

struct EffectVertex {
    float4 position [[position]];
    float2 uv;
};

vertex EffectVertex effect_vertex(uint vid [[vertex_id]],
                                  constant EffectUniforms &u [[buffer(0)]],
                                  constant float2 &viewport [[buffer(1)]]) {
    const float2 corners[6] = { {0, 0}, {1, 0}, {0, 1}, {0, 1}, {1, 0}, {1, 1} };
    float2 uv = corners[vid];
    float2 pixel = u.origin + uv * u.resolution;
    float2 ndc = pixel / viewport * 2.0 - 1.0;
    ndc.y = -ndc.y;
    EffectVertex out;
    out.position = float4(ndc, 0.0, 1.0);
    out.uv = uv;
    return out;
}

float4 effect(float2 uv, constant EffectUniforms &u);

fragment float4 effect_fragment(EffectVertex in [[stage_in]],
                                constant EffectUniforms &u [[buffer(0)]]) {
    return effect(in.uv, u);
}
"#;

enum Pipeline {
    Pending,
    Ready(metal::RenderPipelineState),
    Failed,
}

struct Inner {
    source: String,
    pipeline: Pipeline,
}

/// One fragment shader, drawn as a GPUI element. Cheap to clone; clones share
/// the compiled pipeline.
///
/// The source must define
/// `float4 effect(float2 uv, constant EffectUniforms &u)`. `uv` runs 0..1
/// across the element, top-left origin. The result is premultiplied alpha,
/// composited over whatever GPUI drew below; return alpha 1 for opaque.
#[derive(Clone)]
pub struct Effect(Rc<RefCell<Inner>>);

impl Effect {
    /// Wrap Metal source. Compilation happens on the first frame; a compile
    /// error is logged once and the element then draws nothing.
    pub fn new(source: impl Into<String>) -> Self {
        Effect(Rc::new(RefCell::new(Inner {
            source: source.into(),
            pipeline: Pipeline::Pending,
        })))
    }

    /// Wrap a Shadertoy or Ghostty style GLSL shader. See [`crate::glsl_to_msl`]
    /// for what is and is not translated.
    pub fn shadertoy(glsl: &str) -> Self {
        Self::new(crate::glsl_to_msl(glsl))
    }

    /// The element for this frame. `time` is whatever clock the caller owns.
    pub fn element(&self, time: Duration) -> GpuCanvas {
        gpu_canvas(self.0.clone()).time(time)
    }
}

impl Inner {
    fn pipeline(&mut self, frame: &GpuCanvasFrame<'_>) -> Option<metal::RenderPipelineState> {
        if let Pipeline::Pending = self.pipeline {
            self.pipeline = match build_pipeline(&self.source, frame) {
                Ok(state) => Pipeline::Ready(state),
                Err(err) => {
                    log::error!("gangplank-gpu: effect failed to compile: {err}");
                    Pipeline::Failed
                }
            };
        }
        match &self.pipeline {
            Pipeline::Ready(state) => Some(state.clone()),
            _ => None,
        }
    }
}

fn build_pipeline(
    source: &str,
    frame: &GpuCanvasFrame<'_>,
) -> Result<metal::RenderPipelineState, String> {
    let full = format!("{PREAMBLE}\n{source}");
    let library = frame
        .device
        .new_library_with_source(&full, &metal::CompileOptions::new())?;
    let vertex = library.get_function("effect_vertex", None)?;
    let fragment = library.get_function("effect_fragment", None)?;
    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_vertex_function(Some(&vertex));
    descriptor.set_fragment_function(Some(&fragment));
    let color = descriptor
        .color_attachments()
        .object_at(0)
        .ok_or("no color attachment slot")?;
    color.set_pixel_format(frame.color_format);
    // Source-over with premultiplied alpha, same as GPUI's own quads, so an
    // effect that returns alpha < 1 composites over what is already drawn.
    color.set_blending_enabled(true);
    color.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    frame.device.new_render_pipeline_state(&descriptor)
}

impl GpuCanvasRenderer for Inner {
    fn render(&mut self, frame: &mut GpuCanvasFrame<'_>) {
        let Some(pipeline) = self.pipeline(frame) else {
            return;
        };
        let uniforms = EffectUniforms::from_frame(frame);
        let viewport = [
            frame.viewport_size.width.0 as f32,
            frame.viewport_size.height.0 as f32,
        ];

        let encoder = frame.encoder;
        encoder.set_render_pipeline_state(&pipeline);
        encoder.set_vertex_bytes(
            0,
            std::mem::size_of::<EffectUniforms>() as u64,
            &uniforms as *const EffectUniforms as *const _,
        );
        encoder.set_vertex_bytes(
            1,
            std::mem::size_of_val(&viewport) as u64,
            viewport.as_ptr() as *const _,
        );
        encoder.set_fragment_bytes(
            0,
            std::mem::size_of::<EffectUniforms>() as u64,
            &uniforms as *const EffectUniforms as *const _,
        );
        encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
    }
}
