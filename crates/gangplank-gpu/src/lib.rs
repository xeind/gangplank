//! Shader effects inside GPUI layout. macOS, Metal.
//!
//! Replaces the hand-rolled pipeline, quad and uniform plumbing an app writes
//! to show one fragment shader in a GPUI element. You write the fragment
//! function, in MSL or in WGSL through [`Effect::wgsl`]; the crate owns the
//! vertex stage, the pipeline and the uniforms.
//!
//! ```ignore
//! let plasma = Effect::new(r#"
//!     float4 effect(float2 uv, constant EffectUniforms &u) {
//!         return float4(0.5 + 0.5 * cos(u.time + uv.xyx + float3(0, 2, 4)), 1);
//!     }
//! "#);
//! // in render:
//! plasma.element(elapsed).size_full()
//! ```

mod effect;
mod params;
mod shadertoy;
mod wgsl;

pub use effect::{Effect, EffectUniforms, IMAGE_SLOTS, ImageError};
pub use params::{Param, ParamError, ParamValue};
pub use shadertoy::{glsl_to_msl, glsl_uniforms};
