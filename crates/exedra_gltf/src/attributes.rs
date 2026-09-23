// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mapping of extracted vertex streams to glTF primitive attributes.

use exedra_mesh::attributes::{AttrKey, Domain};
use exedra_mesh::{AttributeBuffer, AttributeStream, StreamValue, TriMesh};
use serde_json::{Value, json};

use crate::GltfError;

/// glTF attribute semantic for a mapped vertex stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum GltfSemantic {
    /// `TEXCOORD_n` for `n >= 1`; `TEXCOORD_0` is the primary UV set.
    /// Requires a `[f32; 2]` stream.
    TexCoord(u32),
    /// `COLOR_n`, linear RGB or RGBA. Requires a `[f32; 3]` or `[f32; 4]`
    /// stream.
    Color(u32),
    /// An application-specific attribute. glTF requires the name to start
    /// with `_`, for example `_WIND_PIVOT`.
    Custom(String),
}

impl GltfSemantic {
    /// The attribute name written into the primitive.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::TexCoord(set) => format!("TEXCOORD_{set}"),
            Self::Color(set) => format!("COLOR_{set}"),
            Self::Custom(name) => name.clone(),
        }
    }
}

/// How `u32` streams are written.
///
/// glTF reserves `UNSIGNED_INT` for indices, so vertex attributes cannot
/// carry full 32-bit integers. Each encoding is exact within its range, and
/// export fails with [`GltfError::UnrepresentableAttribute`] outside it,
/// rather than rounding silently.
#[derive(Copy, Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum IntegerEncoding {
    /// `FLOAT` components, exact for values up to `2^24`.
    #[default]
    Float,
    /// `UNSIGNED_SHORT` components, exact for values up to `65535`.
    UnsignedShort,
}

/// One extracted stream exported as a glTF primitive attribute.
///
/// Streams are matched by their source layer's domain and name, as recorded
/// on [`AttributeStream`]. A mapping whose stream is absent from a body is skipped for that body and
/// counted in [`GltfStats::missing_attribute_streams`](crate::GltfStats::missing_attribute_streams).
///
/// # Example
///
/// ```
/// use exedra_gltf::{GltfAttribute, GltfExportOptions};
/// use exedra_mesh::attr;
/// use exedra_mesh::attributes::{AttrKey, Domain};
///
/// const PIVOT: AttrKey<[f32; 4]> = AttrKey::new(Domain::Vertex, "vertex.pivot");
///
/// let mappings = [
///     GltfAttribute::tex_coord(attr::CORNER_UV1, 1),
///     GltfAttribute::color(attr::CORNER_COLOR, 0),
///     GltfAttribute::custom(PIVOT, "_WIND_PIVOT"),
/// ];
/// let options = GltfExportOptions::z_up_to_y_up().with_attributes(&mappings);
/// assert_eq!(options.attributes.len(), 3);
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GltfAttribute {
    /// Domain of the extracted stream's source layer.
    pub domain: Domain,
    /// Name of the extracted stream's source layer, for example
    /// `"corner.uv1"`.
    pub stream: &'static str,
    /// Semantic the stream is written under.
    pub semantic: GltfSemantic,
    /// Encoding used when the stream holds `u32` values.
    pub integers: IntegerEncoding,
}

impl GltfAttribute {
    /// Maps the stream of layer `key` to `TEXCOORD_set`.
    #[must_use]
    pub fn tex_coord(key: AttrKey<[f32; 2]>, set: u32) -> Self {
        Self::new(key, GltfSemantic::TexCoord(set))
    }

    /// Maps the stream of layer `key` to `COLOR_set`.
    #[must_use]
    pub fn color<T: StreamValue>(key: AttrKey<T>, set: u32) -> Self {
        Self::new(key, GltfSemantic::Color(set))
    }

    /// Maps the stream of layer `key` to an application-specific `name`,
    /// which must start with `_`.
    #[must_use]
    pub fn custom<T: StreamValue>(key: AttrKey<T>, name: impl Into<String>) -> Self {
        Self::new(key, GltfSemantic::Custom(name.into()))
    }

