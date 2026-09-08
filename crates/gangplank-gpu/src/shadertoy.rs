//! Shadertoy and Ghostty style GLSL, run as an [`Effect`](crate::Effect).
//!
//! Replaces porting each shader by hand. Shadertoy shaders are GLSL with a
//! `mainImage(out vec4 fragColor, in vec2 fragCoord)` entry and globals
//! `iTime`, `iResolution`, `iMouse`, `iChannel0..3`. The crate wraps the
//! source in a GLSL 450 fragment shader that declares the crate's bindings
//! and defines those globals, and naga's GLSL front end translates it to
//! MSL. Errors carry the shader's own line numbers.
//!
//! `iChannel0` is what GPUI drew under the element, Ghostty's terminal
//! image, when the element asked for it with `.backdrop(margin)`; otherwise
//! it samples transparent black. `iChannel1..3` are the app's images from
//! `Effect::image(0..2, ..)`, and a `uniform sampler2D name;` takes the
//! next free one of those in order. `texture(iChannelN, uv)` takes
//! Shadertoy's bottom-left uv. File-scope `uniform float|vec2|vec3|vec4|int
//! name;` become [`Effect::params`](crate::Effect::params) fields of the
//! same name.
//!
//! Not covered: `iDate`, `iChannelResolution`, `textureSize`, `iSampleRate`
//! and other uniform types. Those surface as a translation error in the
//! log.

use crate::Param;
use crate::params::ParamLayout;
use naga::back::msl::{Options, PipelineOptions};
use naga::front::glsl::{Frontend, Options as GlslOptions};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use regex::Regex;
use std::sync::OnceLock;

/// Everything before the user's source. Names carry an `effect_` prefix
/// so a shader's own `p`, `u` or `uv` cannot shadow them. Layouts must
/// match `EffectUniforms` in `effect.rs` and the slot table in gpu.md.
const HEAD: &str = r#"#version 450
layout(std140, set = 0, binding = 0) uniform EffectUniforms {
    vec2 resolution;
    vec2 pointer;
    vec2 origin;
    float time;
    float scale;
    float has_pointer;
    float delta;
    uint frame;
    float _pad0;
    vec4 corner_radii;
} effect_uniforms;
layout(std140, set = 0, binding = 1) uniform EffectViewport { vec2 effect_viewport; };
layout(std140, set = 0, binding = 3) uniform EffectImageSizes { vec4 effect_image_sizes[4]; };
layout(set = 1, binding = 0) uniform texture2D effect_backdrop_texture;
layout(set = 1, binding = 1) uniform texture2D effect_image0;
layout(set = 1, binding = 2) uniform texture2D effect_image1;
layout(set = 1, binding = 3) uniform texture2D effect_image2;
layout(set = 1, binding = 4) uniform texture2D effect_image3;
layout(set = 2, binding = 0) uniform sampler effect_backdrop_sampler;
layout(set = 2, binding = 1) uniform sampler effect_image_sampler;
layout(location = 0) in vec2 effect_uv;
layout(location = 0) out vec4 effect_color;
"#;

/// After the params block: the Shadertoy globals and the channel type.
/// `texture` becomes a macro over `effect_texture`, which flips to
/// Shadertoy's bottom-left origin and, for the backdrop, remaps element
/// uv into the drawable. The macro comes after the function so the
/// builtin calls inside it stay builtin.
const GLOBALS: &str = r#"
#define iTime (effect_uniforms.time)
#define iTimeDelta (effect_uniforms.delta)
#define iFrame (int(effect_uniforms.frame))
#define iFrameRate (effect_uniforms.delta > 0.0 ? 1.0 / effect_uniforms.delta : 0.0)
#define iResolution (vec3(effect_uniforms.resolution, 1.0))
#define iMouse (effect_uniforms.has_pointer > 0.5 \
    ? vec4(effect_uniforms.pointer.x, effect_uniforms.resolution.y - effect_uniforms.pointer.y, 0.0, 0.0) \
    : vec4(0.0))
