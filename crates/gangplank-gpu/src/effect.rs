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
    /// Corner radii in device pixels: top-left, top-right, bottom-right,
    /// bottom-left. The wrapper fades the effect out past them.
    pub corner_radii: [f32; 4],
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
            corner_radii: [
                frame.corner_radii.top_left,
                frame.corner_radii.top_right,
                frame.corner_radii.bottom_right,
                frame.corner_radii.bottom_left,
            ],
        }
    }
}

/// Metal source that precedes the user's `effect` function. Must stay in
/// step with [`EffectUniforms`].
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
    // Three scalars, not float3: float3 aligns to 16 and would push
    // corner_radii past the 64 bytes Rust uploads.
    float _pad0;
    float _pad1;
    float _pad2;
    float4 corner_radii;
};

// What GPUI drew under the element, when the element asked for it with
// `.backdrop(margin)`. Without that it is a 1x1 transparent texture.
struct EffectBackdrop {
    texture2d<float> texture;
    sampler s;
    float2 viewport;    // drawable size, device pixels
    float2 origin;      // element origin in the drawable, device pixels
    float2 resolution;  // element size, device pixels
    // Sample at element uv, 0..1 with a top-left origin. Values past the
    // requested margin are undefined: the copy is regional and the texture
    // is never cleared.
    float4 sample(float2 uv) {
        return texture.sample(s, (origin + uv * resolution) / viewport);
    }
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

// Coverage of a rounded rectangle at pixel `p`: 1 inside, 0 outside, with a
// one-pixel ramp at the edge. Radii order matches EffectUniforms.
float effect_corner_coverage(float2 p, float2 size, float4 radii) {
    float2 half_size = size * 0.5;
    float2 q = p - half_size;
    float r = q.x < 0.0 ? (q.y < 0.0 ? radii.x : radii.w)
                        : (q.y < 0.0 ? radii.y : radii.z);
    float2 d = abs(q) - half_size + r;
    float sdf = length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - r;
    return saturate(0.5 - sdf);
}
"#;

/// The fragment entry, after the user's source so it can call `effect`.
/// `EffectBackdrop` in the source selects the three-argument form.
fn fragment_entry(source: &str) -> String {
    let call = if source.contains("EffectBackdrop") {
        "effect(in.uv, u, backdrop)"
    } else {
        "effect(in.uv, u)"
    };
    format!(
        r#"
fragment float4 effect_fragment(EffectVertex in [[stage_in]],
                                constant EffectUniforms &u [[buffer(0)]],
                                constant float2 &viewport [[buffer(1)]],
                                texture2d<float> backdrop_texture [[texture(0)]],
                                sampler backdrop_sampler [[sampler(0)]]) {{
    EffectBackdrop backdrop {{ backdrop_texture, backdrop_sampler, viewport, u.origin, u.resolution }};
    float4 color = {call};
    if (any(u.corner_radii > 0.0)) {{
        color *= effect_corner_coverage(in.uv * u.resolution, u.resolution, u.corner_radii);
    }}
    return color;
}}
"#
    )
}

struct Ready {
    pipeline: metal::RenderPipelineState,
    sampler: metal::SamplerState,
    /// Bound when the frame carries no backdrop, so sampling stays defined.
    blank: metal::Texture,
}

enum Pipeline {
    Pending,
    Ready(Ready),
    Failed,
}

struct Inner {
    source: String,
    pipeline: Pipeline,
    /// The shader reads the pointer, so the element follows it.
    reads_pointer: bool,
}

/// One fragment shader, drawn as a GPUI element. Cheap to clone; clones share
/// the compiled pipeline. Style the element with `.rounded_*()` and the
/// effect fades out past the corners; call `.animate()` on it when the
/// shader depends on time.
///
/// The source must define
/// `float4 effect(float2 uv, constant EffectUniforms &u)`. `uv` runs 0..1
/// across the element, top-left origin. The result is premultiplied alpha,
/// composited over whatever GPUI drew below; return alpha 1 for opaque.
///
/// To read what GPUI drew under the element, define
/// `float4 effect(float2 uv, constant EffectUniforms &u, EffectBackdrop backdrop)`
/// instead and call `.backdrop(margin)` on the element. `backdrop.sample(uv)`
/// returns the pixel under element uv; `margin` is how far past the element
/// the copy reaches, for samples near the edge.
#[derive(Clone)]
pub struct Effect(Rc<RefCell<Inner>>);

impl Effect {
    /// Wrap Metal source. Compilation happens on the first frame; a compile
    /// error is logged once and the element then draws nothing.
    pub fn new(source: impl Into<String>) -> Self {
        Effect(Rc::new(RefCell::new(Inner {
            source: source.into(),
            pipeline: Pipeline::Pending,
            reads_pointer: false,
        })))
    }