    /// True when this mapping selects `stream`.
    #[must_use]
    pub fn selects(&self, stream: &AttributeStream) -> bool {
        self.domain == stream.domain && self.stream == stream.name
    }

    /// Selects the encoding for a `u32` stream.
    #[must_use]
    pub fn with_integers(mut self, integers: IntegerEncoding) -> Self {
        self.integers = integers;
        self
    }

    fn new<T>(key: AttrKey<T>, semantic: GltfSemantic) -> Self {
        Self {
            domain: key.domain(),
            stream: key.name(),
            semantic,
            integers: IntegerEncoding::default(),
        }
    }
}

/// Checks the mapping list once per export, before any geometry is written.
pub(crate) fn validate_mappings(mappings: &[GltfAttribute]) -> Result<(), GltfError> {
    for (i, mapping) in mappings.iter().enumerate() {
        let reason = match &mapping.semantic {
            GltfSemantic::TexCoord(0) => Some("TEXCOORD_0 is the primary UV set"),
            GltfSemantic::Custom(name) if !name.starts_with('_') => {
                Some("custom semantics must start with an underscore")
            }
            GltfSemantic::Custom(name) if name.len() < 2 => Some("custom semantic is empty"),
            _ => None,
        };
        let duplicate = mappings[..i]
            .iter()
            .any(|earlier| earlier.semantic == mapping.semantic);
        let reason = reason.or(duplicate.then_some("semantic is mapped more than once"));
        // glTF indexed semantics start at 0 and are contiguous; TEXCOORD_0 is
        // always the primary UV set.
        let lower_missing = |lower: GltfSemantic| !mappings.iter().any(|m| m.semantic == lower);
        let gap = match mapping.semantic {
            GltfSemantic::TexCoord(set) if set > 1 => {
                lower_missing(GltfSemantic::TexCoord(set - 1))
            }
            GltfSemantic::Color(set) if set > 0 => lower_missing(GltfSemantic::Color(set - 1)),
            _ => false,
        };
        let reason = reason.or(gap.then_some("indexed sets must be contiguous from the first set"));
        if let Some(reason) = reason {
            return Err(GltfError::InvalidAttributeMapping {
                stream: mapping.stream,
                semantic: mapping.semantic.name(),
                reason,
            });
        }
    }
    Ok(())
}

/// Encoded bytes and accessor fields for one mapped stream.
pub(crate) struct EncodedStream {
    pub(crate) semantic: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) accessor: Value,
    /// Buffer-view stride, when elements are padded for alignment.
    pub(crate) byte_stride: Option<usize>,
}

/// Where a stream is being encoded, for error reports.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Site {
    pub(crate) part: u32,
    pub(crate) body: usize,
}

/// Encodes the mapped streams present in `tri`, in mapping order.
///
/// Returns the encoded streams and the number of mappings whose stream the
/// body does not carry.
pub(crate) fn encode(
    tri: &TriMesh,
    mappings: &[GltfAttribute],
    site: Site,
) -> Result<(Vec<EncodedStream>, u64), GltfError> {
    let mut encoded = Vec::new();
    let mut missing = 0;
    let present = |mapping: &GltfAttribute| tri.attributes.iter().any(|s| mapping.selects(s));
    for mapping in mappings {
        let Some(values) = tri
            .attributes
            .iter()
            .find(|stream| mapping.selects(stream))
            .map(|stream| &stream.values)
        else {
            // A missing top set only shortens the run; a missing lower set
            // would leave a gap, which glTF forbids.
            let higher = |m: &&GltfAttribute| match (&mapping.semantic, &m.semantic) {
                (GltfSemantic::TexCoord(a), GltfSemantic::TexCoord(b))
                | (GltfSemantic::Color(a), GltfSemantic::Color(b)) => b > a,
                _ => false,
            };
            if mappings.iter().filter(higher).any(present) {
                return Err(GltfError::AttributeSetGap {
                    stream: mapping.stream,
                    semantic: mapping.semantic.name(),
                    part: site.part,
                    body: site.body,
                });
            }
            missing += 1;
            continue;
        };
        encoded.push(encode_one(mapping, values, site)?);
    }
    Ok((encoded, missing))
}

