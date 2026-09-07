//! App-declared shader parameters. The app names and types them, the crate
//! lays them out the way Metal expects and uploads the bytes as
//! `constant EffectParams &p [[buffer(2)]]`.

use std::fmt;

/// Type of one shader parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    Float,
    Float2,
    Float3,
    Float4,
    Int,
}

impl Param {
    /// Bytes the value occupies in an MSL `constant` struct.
    pub fn size(self) -> usize {
        match self {
            Param::Float | Param::Int => 4,
            Param::Float2 => 8,
            // float3 pads to 16, same as float4.
            Param::Float3 | Param::Float4 => 16,
        }
    }

    /// Byte alignment in an MSL `constant` struct. Equal to size for every
    /// type here; the MSL vector table keeps them the same.
    pub fn align(self) -> usize {
        self.size()
    }

    /// The MSL type name.
    pub fn msl(self) -> &'static str {
        match self {
            Param::Float => "float",
            Param::Float2 => "float2",
            Param::Float3 => "float3",
            Param::Float4 => "float4",
            Param::Int => "int",
        }
    }
}

/// One value for [`Effect::set`](crate::Effect::set).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Float2([f32; 2]),
    Float3([f32; 3]),
    Float4([f32; 4]),
    Int(i32),
}

impl ParamValue {
    /// The type this value fits.
    pub fn ty(&self) -> Param {
        match self {
            ParamValue::Float(_) => Param::Float,
            ParamValue::Float2(_) => Param::Float2,
            ParamValue::Float3(_) => Param::Float3,
            ParamValue::Float4(_) => Param::Float4,
            ParamValue::Int(_) => Param::Int,
        }
    }

    /// Write the value little-endian into `out`. Bytes past the value stay
    /// as they were, so a float3 leaves its padding alone.
    fn write_le(&self, out: &mut [u8]) {
        fn floats(values: &[f32], out: &mut [u8]) {
            for (chunk, value) in out.chunks_exact_mut(4).zip(values) {
                chunk.copy_from_slice(&value.to_le_bytes());
            }
        }
        match self {
            ParamValue::Float(v) => floats(&[*v], out),
            ParamValue::Float2(v) => floats(v, out),
            ParamValue::Float3(v) => floats(v, out),
            ParamValue::Float4(v) => floats(v, out),
            ParamValue::Int(v) => out[..4].copy_from_slice(&v.to_le_bytes()),
        }
    }
}

macro_rules! from_value {
    ($($from:ty => $variant:ident),* $(,)?) => {$(
        impl From<$from> for ParamValue {
            fn from(v: $from) -> Self {
                ParamValue::$variant(v)
            }
        }
    )*};
}

from_value! {
    f32 => Float,
    [f32; 2] => Float2,
    [f32; 3] => Float3,
    [f32; 4] => Float4,
    i32 => Int,
}

/// What `params` and `set` can reject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamError {
    UnknownName(String),
    TypeMismatch {
        name: String,
        expected: Param,
        got: Param,
    },
    DuplicateName(String),
    BadIdentifier(String),
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamError::UnknownName(name) => write!(f, "no parameter named `{name}`"),
            ParamError::TypeMismatch {
                name,
                expected,
                got,
            } => write!(
                f,
                "parameter `{name}` is {} but the value is {}",
                expected.msl(),
                got.msl()
            ),
            ParamError::DuplicateName(name) => write!(f, "parameter `{name}` declared twice"),
            ParamError::BadIdentifier(name) => write!(f, "`{name}` is not a valid MSL identifier"),
        }
    }
}

impl std::error::Error for ParamError {}

/// One declared parameter and where Metal puts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) ty: Param,
    pub(crate) offset: usize,
}

/// The declared parameters in Metal layout, plus their current bytes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParamLayout {
    fields: Vec<Field>,
    size: usize,
    bytes: Vec<u8>,
}

fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

/// `[A-Za-z_][A-Za-z0-9_]*`, and not a `_pad` name the crate hands out.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let head = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    head && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.starts_with("_pad")
}

impl ParamLayout {
    /// Lay out `decls` in order. An empty list becomes one `float _pad0`
    /// so the MSL struct is never empty.
    pub(crate) fn new(decls: &[(&str, Param)]) -> Result<ParamLayout, ParamError> {
        for (i, (name, _)) in decls.iter().enumerate() {
            if !is_identifier(name) {
                return Err(ParamError::BadIdentifier(name.to_string()));
            }
            if decls[..i].iter().any(|(other, _)| other == name) {
                return Err(ParamError::DuplicateName(name.to_string()));
            }
        }
        let decls = if decls.is_empty() {
            &[("_pad0", Param::Float)]
        } else {
            decls
        };

        let mut cursor = 0;
        let mut max_align = 1;
        let fields = decls
            .iter()
            .map(|(name, ty)| {
                let offset = align_up(cursor, ty.align());
                cursor = offset + ty.size();
                max_align = max_align.max(ty.align());
                Field {
                    name: name.to_string(),
                    ty: *ty,
                    offset,
                }
            })
            .collect();
        let size = align_up(cursor, max_align);
        Ok(ParamLayout {
            fields,
            size,
            bytes: vec![0; size],
        })
    }