struct EffectChannel { int slot; };
#define iChannel0 EffectChannel(0)
#define iChannel1 EffectChannel(1)
#define iChannel2 EffectChannel(2)
#define iChannel3 EffectChannel(3)
vec4 effect_texture(EffectChannel c, vec2 uv) {
    vec2 st = vec2(uv.x, 1.0 - uv.y);
    switch (c.slot) {
        case 0: return texture(sampler2D(effect_backdrop_texture, effect_backdrop_sampler),
                               (effect_uniforms.origin + st * effect_uniforms.resolution) / effect_viewport);
        case 1: return texture(sampler2D(effect_image0, effect_image_sampler), st);
        case 2: return texture(sampler2D(effect_image1, effect_image_sampler), st);
        case 3: return texture(sampler2D(effect_image2, effect_image_sampler), st);
        default: return vec4(0.0);
    }
}
#define texture(c, uv) effect_texture(c, uv)
"#;

/// After the user's source: the corner fade and `main`.
const TAIL: &str = r#"
float effect_corner_coverage(vec2 pixel, vec2 size, vec4 radii) {
    vec2 half_size = size * 0.5;
    vec2 q = pixel - half_size;
    float r = q.x < 0.0 ? (q.y < 0.0 ? radii.x : radii.w) : (q.y < 0.0 ? radii.y : radii.z);
    vec2 d = abs(q) - half_size + r;
    float sdf = length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - r;
    return clamp(0.5 - sdf, 0.0, 1.0);
}
void main() {
    vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
    // Shadertoy's fragCoord has its origin at the bottom-left.
    mainImage(color, vec2(effect_uv.x, 1.0 - effect_uv.y) * effect_uniforms.resolution);
    if (any(greaterThan(effect_uniforms.corner_radii, vec4(0.0)))) {
        color *= effect_corner_coverage(effect_uv * effect_uniforms.resolution,
                                        effect_uniforms.resolution, effect_uniforms.corner_radii);
    }
    effect_color = color;
}
"#;

/// The vertex stage, appended to naga's MSL. Naga names the varying
/// `[[user(loc0)]]`, so this matches it by attribute. It reads only the
/// head of `EffectUniforms`, declared here as a prefix of that struct.
const VERTEX_MSL: &str = r#"
struct EffectVertexUniforms { metal::float2 resolution; metal::float2 pointer; metal::float2 origin; };
struct EffectVertexOut { metal::float4 position [[position]]; metal::float2 uv [[user(loc0)]]; };
vertex EffectVertexOut effect_vertex(uint vid [[vertex_id]],
                                     constant EffectVertexUniforms &u [[buffer(0)]],
                                     constant metal::float2 &viewport [[buffer(1)]]) {
    const metal::float2 corners[6] = { {0, 0}, {1, 0}, {0, 1}, {0, 1}, {1, 0}, {1, 1} };
    metal::float2 uv = corners[vid];
    metal::float2 pixel = u.origin + uv * u.resolution;
    metal::float2 ndc = pixel / viewport * 2.0 - 1.0;
    ndc.y = -ndc.y;
    return EffectVertexOut { metal::float4(ndc, 0.0, 1.0), uv };
}
"#;

/// Naga's name for GLSL `main`, the fragment entry `build_pipeline` asks for.
pub(crate) const FRAGMENT_ENTRY: &str = "main_";

/// Image channels a `uniform sampler2D` can take: `iChannel1..3`.
const SAMPLER_CHANNELS: usize = 3;

