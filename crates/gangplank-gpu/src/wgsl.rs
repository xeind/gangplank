//! WGSL effects, translated to MSL by naga at pipeline build time.
//!
//! The user defines `fn effect(uv: vec2<f32>) -> vec4<f32>` and reads the
//! uniforms, params and textures as module globals, which is how WGSL is
//! written. The crate appends a preamble that declares those globals, the
//! vertex stage and the fragment entry, and maps each binding onto the
//! Metal slot the renderer already fills. Naga emits both stages, so the
//! varying between them matches by construction.
//!
//! The user's source goes first: WGSL declarations are order independent,
//! and this way a parse error reports the user's own line numbers.

use crate::params::ParamLayout;
use naga::ResourceBinding;
use naga::back::msl::{
    BindSamplerTarget, BindTarget, EntryPointResources, Options, PipelineOptions,
};
use naga::valid::{Capabilities, ValidationFlags, Validator};

/// Declarations every WGSL effect sees. Must stay in step with
/// `EffectUniforms` in `effect.rs` and the slot table in
/// `docs/architecture/gpu.md`. The params struct is emitted separately
/// from the layout.
const PREAMBLE: &str = r#"
struct EffectUniforms {
    resolution: vec2<f32>,
    pointer: vec2<f32>,
    origin: vec2<f32>,
    time: f32,
    scale: f32,
    has_pointer: f32,
    delta: f32,
    frame: u32,
    _pad0: f32,
    corner_radii: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: EffectUniforms;
@group(0) @binding(1) var<uniform> viewport: vec2<f32>;
@group(0) @binding(2) var<uniform> p: EffectParams;
@group(0) @binding(3) var<uniform> image_sizes: array<vec4<f32>, 4>;
@group(1) @binding(0) var backdrop_texture: texture_2d<f32>;
@group(1) @binding(1) var image0: texture_2d<f32>;
@group(1) @binding(2) var image1: texture_2d<f32>;
@group(1) @binding(3) var image2: texture_2d<f32>;
@group(1) @binding(4) var image3: texture_2d<f32>;
@group(2) @binding(0) var backdrop_sampler: sampler;
@group(2) @binding(1) var image_sampler: sampler;

// What GPUI drew under the element at uv, when the element asked for it
// with `.backdrop(margin)`.
fn backdrop_sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(backdrop_texture, backdrop_sampler, (u.origin + uv * u.resolution) / viewport);
}

struct EffectVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn effect_vertex(@builtin(vertex_index) vid: u32) -> EffectVertex {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let uv = corners[vid];
    let pixel = u.origin + uv * u.resolution;
    var ndc = pixel / viewport * 2.0 - 1.0;
    ndc.y = -ndc.y;
    return EffectVertex(vec4(ndc, 0.0, 1.0), uv);
}

// Coverage of a rounded rectangle at pixel `p`: 1 inside, 0 outside, with a
// one-pixel ramp at the edge. Radii order matches EffectUniforms.
fn effect_corner_coverage(pixel: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = pixel - half_size;
    var r: f32;
    if (q.x < 0.0) {
        r = select(radii.w, radii.x, q.y < 0.0);
    } else {
        r = select(radii.z, radii.y, q.y < 0.0);
    }
    let d = abs(q) - half_size + r;
    let sdf = length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
    return saturate(0.5 - sdf);
}

@fragment fn effect_fragment(in: EffectVertex) -> @location(0) vec4<f32> {
    var color = effect(in.uv);
    if (any(u.corner_radii > vec4(0.0))) {
        color *= effect_corner_coverage(in.uv * u.resolution, u.resolution, u.corner_radii);
    }
    return color;
}
"#;