    /// The layout with no declarations: one `float _pad0`, four zero bytes.
    pub(crate) fn empty() -> ParamLayout {
        Self::new(&[]).expect("empty layout is valid")
    }

    /// Store `value` for `name`. The bytes reach the GPU on the next frame.
    pub(crate) fn set(&mut self, name: &str, value: ParamValue) -> Result<(), ParamError> {
        let field = self
            .fields
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| ParamError::UnknownName(name.to_string()))?;
        if field.ty != value.ty() {
            return Err(ParamError::TypeMismatch {
                name: name.to_string(),
                expected: field.ty,
                got: value.ty(),
            });
        }
        let end = field.offset + field.ty.size();
        value.write_le(&mut self.bytes[field.offset..end]);
        Ok(())
    }

    /// The `EffectParams` struct as MSL, one field per line.
    pub(crate) fn msl_struct(&self) -> String {
        let mut out = String::from("struct EffectParams {\n");
        for field in &self.fields {
            out.push_str(&format!("    {} {};\n", field.ty.msl(), field.name));
        }
        out.push_str("};\n");
        out
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[cfg(test)]
    pub(crate) fn size(&self) -> usize {
        self.size
    }

    #[cfg(test)]
    pub(crate) fn fields(&self) -> &[Field] {
        &self.fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn awkward() -> ParamLayout {
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

    /// Offsets follow the MSL table: float3 aligns to 16, float2 to 8, and
    /// the struct rounds up to its widest member.
    #[test]
    fn layout_awkward_order_follows_metal_alignment() {
        let layout = awkward();
        let offsets: Vec<usize> = layout.fields().iter().map(|f| f.offset).collect();
        assert_eq!(offsets, [0, 16, 32, 40, 48, 64]);
        assert_eq!(layout.size(), 80);
        assert_eq!(layout.bytes().len(), 80);
    }

    #[test]
    fn layout_empty_emits_pad() {
        let layout = ParamLayout::empty();
        assert_eq!(layout.size(), 4);
        assert_eq!(layout.bytes(), &[0; 4]);
        assert!(layout.msl_struct().contains("_pad0"));
        assert_eq!(layout, ParamLayout::new(&[]).unwrap());
    }

    #[test]
    fn msl_struct_text_matches() {
        let layout =
            ParamLayout::new(&[("tint", Param::Float4), ("distortion", Param::Float)]).unwrap();
        assert_eq!(
            layout.msl_struct(),
            "struct EffectParams {\n    float4 tint;\n    float distortion;\n};\n"
        );
    }

    #[test]
    fn set_writes_bytes_at_offset() {
        let mut layout =
            ParamLayout::new(&[("tint", Param::Float4), ("distortion", Param::Float)]).unwrap();
        layout.set("distortion", 0.5f32.into()).unwrap();
        layout.set("tint", [1., 0.5, 0.25, 1.].into()).unwrap();
        let bytes = layout.bytes();
        assert_eq!(&bytes[16..20], &0.5f32.to_le_bytes());
        assert_eq!(&bytes[0..4], &1f32.to_le_bytes());
        assert_eq!(&bytes[12..16], &1f32.to_le_bytes());
        assert_eq!(layout.size(), 32);

        // A float3 fills 12 bytes and leaves its padding alone; an int lands
        // little-endian.
        let mut layout = awkward();
        layout.set("b", [1., 2., 3.].into()).unwrap();
        layout.set("f", (-2i32).into()).unwrap();
        let bytes = layout.bytes();
        assert_eq!(&bytes[24..28], &3f32.to_le_bytes());
        assert_eq!(&bytes[28..32], &[0; 4]);
        assert_eq!(&bytes[64..68], &(-2i32).to_le_bytes());
    }

    #[test]
    fn set_unknown_name_is_error() {
        let mut layout = ParamLayout::new(&[("tint", Param::Float4)]).unwrap();
        assert_eq!(
            layout.set("tnt", 1f32.into()),
            Err(ParamError::UnknownName("tnt".into()))
        );
    }

    #[test]
    fn set_wrong_type_is_error() {
        let mut layout = ParamLayout::new(&[("tint", Param::Float4)]).unwrap();
        assert_eq!(
            layout.set("tint", 1f32.into()),
            Err(ParamError::TypeMismatch {
                name: "tint".into(),
                expected: Param::Float4,
                got: Param::Float,
            })
        );
    }

    #[test]
    fn params_rejects_duplicate_and_bad_identifier() {
        assert_eq!(
            ParamLayout::new(&[("a", Param::Float), ("a", Param::Int)]),
            Err(ParamError::DuplicateName("a".into()))
        );
        for bad in ["", "1a", "a-b", "a b", "_pad0", "_pad", "ü"] {
            assert_eq!(
                ParamLayout::new(&[(bad, Param::Float)]),
                Err(ParamError::BadIdentifier(bad.into())),
                "{bad:?}"
            );
        }
        assert!(ParamLayout::new(&[("_ok", Param::Float), ("a1", Param::Int)]).is_ok());
    }
}