/// GLSL to MSL that defines `effect_vertex` and [`FRAGMENT_ENTRY`] against
/// the crate's slots. `params` must be the layout from [`glsl_uniforms`]
/// of the same source. `Err` carries naga's report with the user's own
/// line numbers.
pub(crate) fn glsl_to_msl(glsl: &str, params: &ParamLayout) -> Result<String, String> {
    let samplers = glsl_samplers(glsl);
    if samplers.len() > SAMPLER_CHANNELS {
        return Err(format!(
            "GLSL declares {} `uniform sampler2D`; only {SAMPLER_CHANNELS} image channels exist",
            samplers.len()
        ));
    }
    let channels: String = samplers
        .iter()
        .enumerate()
        .map(|(i, name)| format!("const EffectChannel {name} = EffectChannel({});\n", i + 1))
        .collect();
    let head = format!("{HEAD}{}{GLOBALS}{channels}", params.glsl_block());
    let body = sampler_regex().replace_all(glsl, "");
    let body = uniform_regex().replace_all(&body, "");
    let source = format!("{head}{body}{TAIL}");
    let user_lines = head.lines().count()..head.lines().count() + body.lines().count();

    let module = Frontend::default()
        .parse(&GlslOptions::from(naga::ShaderStage::Fragment), &source)
        .map_err(|err| renumber(&err.emit_to_string(&source), &user_lines))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|err| renumber(&err.emit_to_string(&source), &user_lines))?;
    let mut options = Options {
        lang_version: (2, 2),
        fake_missing_bindings: false,
        ..Default::default()
    };
    options
        .per_entry_point_map
        .insert("main".to_string(), crate::wgsl::resources());
    let (msl, _) =
        naga::back::msl::write_string(&module, &info, &options, &PipelineOptions::default())
            .map_err(|err| format!("GLSL to MSL: {err}"))?;
    Ok(format!("{msl}\n{VERTEX_MSL}"))
}

/// Shift naga's `glsl:LINE:COL` references so lines count from the
/// user's source. `user_lines` is where that source sits in the wrapped
/// file, zero-based; a line outside it, such as `main` failing to find
/// `mainImage`, is reported as the wrapper's with its absolute number.
fn renumber(report: &str, user_lines: &std::ops::Range<usize>) -> String {
    static LINE: OnceLock<Regex> = OnceLock::new();
    let line = LINE.get_or_init(|| Regex::new(r"glsl:(\d+):").unwrap());
    let shifted = line.replace_all(report, |c: &regex::Captures| {
        let n: usize = c[1].parse().unwrap_or(0);
        if n >= 1 && user_lines.contains(&(n - 1)) {
            format!("glsl:{}:", n - user_lines.start)
        } else {
            format!("wrapper:{n}:")
        }
    });
    format!("GLSL:\n{shifted}")
}

/// The `uniform float|vec2|vec3|vec4|int name;` declarations in `glsl`, in
/// order, as params.
pub fn glsl_uniforms(glsl: &str) -> Vec<(String, Param)> {
    uniform_regex()
        .captures_iter(glsl)
        .map(|c| {
            let ty = match &c[1] {
                "float" => Param::Float,
                "vec2" => Param::Float2,
                "vec3" => Param::Float3,
                "vec4" => Param::Float4,
                _ => Param::Int,
            };
            (c[2].to_string(), ty)
        })
        .collect()
}

/// The `uniform sampler2D name;` declarations, in order.
fn glsl_samplers(glsl: &str) -> Vec<String> {
    sampler_regex()
        .captures_iter(glsl)
        .map(|c| c[1].to_string())
        .collect()
}

fn uniform_regex() -> &'static Regex {
    static UNIFORM: OnceLock<Regex> = OnceLock::new();
    UNIFORM.get_or_init(|| {
        Regex::new(r"(?m)^[ \t]*uniform[ \t]+(float|vec2|vec3|vec4|int)[ \t]+(\w+)[ \t]*;[ \t]*$")
            .unwrap()
    })
}