    /// Wrap a Shadertoy or Ghostty style GLSL shader. See [`crate::glsl_to_msl`]
    /// for what is and is not translated. A shader that mentions `iMouse`
    /// gets `.follow_pointer()` on its element without being asked.
    pub fn shadertoy(glsl: &str) -> Self {
        let effect = Self::new(crate::glsl_to_msl(glsl));
        effect.0.borrow_mut().reads_pointer = glsl.contains("iMouse");
        effect
    }

    /// The element for this frame. `time` is whatever clock the caller owns.
    /// Call `.follow_pointer()` on it when an MSL shader reads `u.pointer`.
    pub fn element(&self, time: Duration) -> GpuCanvas {
        let canvas = gpu_canvas(self.0.clone()).time(time);
        if self.0.borrow().reads_pointer {
            canvas.follow_pointer()
        } else {
            canvas
        }
    }

    #[cfg(test)]
    fn reads_pointer(&self) -> bool {
        self.0.borrow().reads_pointer
    }
}

impl Inner {
    fn pipeline(&mut self, frame: &GpuCanvasFrame<'_>) -> Option<&Ready> {
        if let Pipeline::Pending = self.pipeline {
            self.pipeline = match build_pipeline(&self.source, frame) {
                Ok(ready) => Pipeline::Ready(ready),
                Err(err) => {
                    log::error!("gangplank-gpu: effect failed to compile: {err}");
                    Pipeline::Failed
                }
            };
        }
        match &self.pipeline {
            Pipeline::Ready(ready) => Some(ready),
            _ => None,
        }
    }
}

fn build_pipeline(source: &str, frame: &GpuCanvasFrame<'_>) -> Result<Ready, String> {
    let full = format!("{PREAMBLE}\n{source}\n{}", fragment_entry(source));
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
    let pipeline = frame.device.new_render_pipeline_state(&descriptor)?;

    let sampler = metal::SamplerDescriptor::new();
    sampler.set_min_filter(metal::MTLSamplerMinMagFilter::Linear);
    sampler.set_mag_filter(metal::MTLSamplerMinMagFilter::Linear);
    sampler.set_address_mode_s(metal::MTLSamplerAddressMode::ClampToEdge);
    sampler.set_address_mode_t(metal::MTLSamplerAddressMode::ClampToEdge);
    let sampler = frame.device.new_sampler(&sampler);

    let blank = metal::TextureDescriptor::new();
    blank.set_width(1);
    blank.set_height(1);
    blank.set_pixel_format(frame.color_format);
    blank.set_usage(metal::MTLTextureUsage::ShaderRead);
    let blank = frame.device.new_texture(&blank);
    let zero = [0u8; 4];
    blank.replace_region(
        metal::MTLRegion::new_2d(0, 0, 1, 1),
        0,
        zero.as_ptr() as *const _,
        4,
    );

    Ok(Ready {
        pipeline,
        sampler,
        blank,
    })
}

impl GpuCanvasRenderer for Inner {
    fn render(&mut self, frame: &mut GpuCanvasFrame<'_>) {
        let Some(ready) = self.pipeline(frame) else {
            return;
        };
        let uniforms = EffectUniforms::from_frame(frame);
        let viewport = [
            frame.viewport_size.width.0 as f32,
            frame.viewport_size.height.0 as f32,
        ];

        let encoder = frame.encoder;
        encoder.set_render_pipeline_state(&ready.pipeline);
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
        encoder.set_fragment_bytes(
            1,
            std::mem::size_of_val(&viewport) as u64,
            viewport.as_ptr() as *const _,
        );
        encoder.set_fragment_texture(0, Some(frame.backdrop.unwrap_or(&ready.blank)));
        encoder.set_fragment_sampler_state(0, Some(&ready.sampler));
        encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    /// Compile the preamble and ask Metal where it put each member of
    /// `EffectUniforms`. A size pin alone let a `float3` alignment bug
    /// through: the struct was still 64 bytes while `corner_radii` had moved.
    #[test]
    fn metal_uniform_offsets_match_rust() {
        let Some(device) = metal::Device::system_default() else {
            eprintln!("no Metal device; skipping");
            return;
        };
        let effect =
            "float4 effect(float2 uv, constant EffectUniforms &u) { return float4(uv, 0, 1); }";
        let source = format!("{PREAMBLE}\n{effect}\n{}", fragment_entry(effect));
        let library = device
            .new_library_with_source(&source, &metal::CompileOptions::new())
            .expect("preamble compiles");
        let descriptor = metal::RenderPipelineDescriptor::new();
        descriptor.set_vertex_function(Some(&library.get_function("effect_vertex", None).unwrap()));
        descriptor.set_fragment_function(Some(
            &library.get_function("effect_fragment", None).unwrap(),
        ));
        descriptor
            .color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        let (_, reflection) = device
            .new_render_pipeline_state_with_reflection(
                &descriptor,
                metal::MTLPipelineOption::ArgumentInfo | metal::MTLPipelineOption::BufferTypeInfo,
            )
            .expect("pipeline builds");

        let arguments = reflection.fragment_arguments();
        let uniforms = (0..arguments.count())
            .filter_map(|i| arguments.object_at(i))
            .find(|a| a.name() == "u")
            .expect("fragment argument `u`");
        assert_eq!(
            uniforms.buffer_data_size() as usize,
            std::mem::size_of::<EffectUniforms>()
        );

        let members = uniforms.buffer_struct_type().members();
        let metal_offset = |name: &str| -> usize {
            (0..members.count())
                .filter_map(|i| members.object_at(i))
                .find(|m| m.name() == name)
                .unwrap_or_else(|| panic!("MSL struct has no member `{name}`"))
                .offset() as usize
        };
        let expected = [
            ("resolution", offset_of!(EffectUniforms, resolution)),
            ("pointer", offset_of!(EffectUniforms, pointer)),
            ("origin", offset_of!(EffectUniforms, origin)),
            ("time", offset_of!(EffectUniforms, time)),
            ("scale", offset_of!(EffectUniforms, scale)),
            ("has_pointer", offset_of!(EffectUniforms, has_pointer)),
            ("_pad0", offset_of!(EffectUniforms, _pad)),
            ("corner_radii", offset_of!(EffectUniforms, corner_radii)),
        ];
        for (name, rust_offset) in expected {
            assert_eq!(metal_offset(name), rust_offset, "offset of `{name}`");
        }
    }

    /// GLSL that reads `iMouse` follows the pointer; MSL never does by itself.
    #[test]
    fn shadertoy_follows_pointer_only_when_it_reads_imouse() {
        let reads = "void mainImage(out vec4 c, in vec2 p) { c = vec4(iMouse.x); }";
        let ignores = "void mainImage(out vec4 c, in vec2 p) { c = vec4(iTime); }";
        assert!(Effect::shadertoy(reads).reads_pointer());
        assert!(!Effect::shadertoy(ignores).reads_pointer());
        assert!(!Effect::new("float4 effect(float2 uv, constant EffectUniforms &u) { return float4(u.pointer, 0, 1); }").reads_pointer());
    }

    /// The three-argument form compiles and links against the wrapper.
    #[test]
    fn backdrop_effect_compiles() {
        let Some(device) = metal::Device::system_default() else {
            eprintln!("no Metal device; skipping");
            return;
        };
        let effect = "float4 effect(float2 uv, constant EffectUniforms &u, EffectBackdrop b) { return b.sample(uv); }";
        assert!(fragment_entry(effect).contains("effect(in.uv, u, backdrop)"));
        let source = format!("{PREAMBLE}\n{effect}\n{}", fragment_entry(effect));
        let library = device
            .new_library_with_source(&source, &metal::CompileOptions::new())
            .expect("backdrop preamble compiles");
        library.get_function("effect_fragment", None).unwrap();
    }

    /// The CRT demo goes GLSL -> MSL and samples iChannel0; a translation
    /// slip would otherwise show up only in the example's log.
    #[test]
    fn translated_crt_shader_compiles() {
        let Some(device) = metal::Device::system_default() else {
            eprintln!("no Metal device; skipping");
            return;
        };
        let effect = crate::glsl_to_msl(include_str!("../examples/shaders/crt.glsl"));
        let source = format!("{PREAMBLE}\n{effect}\n{}", fragment_entry(&effect));
        device
            .new_library_with_source(&source, &metal::CompileOptions::new())
            .expect("crt.glsl compiles");
    }
}