fn encode_one(
    mapping: &GltfAttribute,
    values: &AttributeBuffer,
    site: Site,
) -> Result<EncodedStream, GltfError> {
    let semantic = mapping.semantic.name();
    let mismatch = || GltfError::AttributeTypeMismatch {
        stream: mapping.stream,
        semantic: semantic.clone(),
        part: site.part,
        body: site.body,
    };
    let floats = |components: &[f32], kind: &str| {
        let mut bytes = Vec::with_capacity(components.len() * 4);
        for c in components {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        (bytes, json!({ "componentType": 5126, "type": kind }))
    };
    let (bytes, mut accessor) = match (&mapping.semantic, values) {
        (GltfSemantic::TexCoord(_), AttributeBuffer::Vec2(v)) => floats(v.as_flattened(), "VEC2"),
        (GltfSemantic::Color(_), AttributeBuffer::Vec3(v)) => floats(v.as_flattened(), "VEC3"),
        (GltfSemantic::Color(_), AttributeBuffer::Vec4(v)) => floats(v.as_flattened(), "VEC4"),
        (GltfSemantic::TexCoord(_) | GltfSemantic::Color(_), _) => return Err(mismatch()),
        (GltfSemantic::Custom(_), AttributeBuffer::F32(v)) => floats(v, "SCALAR"),
        (GltfSemantic::Custom(_), AttributeBuffer::Vec2(v)) => floats(v.as_flattened(), "VEC2"),
        (GltfSemantic::Custom(_), AttributeBuffer::Vec3(v)) => floats(v.as_flattened(), "VEC3"),
        (GltfSemantic::Custom(_), AttributeBuffer::Vec4(v)) => floats(v.as_flattened(), "VEC4"),
        (GltfSemantic::Custom(_), AttributeBuffer::U32(v)) => {
            encode_integers(v, mapping, &semantic, site)?
        }
    };
    accessor["count"] = json!(values.len());
    let byte_stride = matches!(
        (values, mapping.integers),
        (AttributeBuffer::U32(_), IntegerEncoding::UnsignedShort)
    )
    .then_some(4);
    Ok(EncodedStream {
        semantic,
        bytes,
        accessor,
        byte_stride,
    })
}

fn encode_integers(
    values: &[u32],
    mapping: &GltfAttribute,
    semantic: &str,
    site: Site,
) -> Result<(Vec<u8>, Value), GltfError> {
    let limit = match mapping.integers {
        IntegerEncoding::Float => 1_u32 << 24,
        IntegerEncoding::UnsignedShort => u32::from(u16::MAX),
    };
    if let Some(&value) = values.iter().find(|&&v| v > limit) {
        return Err(GltfError::UnrepresentableAttribute {
            stream: mapping.stream,
            semantic: semantic.to_owned(),
            part: site.part,
            body: site.body,
            value,
        });
    }
    Ok(match mapping.integers {
        IntegerEncoding::Float => {
            let mut bytes = Vec::with_capacity(values.len() * 4);
            for &v in values {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "values are checked to be at most 2^24, which f32 represents exactly"
                )]
                let f = v as f32;
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            (bytes, json!({ "componentType": 5126, "type": "SCALAR" }))
        }
        IntegerEncoding::UnsignedShort => {
            // Vertex attribute elements must start on four-byte boundaries,
            // so each two-byte value is padded and the view declares stride 4.
            let mut bytes = Vec::with_capacity(values.len() * 4);
            for &v in values {
                let short = u16::try_from(v).expect("checked against u16::MAX");
                bytes.extend_from_slice(&short.to_le_bytes());
                bytes.extend_from_slice(&[0, 0]);
            }
            (bytes, json!({ "componentType": 5123, "type": "SCALAR" }))
        }
    })
}