fn sampler_regex() -> &'static Regex {
    static SAMPLER: OnceLock<Regex> = OnceLock::new();
    SAMPLER.get_or_init(|| {
        Regex::new(r"(?m)^[ \t]*uniform[ \t]+sampler2D[ \t]+(\w+)[ \t]*;[ \t]*$").unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_for(glsl: &str) -> ParamLayout {
        let decls = glsl_uniforms(glsl);
        let decls: Vec<(&str, Param)> = decls.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        ParamLayout::new(&decls).unwrap()
    }

    /// The example shaders translate without a device: the CRT one reads
    /// `iChannel0` and `col.b`, the starfield one `iMouse`.
    #[test]
    fn example_shaders_translate() {
        for glsl in [
            include_str!("../examples/shaders/crt.glsl"),
            include_str!("../examples/shaders/starfield.glsl"),
            include_str!("../examples/shaders/waves.glsl"),
        ] {
            let msl = glsl_to_msl(glsl, &layout_for(glsl)).unwrap();
            assert!(
                msl.contains(&format!("fragment main_Output {FRAGMENT_ENTRY}(")),
                "{msl}"
            );
            assert!(msl.contains("vertex EffectVertexOut effect_vertex("));
        }
    }

    /// `out`, `inout`, `precision`, file-scope `const`, a local named `p`
    /// and a swizzle `.b` all pass through naga untouched.
    #[test]
    fn glsl_features_need_no_rewrite() {
        let glsl = "precision mediump float;\nconst float PI = 3.14;\nvoid bump(inout vec3 c) { c.b += PI; }\nvoid mainImage(out vec4 fragColor, in vec2 p) { vec3 u = vec3(iTime); bump(u); fragColor = vec4(u, 1.0) * mod(p.x, 2.0) + texture(iChannel0, p).r; }";
        glsl_to_msl(glsl, &ParamLayout::empty()).unwrap();
    }

    /// Uniforms become block members of the same name, read as globals.
    #[test]
    fn uniforms_become_params() {
        let glsl = "uniform vec4 tint;\nuniform float amount;\nuniform vec3 axis;\nvoid mainImage(out vec4 c, in vec2 p) { c = tint * amount + vec4(axis, 1.0); }";
        assert_eq!(
            glsl_uniforms(glsl),
            vec![
                ("tint".to_string(), Param::Float4),
                ("amount".to_string(), Param::Float),
                ("axis".to_string(), Param::Float3),
            ]
        );
        let msl = glsl_to_msl(glsl, &layout_for(glsl)).unwrap();
        assert!(msl.contains("[[buffer(2)]]"), "{msl}");
    }

    /// A `sampler2D` uniform takes the next image channel, in order.
    #[test]
    fn samplers_take_image_channels() {
        let glsl = "uniform sampler2D tex;\nuniform sampler2D mask;\nvoid mainImage(out vec4 c, in vec2 p) { c = texture(tex, p) * texture(mask, p).a; }";
        let msl = glsl_to_msl(glsl, &ParamLayout::empty()).unwrap();
        // `tex` is image0 at texture(1), `mask` image1 at texture(2).
        let fragment = &msl[msl.find("fragment ").unwrap()..];
        assert!(
            fragment.contains("effect_image0_ [[texture(1)]]"),
            "{fragment}"
        );
        assert!(
            fragment.contains("effect_image1_ [[texture(2)]]"),
            "{fragment}"
        );
        let four = "uniform sampler2D a;\nuniform sampler2D b;\nuniform sampler2D c;\nuniform sampler2D d;\nvoid mainImage(out vec4 o, in vec2 p) { o = vec4(0.0); }";
        assert!(
            glsl_to_msl(four, &ParamLayout::empty())
                .unwrap_err()
                .contains("only 3")
        );
    }

    /// The report points at the user's line, not the wrapper's.
    #[test]
    fn error_reports_user_line() {
        let glsl = "// one\nvoid mainImage(out vec4 c, in vec2 p) {\n    c = vec4(missing);\n}";
        let err = glsl_to_msl(glsl, &ParamLayout::empty()).unwrap_err();
        assert!(err.contains("missing"), "{err}");
        assert!(err.contains("glsl:3:"), "{err}");
        // No `mainImage`: the wrapper's `main` fails, reported as such.
        let err = glsl_to_msl("// nothing", &ParamLayout::empty()).unwrap_err();
        assert!(err.contains("wrapper:"), "{err}");
        assert!(!err.contains("glsl:"), "{err}");
    }
}