/// Group and binding of each preamble global, and the Metal slot it lands
/// in. Buffers 0..3, textures 0..4, samplers 0..1: the table in gpu.md.
/// The GLSL wrapper in `shadertoy.rs` declares the same set.
pub(crate) fn resources() -> EntryPointResources {
    let mut resources = EntryPointResources::default();
    let mut bind = |group: u32, binding: u32, target: BindTarget| {
        resources
            .resources
            .insert(ResourceBinding { group, binding }, target);
    };
    for slot in 0..4u8 {
        bind(
            0,
            slot as u32,
            BindTarget {
                buffer: Some(slot),
                ..Default::default()
            },
        );
    }
    for slot in 0..5u8 {
        bind(
            1,
            slot as u32,
            BindTarget {
                texture: Some(slot),
                ..Default::default()
            },
        );
    }
    for slot in 0..2u8 {
        bind(
            2,
            slot as u32,
            BindTarget {
                sampler: Some(BindSamplerTarget::Resource(slot)),
                ..Default::default()
            },
        );
    }
    resources
}

/// WGSL to MSL that defines `effect_vertex` and `effect_fragment` against
/// the crate's slots. `Err` carries naga's report with the offending line.
pub(crate) fn wgsl_to_msl(user: &str, params: &ParamLayout) -> Result<String, String> {
    let source = format!("{user}\n{}\n{PREAMBLE}", params.wgsl_struct());
    let module = naga::front::wgsl::parse_str(&source)
        .map_err(|err| format!("WGSL parse:\n{}", err.emit_to_string(&source)))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|err| format!("WGSL validation:\n{}", err.emit_to_string(&source)))?;
    let mut options = Options {
        // MSL 2.2 is macOS 10.15, the floor gpui-ce compiles its own
        // shaders for.
        lang_version: (2, 2),
        // A binding the table above forgot is an error here, not invalid
        // MSL that Metal then rejects with a worse message.
        fake_missing_bindings: false,
        ..Default::default()
    };
    for entry in ["effect_vertex", "effect_fragment"] {
        options
            .per_entry_point_map
            .insert(entry.to_string(), resources());
    }
    let (msl, _) =
        naga::back::msl::write_string(&module, &info, &options, &PipelineOptions::default())
            .map_err(|err| format!("WGSL to MSL: {err}"))?;
    Ok(msl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Param;

    #[test]
    fn translates_minimal_effect() {
        let msl = wgsl_to_msl(
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return vec4(uv, 0.0, 1.0); }",
            &ParamLayout::empty(),
        )
        .unwrap();
        assert!(msl.contains("vertex "), "{msl}");
        assert!(msl.contains(" effect_vertex("), "{msl}");
        assert!(msl.contains("fragment "), "{msl}");
        assert!(msl.contains(" effect_fragment("), "{msl}");
    }

    /// Every global reaches the slot the renderer fills, once each.
    #[test]
    fn binds_the_contract_slots() {
        let user = r#"
fn effect(uv: vec2<f32>) -> vec4<f32> {
    var c = backdrop_sample(uv) * p.tint;
    c += textureSample(image0, image_sampler, uv) * image_sizes[0].x;
    c += textureSample(image1, image_sampler, uv);
    c += textureSample(image2, image_sampler, uv);
    c += textureSample(image3, image_sampler, uv);
    return c;
}"#;
        let layout = ParamLayout::new(&[("tint", Param::Float4)]).unwrap();
        let msl = wgsl_to_msl(user, &layout).unwrap();
        let fragment = &msl[msl.find("fragment ").unwrap()..];
        for slot in [
            "[[buffer(0)]]",
            "[[buffer(1)]]",
            "[[buffer(2)]]",
            "[[buffer(3)]]",
            "[[texture(0)]]",
            "[[texture(1)]]",
            "[[texture(2)]]",
            "[[texture(3)]]",
            "[[texture(4)]]",
            "[[sampler(0)]]",
            "[[sampler(1)]]",
        ] {
            assert_eq!(fragment.matches(slot).count(), 1, "{slot}\n{fragment}");
        }
    }

    /// The user's source comes first, so the reported line is theirs.
    #[test]
    fn parse_error_names_the_line() {
        let user = "// one\n// two\nfn effect(uv: vec2<f32>) -> vec4<f32> { return vec4(uv, 0.0, missing); }";
        let err = wgsl_to_msl(user, &ParamLayout::empty()).unwrap_err();
        assert!(err.contains("missing"), "{err}");
        assert!(err.contains("wgsl:3:"), "{err}");
    }
}
