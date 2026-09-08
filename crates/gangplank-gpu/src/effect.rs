use crate::params::{Param, ParamError, ParamLayout, ParamValue};
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
    /// Seconds since this effect last drew; `0.0` on its first frame.
    pub delta: f32,
    /// How many frames this effect has drawn before this one.
    pub frame: u32,
    _pad: f32,
    /// Corner radii in device pixels: top-left, top-right, bottom-right,
    /// bottom-left. The wrapper fades the effect out past them.
    pub corner_radii: [f32; 4],
}

impl EffectUniforms {
    fn from_frame(frame: &GpuCanvasFrame<'_>, clock: &Clock) -> Self {
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
            delta: clock.delta.as_secs_f32(),
            frame: clock.frame,
            _pad: 0.,
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
    float delta;        // seconds since this effect last drew, 0 on the first frame
    uint frame;         // frames this effect drew before this one
    float _pad0;
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

// App images, uploaded with `Effect::image(slot, ..)`. An empty slot is a
// 1x1 transparent texture with size (0, 0).
struct EffectImages {
    array<texture2d<float>, 4> textures;
    sampler s;
    constant float4 *sizes;
    // Sample slot `i` at uv, 0..1 with a top-left origin. Wraps.
    float4 sample(int i, float2 uv) {
        return textures[i].sample(s, uv);
    }
    // Slot `i` in pixels, (0, 0) when empty.
    float2 size(int i) {
        return sizes[i].xy;
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
/// Each wrapper type the source mentions adds an argument, in this order:
/// `EffectBackdrop` the backdrop, `EffectParams` the params,
/// `EffectImages` the images.
fn fragment_entry(source: &str) -> String {
    let mut args = vec!["in.uv", "u"];
    for (marker, arg) in [
        ("EffectBackdrop", "backdrop"),
        ("EffectParams", "p"),
        ("EffectImages", "images"),
    ] {
        if source.contains(marker) {
            args.push(arg);
        }
    }
    let call = format!("effect({})", args.join(", "));
    format!(
        r#"
fragment float4 effect_fragment(EffectVertex in [[stage_in]],
                                constant EffectUniforms &u [[buffer(0)]],
                                constant float2 &viewport [[buffer(1)]],
                                constant EffectParams &p [[buffer(2)]],
                                constant float4 *image_sizes [[buffer(3)]],
                                texture2d<float> backdrop_texture [[texture(0)]],
                                array<texture2d<float>, 4> image_textures [[texture(1)]],
                                sampler backdrop_sampler [[sampler(0)]],
                                sampler image_sampler [[sampler(1)]]) {{
    EffectBackdrop backdrop {{ backdrop_texture, backdrop_sampler, viewport, u.origin, u.resolution }};
    EffectImages images {{ image_textures, image_sampler, image_sizes }};
    float4 color = {call};
    if (any(u.corner_radii > 0.0)) {{
        color *= effect_corner_coverage(in.uv * u.resolution, u.resolution, u.corner_radii);
    }}
    return color;
}}
"#
    )
}

/// What the app handed over, in the language it wrote.
enum Source {
    Msl(String),
    Wgsl(String),
}

impl Source {
    /// Whether the shader can see the params, for the `set` warning. MSL
    /// names the type; WGSL reads the global `p`.
    fn reads_params(&self) -> bool {
        static READS_P: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        match self {
            Source::Msl(msl) => msl.contains("EffectParams"),
            Source::Wgsl(wgsl) => READS_P
                .get_or_init(|| regex::Regex::new(r"\bp\.").unwrap())
                .is_match(wgsl),
        }
    }

    /// What the `set` warning calls the params in this language.
    fn params_name(&self) -> &'static str {
        match self {
            Source::Msl(_) => "EffectParams",
            Source::Wgsl(_) => "p",
        }
    }
}

/// What Metal compiles. MSL: preamble, params struct, user source, entry.
/// WGSL: naga's translation of the user source plus the WGSL preamble.
fn full_source(source: &Source, params: &ParamLayout) -> Result<String, String> {
    match source {
        Source::Msl(msl) => Ok(format!(
            "{PREAMBLE}\n{}\n{msl}\n{}",
            params.msl_struct(),
            fragment_entry(msl)
        )),
        Source::Wgsl(wgsl) => crate::wgsl::wgsl_to_msl(wgsl, params),
    }
}

struct Ready {
    pipeline: metal::RenderPipelineState,
    sampler: metal::SamplerState,
    /// Wraps, for tiled image lookups.
    image_sampler: metal::SamplerState,
    /// Bound to every texture slot that has nothing, so sampling stays defined.
    blank: metal::Texture,
}

/// One image slot: bytes from the app, and the texture once uploaded.
/// `texture` is `None` until the next frame after `Effect::image`.
struct Image {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
    texture: Option<metal::Texture>,
}

/// Why `Effect::image` refused an upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// Slots run 0..4.
    BadSlot(usize),
    /// `bgra.len()` must be `width * height * 4`.
    BadLength { expected: usize, got: usize },
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::BadSlot(slot) => write!(f, "image slot {slot} is not in 0..4"),
            ImageError::BadLength { expected, got } => {
                write!(f, "image bytes: expected {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for ImageError {}

/// Image slots the shader can read.
pub const IMAGE_SLOTS: usize = 4;

enum Pipeline {
    Pending,
    Ready(Ready),
    Failed,
}

/// What this effect has drawn so far, for `delta` and `frame`.
#[derive(Default)]
struct Clock {
    last_time: Option<Duration>,
    delta: Duration,
    frame: u32,
}

impl Clock {
    /// Advance to `time`. `delta` is 0 on the first frame and never
    /// negative: a clock that jumps back reads as a fresh start.
    fn tick(&mut self, time: Duration) {
        self.delta = match self.last_time {
            Some(last) => time.saturating_sub(last),
            None => Duration::ZERO,
        };
        self.last_time = Some(time);
    }

    /// The frame just drawn is now in the past.
    fn drew(&mut self) {
        self.frame += 1;
    }
}

struct Inner {
    source: Source,
    clock: Clock,
    pipeline: Pipeline,
    /// The pipeline that drew before `set_source`, kept until the new
    /// source compiles so an edit that fails never blanks the element.
    last_good: Option<Ready>,
    /// App parameters, uploaded at buffer(2) as `EffectParams`.
    params: ParamLayout,
    warned_unread_params: bool,
    images: [Option<Image>; IMAGE_SLOTS],
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
        Self::from_source(Source::Msl(source.into()))
    }

    /// Wrap a WGSL effect: `fn effect(uv: vec2<f32>) -> vec4<f32>` that
    /// reads `u`, `p`, `viewport`, the textures and samplers as globals
    /// (`src/wgsl.rs` declares them). Translated by naga and compiled on
    /// the first frame; a translation error logs once like a compile error.
    pub fn wgsl(source: impl Into<String>) -> Self {
        Self::from_source(Source::Wgsl(source.into()))
    }

    fn from_source(source: Source) -> Self {
        Effect(Rc::new(RefCell::new(Inner {
            source,
            clock: Clock::default(),
            pipeline: Pipeline::Pending,
            last_good: None,
            params: ParamLayout::empty(),
            warned_unread_params: false,
            images: [None, None, None, None],
            reads_pointer: false,
        })))
    }

    /// Declare app parameters, in struct order. The shader receives them as
    /// `constant EffectParams &p`; take it as a third argument, or a fourth
    /// after the backdrop. Call before the first frame.
    pub fn params(self, decls: &[(&str, Param)]) -> Result<Self, ParamError> {
        self.0.borrow_mut().params = ParamLayout::new(decls)?;
        Ok(self)
    }

    /// Write one parameter. The next frame sees it; call `cx.notify()` to
    /// get one. Errors name the problem, so `?` or `expect` at the call.
    pub fn set(&self, name: &str, value: impl Into<ParamValue>) -> Result<(), ParamError> {
        let mut inner = self.0.borrow_mut();
        if !inner.warned_unread_params && !inner.source.reads_params() {
            inner.warned_unread_params = true;
            log::warn!(
                "gangplank-gpu: set(\"{name}\") on an effect whose shader never reads {}",
                inner.source.params_name()
            );
        }
        inner.params.set(name, value.into())
    }

    /// Replace the shader source. It compiles on the next frame; if that
    /// fails, the element keeps drawing the last source that compiled and
    /// the error is logged once. Params and images carry over.
    pub fn set_source(&self, source: impl Into<String>) {
        self.replace_source(Source::Msl(source.into()));
    }

    /// Replace the source with WGSL, as [`Effect::set_source`] does for MSL.
    pub fn set_wgsl(&self, source: impl Into<String>) {
        self.replace_source(Source::Wgsl(source.into()));
    }

    fn replace_source(&self, source: Source) {
        let mut inner = self.0.borrow_mut();
        inner.source = source;
        if let Pipeline::Ready(ready) = std::mem::replace(&mut inner.pipeline, Pipeline::Pending) {
            inner.last_good = Some(ready);
        }
    }

    /// Wrap a Shadertoy or Ghostty style GLSL shader, replacing the source
    /// as [`Effect::set_source`] does. Its `uniform` declarations become the
    /// params, so earlier values reset.
    pub fn set_shadertoy(&self, glsl: &str) {
        self.set_source(crate::glsl_to_msl(glsl));
        let decls = crate::glsl_uniforms(glsl);
        let decls: Vec<(&str, Param)> = decls.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        let mut inner = self.0.borrow_mut();
        inner.params = ParamLayout::new(&decls).unwrap_or_else(|err| {
            log::error!("gangplank-gpu: GLSL uniforms: {err}");
            ParamLayout::empty()
        });
        inner.reads_pointer = glsl.contains("iMouse");
    }

    /// Give the shader an image in `slot` (0..4). `bgra` is premultiplied
    /// BGRA8, `width * height * 4` bytes, the format gpui's `RenderImage`
    /// holds. The upload happens on the next frame; call `cx.notify()` to
    /// get one. The shader takes `EffectImages images` as its last argument
    /// and reads `images.sample(slot, uv)`; GLSL sees `iChannel1..3`.
    pub fn image(
        &self,
        slot: usize,
        width: u32,
        height: u32,
        bgra: Vec<u8>,
    ) -> Result<(), ImageError> {
        if slot >= IMAGE_SLOTS {
            return Err(ImageError::BadSlot(slot));
        }
        let expected = width as usize * height as usize * 4;
        if bgra.len() != expected {
            return Err(ImageError::BadLength {
                expected,
                got: bgra.len(),
            });
        }
        self.0.borrow_mut().images[slot] = Some(Image {
            width,
            height,
            bgra,
            texture: None,
        });
        Ok(())
    }

    /// Wrap a Shadertoy or Ghostty style GLSL shader. See [`crate::glsl_to_msl`]
    /// for what is and is not translated. A shader that mentions `iMouse`
    /// gets `.follow_pointer()` on its element without being asked.
    pub fn shadertoy(glsl: &str) -> Self {
        let effect = Self::new(String::new());
        effect.set_shadertoy(glsl);
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

impl Pipeline {
    /// Build on first use, then hand back the compiled state. A failed
    /// build falls back to `last_good` when there is one.
    fn ready(
        &mut self,
        source: &Source,
        params: &ParamLayout,
        last_good: &mut Option<Ready>,
        frame: &GpuCanvasFrame<'_>,
    ) -> Option<&Ready> {
        if let Pipeline::Pending = self {
            *self = match build_pipeline(source, params, frame) {
                Ok(ready) => {
                    *last_good = None;
                    Pipeline::Ready(ready)
                }
                Err(err) => match last_good.take() {
                    Some(previous) => {
                        log::error!(
                            "gangplank-gpu: effect failed to compile, keeping the last working one: {err}"
                        );
                        Pipeline::Ready(previous)
                    }
                    None => {
                        log::error!("gangplank-gpu: effect failed to compile: {err}");
                        Pipeline::Failed
                    }
                },
            };
        }
        match self {
            Pipeline::Ready(ready) => Some(ready),
            _ => None,
        }
    }
}

fn build_pipeline(
    source: &Source,
    params: &ParamLayout,
    frame: &GpuCanvasFrame<'_>,
) -> Result<Ready, String> {
    let full = full_source(source, params)?;
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
    let image_sampler = metal::SamplerDescriptor::new();
    image_sampler.set_min_filter(metal::MTLSamplerMinMagFilter::Linear);
    image_sampler.set_mag_filter(metal::MTLSamplerMinMagFilter::Linear);
    image_sampler.set_address_mode_s(metal::MTLSamplerAddressMode::Repeat);
    image_sampler.set_address_mode_t(metal::MTLSamplerAddressMode::Repeat);
    let image_sampler = frame.device.new_sampler(&image_sampler);

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
        image_sampler,
        blank,
    })
}

/// Upload `image` if it has not been yet, and return its texture.
fn image_texture<'a>(image: &'a mut Image, device: &metal::DeviceRef) -> &'a metal::Texture {
    image.texture.get_or_insert_with(|| {
        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_width(image.width as u64);
        descriptor.set_height(image.height as u64);
        descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        descriptor.set_usage(metal::MTLTextureUsage::ShaderRead);
        let texture = device.new_texture(&descriptor);
        texture.replace_region(
            metal::MTLRegion::new_2d(0, 0, image.width as u64, image.height as u64),
            0,
            image.bgra.as_ptr() as *const _,
            image.width as u64 * 4,
        );
        // The GPU has its own copy now.
        image.bgra = Vec::new();
        texture
    })
}

impl GpuCanvasRenderer for Inner {
    fn render(&mut self, frame: &mut GpuCanvasFrame<'_>) {
        // Split the borrow: the pipeline is built from `params` and then
        // drawn alongside it.
        let params = &self.params;
        let images = &mut self.images;
        let last_good = &mut self.last_good;
        let Some(ready) = self.pipeline.ready(&self.source, params, last_good, frame) else {
            return;
        };
        self.clock.tick(frame.time);
        let uniforms = EffectUniforms::from_frame(frame, &self.clock);
        self.clock.drew();
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
        let params = params.bytes();
        encoder.set_fragment_bytes(2, params.len() as u64, params.as_ptr() as *const _);
        encoder.set_fragment_texture(0, Some(frame.backdrop.unwrap_or(&ready.blank)));
        encoder.set_fragment_sampler_state(0, Some(&ready.sampler));
        let mut sizes = [[0f32; 4]; IMAGE_SLOTS];
        for (slot, image) in images.iter_mut().enumerate() {
            let texture = match image {
                Some(image) => {
                    sizes[slot] = [image.width as f32, image.height as f32, 0., 0.];
                    image_texture(image, frame.device)
                }
                None => &ready.blank,
            };
            encoder.set_fragment_texture(1 + slot as u64, Some(texture));
        }
        encoder.set_fragment_bytes(
            3,
            std::mem::size_of_val(&sizes) as u64,
            sizes.as_ptr() as *const _,
        );
        encoder.set_fragment_sampler_state(1, Some(&ready.image_sampler));
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
        let uniforms = fragment_argument(&device, &msl(effect, &ParamLayout::empty()), "u");
        assert_eq!(
            uniforms.buffer_data_size() as usize,
            std::mem::size_of::<EffectUniforms>()
        );
        let metal_offset = member_offsets(&uniforms);
        let expected = [
            ("resolution", offset_of!(EffectUniforms, resolution)),
            ("pointer", offset_of!(EffectUniforms, pointer)),
            ("origin", offset_of!(EffectUniforms, origin)),
            ("time", offset_of!(EffectUniforms, time)),
            ("scale", offset_of!(EffectUniforms, scale)),
            ("has_pointer", offset_of!(EffectUniforms, has_pointer)),
            ("delta", offset_of!(EffectUniforms, delta)),
            ("frame", offset_of!(EffectUniforms, frame)),
            ("_pad0", offset_of!(EffectUniforms, _pad)),
            ("corner_radii", offset_of!(EffectUniforms, corner_radii)),
        ];
        for (name, rust_offset) in expected {
            assert_eq!(metal_offset(name), rust_offset, "offset of `{name}`");
        }
    }

    /// What Metal compiles for an MSL `effect` with `params`.
    fn msl(effect: &str, params: &ParamLayout) -> String {
        full_source(&Source::Msl(effect.into()), params).unwrap()
    }

    /// Compile `source` and return the fragment argument named `name`,
    /// with Metal's own view of its layout.
    fn fragment_argument(device: &metal::Device, source: &str, name: &str) -> metal::Argument {
        let library = device
            .new_library_with_source(source, &metal::CompileOptions::new())
            .expect("source compiles");
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
        (0..arguments.count())
            .filter_map(|i| arguments.object_at(i))
            .find(|a| a.name() == name)
            .unwrap_or_else(|| panic!("fragment argument `{name}`"))
            .to_owned()
    }

    fn member_offsets(argument: &metal::Argument) -> impl Fn(&str) -> usize {
        let members = argument.buffer_struct_type().members();
        move |name: &str| {
            (0..members.count())
                .filter_map(|i| members.object_at(i))
                .find(|m| m.name() == name)
                .unwrap_or_else(|| panic!("MSL struct has no member `{name}`"))
                .offset() as usize
        }
    }

    /// Every type, in an order that exercises float3 and float2 padding.
    fn awkward_layout() -> ParamLayout {
        ParamLayout::new(&[
            ("a", Param::Float),
            ("b", Param::Float3),
            ("c", Param::Float),
            ("d", Param::Float2),
            ("e", Param::Float4),
            ("f", Param::Int),
        ])
        .unwrap()
    }

    /// Metal's offsets for fragment argument `p` must equal `layout`'s.
    fn assert_params_match(device: &metal::Device, source: &str, layout: &ParamLayout) {
        let argument = fragment_argument(device, source, "p");
        assert_eq!(argument.buffer_data_size() as usize, layout.size());
        let metal_offset = member_offsets(&argument);
        for field in layout.fields() {
            assert_eq!(
                metal_offset(&field.name),
                field.offset,
                "offset of `{}`",
                field.name
            );
        }
    }

    /// The pure-Rust layout must agree with Metal for every type.
    #[test]
    fn metal_param_offsets_match_layout() {
        let Some(device) = metal::Device::system_default() else {
            eprintln!("no Metal device; skipping");
            return;
        };
        let layout = awkward_layout();
        let effect = "float4 effect(float2 uv, constant EffectUniforms &u, constant EffectParams &p) { return p.e + p.a; }";
        assert_params_match(&device, &msl(effect, &layout), &layout);
    }

    /// Naga lays out the WGSL preamble's structs; Metal must read them at
    /// the offsets Rust uploads, for the uniforms and the awkward params.
    #[test]
    fn metal_wgsl_offsets_match_rust() {
        let Some(device) = metal::Device::system_default() else {
            eprintln!("no Metal device; skipping");
            return;
        };
        let layout = awkward_layout();
        let effect = "fn effect(uv: vec2<f32>) -> vec4<f32> { return p.e + p.a + u.corner_radii + u.time + f32(u.frame) + u.delta; }";
        let source = full_source(&Source::Wgsl(effect.into()), &layout).unwrap();
        assert_params_match(&device, &source, &layout);
        let uniforms = fragment_argument(&device, &source, "u");
        assert_eq!(
            uniforms.buffer_data_size() as usize,
            std::mem::size_of::<EffectUniforms>()
        );
        let metal_offset = member_offsets(&uniforms);
        assert_eq!(metal_offset("time"), offset_of!(EffectUniforms, time));
        assert_eq!(metal_offset("frame"), offset_of!(EffectUniforms, frame));
        assert_eq!(metal_offset("delta"), offset_of!(EffectUniforms, delta));
        assert_eq!(
            metal_offset("corner_radii"),
            offset_of!(EffectUniforms, corner_radii)
        );
    }

    #[test]
    fn fragment_entry_picks_params_call() {
        let cases = [
            (
                "float4 effect(float2 uv, constant EffectUniforms &u)",
                "effect(in.uv, u);",
            ),
            ("... EffectBackdrop b)", "effect(in.uv, u, backdrop);"),
            ("... constant EffectParams &p)", "effect(in.uv, u, p);"),
            (
                "... EffectBackdrop b, constant EffectParams &p)",
                "effect(in.uv, u, backdrop, p);",
            ),
            ("... EffectImages i)", "effect(in.uv, u, images);"),
            (
                "... EffectBackdrop b, constant EffectParams &p, EffectImages i)",
                "effect(in.uv, u, backdrop, p, images);",
            ),
        ];
        for (source, call) in cases {
            assert!(fragment_entry(source).contains(call), "{source}");
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

    /// A shader that reads images compiles against the wrapper.
    #[test]
    fn image_effect_compiles() {
        let Some(device) = metal::Device::system_default() else {
            eprintln!("no Metal device; skipping");
            return;
        };
        let effect = "float4 effect(float2 uv, constant EffectUniforms &u, EffectImages i) { return i.sample(1, uv) * i.size(1).x; }";
        let source = msl(effect, &ParamLayout::empty());
        device
            .new_library_with_source(&source, &metal::CompileOptions::new())
            .expect("image preamble compiles")
            .get_function("effect_fragment", None)
            .unwrap();
    }

    #[test]
    fn set_source_recompiles_and_shadertoy_uniforms_are_settable() {
        let effect = Effect::shadertoy(
            "uniform vec4 tint;\nvoid mainImage(out vec4 c, in vec2 p) { c = tint; }",
        );
        assert_eq!(effect.set("tint", [1., 0., 0., 1.]), Ok(()));
        effect.set_source("float4 effect(float2 uv, constant EffectUniforms &u) { return 0; }");
        assert!(matches!(effect.0.borrow().pipeline, Pipeline::Pending));
        assert!(!effect.reads_pointer());
    }

    /// A WGSL effect that reads `p` counts as reading params; one that
    /// does not gets the warning, same as MSL without `EffectParams`.
    #[test]
    fn wgsl_source_reads_params_through_p() {
        let reads = |wgsl: &str| Source::Wgsl(wgsl.into()).reads_params();
        assert!(reads(
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return p.tint; }"
        ));
        assert!(!reads(
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return vec4(uv, 0.0, 1.0); }"
        ));
        // `tmp.x` is not a params read.
        assert!(!reads(
            "fn effect(uv: vec2<f32>) -> vec4<f32> { let tmp = vec4(uv, 0.0, 1.0); return tmp.xyzw; }"
        ));
        let effect =
            Effect::wgsl("fn effect(uv: vec2<f32>) -> vec4<f32> { return vec4(uv, 0.0, 1.0); }");
        assert!(!effect.0.borrow().source.reads_params());
        effect.set_wgsl("fn effect(uv: vec2<f32>) -> vec4<f32> { return p.tint; }");
        assert!(effect.0.borrow().source.reads_params());
    }

    #[test]
    fn clock_measures_delta_and_counts_frames() {
        let mut clock = Clock::default();
        clock.tick(Duration::from_millis(100));
        assert_eq!((clock.delta, clock.frame), (Duration::ZERO, 0));
        clock.drew();
        clock.tick(Duration::from_millis(116));
        assert_eq!((clock.delta, clock.frame), (Duration::from_millis(16), 1));
        clock.drew();
        // Time went backwards: no negative delta, no panic.
        clock.tick(Duration::from_millis(50));
        assert_eq!((clock.delta, clock.frame), (Duration::ZERO, 2));
    }

    #[test]
    fn image_rejects_bad_slot_and_length() {
        let effect = Effect::new("");
        assert_eq!(
            effect.image(4, 1, 1, vec![0; 4]),
            Err(ImageError::BadSlot(4))
        );
        assert_eq!(
            effect.image(0, 2, 2, vec![0; 4]),
            Err(ImageError::BadLength {
                expected: 16,
                got: 4
            })
        );
        assert_eq!(effect.image(3, 2, 2, vec![0; 16]), Ok(()));
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
        let source = msl(effect, &ParamLayout::empty());
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
        let source = msl(&effect, &ParamLayout::empty());
        device
            .new_library_with_source(&source, &metal::CompileOptions::new())
            .expect("crt.glsl compiles");
    }
}
