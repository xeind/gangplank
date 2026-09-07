//! Shadertoy and Ghostty style GLSL, run as an [`Effect`](crate::Effect).
//!
//! Replaces porting each shader by hand. Shadertoy shaders are GLSL with a
//! `mainImage(out vec4 fragColor, in vec2 fragCoord)` entry and globals
//! `iTime`, `iResolution`, `iMouse`. Metal has neither `out` params nor
//! per-draw globals, so the source is wrapped in a struct that holds the
//! uniforms; every helper becomes a member and sees `iTime` as `u.time`.
//!
//! Not covered: `iChannel0..3` textures (Ghostty's terminal image), `iDate`,
//! `mat2(a, b, c, d)` scalar constructors, and GLSL-only builtins. Those
//! surface as a Metal compile error in the log.

use regex::Regex;
use std::sync::OnceLock;

/// GLSL to MSL. The result defines `effect(uv, u)` as [`crate::Effect`] needs.
pub fn glsl_to_msl(glsl: &str) -> String {
    let body = rewrite_body(glsl);
    format!("{PRELUDE}\nstruct Shadertoy {{\n    constant EffectUniforms &u;\n{body}\n}};\n{ENTRY}")
}

fn rewrite_body(glsl: &str) -> String {
    static OUT: OnceLock<Regex> = OnceLock::new();
    static INOUT: OnceLock<Regex> = OnceLock::new();
    static IN: OnceLock<Regex> = OnceLock::new();
    static PRECISION: OnceLock<Regex> = OnceLock::new();
    static CONST: OnceLock<Regex> = OnceLock::new();

    let out = OUT.get_or_init(|| Regex::new(r"\bout\s+(\w+)\s+(\w+)").unwrap());
    let inout = INOUT.get_or_init(|| Regex::new(r"\binout\s+(\w+)\s+(\w+)").unwrap());
    let inp = IN.get_or_init(|| Regex::new(r"\bin\s+(\w+)\s+(\w+)").unwrap());
    let precision =
        PRECISION.get_or_init(|| Regex::new(r"(?m)^\s*precision\s+[^;]+;\s*$").unwrap());
    // File-scope `const T x = ...;` becomes a member with a default initializer.
    let konst = CONST.get_or_init(|| Regex::new(r"(?m)^\s*const\s+").unwrap());

    let s = precision.replace_all(glsl, "");
    let s = inout.replace_all(&s, "thread $1 &$2");
    let s = out.replace_all(&s, "thread $1 &$2");
    let s = inp.replace_all(&s, "$1 $2");
    let s = konst.replace_all(&s, "const ");
    s.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

const PRELUDE: &str = r#"
typedef float2 vec2; typedef float3 vec3; typedef float4 vec4;
typedef int2 ivec2; typedef int3 ivec3; typedef int4 ivec4;
typedef bool2 bvec2; typedef bool3 bvec3; typedef bool4 bvec4;
typedef float2x2 mat2; typedef float3x3 mat3; typedef float4x4 mat4;

inline float  mod(float  x, float  y) { return x - y * floor(x / y); }
inline float2 mod(float2 x, float2 y) { return x - y * floor(x / y); }
inline float3 mod(float3 x, float3 y) { return x - y * floor(x / y); }
inline float4 mod(float4 x, float4 y) { return x - y * floor(x / y); }
inline float2 mod(float2 x, float  y) { return x - y * floor(x / y); }
inline float3 mod(float3 x, float  y) { return x - y * floor(x / y); }
inline float4 mod(float4 x, float  y) { return x - y * floor(x / y); }
inline float  atan(float y, float x) { return atan2(y, x); }
inline float2 atan(float2 y, float2 x) { return atan2(y, x); }
#define inversesqrt rsqrt
#define lessThan(a, b) ((a) < (b))
#define greaterThan(a, b) ((a) > (b))

// Shadertoy globals, read through the wrapping struct's `u`.
#define iTime (u.time)
#define iTimeDelta (1.0 / 60.0)
#define iFrame (int(u.time * 60.0))
#define iResolution (float3(u.resolution, 1.0))
#define iMouse (u.has_pointer > 0.5 \
    ? float4(u.pointer.x, u.resolution.y - u.pointer.y, 0.0, 0.0) \
    : float4(0.0))
"#;

const ENTRY: &str = r#"
float4 effect(float2 uv, constant EffectUniforms &u) {
    Shadertoy s{u};
    float4 color = float4(0.0, 0.0, 0.0, 1.0);
    // Shadertoy's fragCoord has its origin at the bottom-left.
    float2 fragCoord = float2(uv.x, 1.0 - uv.y) * u.resolution;
    s.mainImage(color, fragCoord);
    return color;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_entry_and_qualifiers() {
        let glsl = "precision mediump float;\nconst float PI = 3.14;\nvoid bump(inout vec3 c) { c += PI; }\nvoid mainImage(out vec4 fragColor, in vec2 fragCoord) { fragColor = vec4(iTime); }";
        let msl = glsl_to_msl(glsl);
        assert!(!msl.contains("precision"));
        assert!(msl.contains("void bump(thread vec3 &c)"));
        assert!(msl.contains("void mainImage(thread vec4 &fragColor, vec2 fragCoord)"));
        assert!(msl.contains("struct Shadertoy {"));
        assert!(msl.contains("s.mainImage(color, fragCoord);"));
    }
}
