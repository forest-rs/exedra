// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Extra attribute streams carried through render extraction.

use alloc::vec::Vec;

use crate::attributes::{AttrKey, Domain, Layer, LayerKind, sealed};
use crate::{CornerId, FaceId, Id, Mesh, VertexId};

/// Value type of a carried attribute stream.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum AttributeKind {
    /// One `f32` per render vertex.
    F32,
    /// Two `f32` components per render vertex.
    Vec2,
    /// Three `f32` components per render vertex.
    Vec3,
    /// Four `f32` components per render vertex.
    Vec4,
    /// One `u32` per render vertex.
    U32,
}

impl AttributeKind {
    /// Number of 32-bit components per value.
    #[must_use]
    pub const fn components(self) -> usize {
        match self {
            Self::F32 | Self::U32 => 1,
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 => 4,
        }
    }
}

/// One attribute value, typed by [`AttributeKind`].
///
/// Equality compares bit patterns, like render-vertex splitting and
/// `exedra_assembly`'s policy fingerprint: `NaN == NaN` when the bits match,
/// and `0.0 != -0.0`.
#[derive(Copy, Clone, Debug)]
pub enum AttributeValue {
    /// A scalar `f32`.
    F32(f32),
    /// A two-component vector.
    Vec2([f32; 2]),
    /// A three-component vector.
    Vec3([f32; 3]),
    /// A four-component vector.
    Vec4([f32; 4]),
    /// An unsigned integer.
    U32(u32),
}

impl PartialEq for AttributeValue {
    fn eq(&self, other: &Self) -> bool {
        self.kind() == other.kind() && self.bits() == other.bits()
    }
}

impl AttributeValue {
    /// Returns the kind of this value.
    #[must_use]
    pub const fn kind(&self) -> AttributeKind {
        match self {
            Self::F32(_) => AttributeKind::F32,
            Self::Vec2(_) => AttributeKind::Vec2,
            Self::Vec3(_) => AttributeKind::Vec3,
            Self::Vec4(_) => AttributeKind::Vec4,
            Self::U32(_) => AttributeKind::U32,
        }
    }

    /// Returns the component bit patterns, zero-padded to four words.
    fn bits(&self) -> [u32; 4] {
        let mut out = [0; 4];
        match self {
            Self::F32(v) => out[0] = v.to_bits(),
            Self::Vec2(v) => out[..2].copy_from_slice(&v.map(f32::to_bits)),
            Self::Vec3(v) => out[..3].copy_from_slice(&v.map(f32::to_bits)),
            Self::Vec4(v) => out = v.map(f32::to_bits),
            Self::U32(v) => out[0] = *v,
        }
        out
    }

    /// Appends the value's bit patterns, in component order, to `out`.
    fn push_bits(&self, out: &mut Vec<u32>) {
        out.extend_from_slice(&self.bits()[..self.kind().components()]);
    }
}

/// Value types that can be carried as extraction streams.
///
/// Implemented for `f32`, `[f32; 2]`, `[f32; 3]`, `[f32; 4]`, and `u32`. The
/// trait is sealed.
pub trait StreamValue: crate::attributes::LayerValue + sealed::Sealed {
    /// Wraps a value of this type.
    fn into_value(self) -> AttributeValue;
}

impl sealed::Sealed for f32 {}
impl StreamValue for f32 {
    fn into_value(self) -> AttributeValue {
        AttributeValue::F32(self)
    }
}

impl sealed::Sealed for [f32; 2] {}
impl StreamValue for [f32; 2] {
    fn into_value(self) -> AttributeValue {
        AttributeValue::Vec2(self)
    }
}

impl sealed::Sealed for [f32; 3] {}
impl StreamValue for [f32; 3] {
    fn into_value(self) -> AttributeValue {
        AttributeValue::Vec3(self)
    }
}

impl sealed::Sealed for [f32; 4] {}
impl StreamValue for [f32; 4] {
    fn into_value(self) -> AttributeValue {
        AttributeValue::Vec4(self)
    }
}

impl sealed::Sealed for u32 {}
impl StreamValue for u32 {
    fn into_value(self) -> AttributeValue {
        AttributeValue::U32(self)
    }
}

/// A mesh attribute layer to emit as an extra render-vertex stream.
///
/// The layer is looked up by its key's domain and name, dense or sparse, and
/// resolved per face corner:
///
/// - [`Domain::HalfEdge`] layers read the corner itself,
/// - [`Domain::Vertex`] layers read the corner's vertex,
/// - [`Domain::Face`] layers read the corner's face.
///
/// A corner without a value (a sparse gap, a missing layer, or a layer
/// registered with another value type) emits `missing`. Such fallbacks are
/// counted in [`ExtractStats`](crate::ExtractStats), never silent.
///
/// Carried values take part in render-vertex splitting exactly like UVs and
/// normals: two corners of one vertex share a render vertex only when every
/// carried value is bit-identical.
///
/// # Example
/// ```rust
/// use exedra_mesh::{ExtractAttribute, ExtractParams, attr};
///
/// let params = ExtractParams {
///     attributes: vec![
///         ExtractAttribute::new(attr::CORNER_UV1, [0.0, 0.0]),
///         ExtractAttribute::new(attr::CORNER_COLOR, [1.0, 1.0, 1.0, 1.0]),
///     ],
///     ..ExtractParams::default()
/// };
/// assert_eq!(params.attributes[1].name(), "corner.color");
/// ```
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExtractAttribute {
    domain: Domain,
    name: &'static str,
    missing: AttributeValue,
}

impl ExtractAttribute {
    /// Carries the layer named by `key`, emitting `missing` where it has no value.
    #[must_use]
    pub fn new<T: StreamValue>(key: AttrKey<T>, missing: T) -> Self {
        Self {
            domain: key.domain(),
            name: key.name(),
            missing: missing.into_value(),
        }
    }

    /// Returns the source layer's domain.
    #[must_use]
    pub const fn domain(&self) -> Domain {
        self.domain
    }

    /// Returns the source layer's name, which also names the output stream.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the stream's value kind.
    #[must_use]
    pub const fn kind(&self) -> AttributeKind {
        self.missing.kind()
    }

    /// Returns the value emitted where the layer has none.
    #[must_use]
    pub const fn missing(&self) -> AttributeValue {
        self.missing
    }
}

/// Values of one extracted attribute stream, parallel to
/// [`TriMesh::positions`](crate::TriMesh::positions).
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeBuffer {
    /// Scalar `f32` values.
    F32(Vec<f32>),
    /// Two-component values.
    Vec2(Vec<[f32; 2]>),
    /// Three-component values.
    Vec3(Vec<[f32; 3]>),
    /// Four-component values.
    Vec4(Vec<[f32; 4]>),
    /// Unsigned integer values.
    U32(Vec<u32>),
}

impl AttributeBuffer {
    fn empty(kind: AttributeKind) -> Self {
        match kind {
            AttributeKind::F32 => Self::F32(Vec::new()),
            AttributeKind::Vec2 => Self::Vec2(Vec::new()),
            AttributeKind::Vec3 => Self::Vec3(Vec::new()),
            AttributeKind::Vec4 => Self::Vec4(Vec::new()),
            AttributeKind::U32 => Self::U32(Vec::new()),
        }
    }

    /// Returns the value kind.
    #[must_use]
    pub const fn kind(&self) -> AttributeKind {
        match self {
            Self::F32(_) => AttributeKind::F32,
            Self::Vec2(_) => AttributeKind::Vec2,
            Self::Vec3(_) => AttributeKind::Vec3,
            Self::Vec4(_) => AttributeKind::Vec4,
            Self::U32(_) => AttributeKind::U32,
        }
    }

    /// Returns the number of values.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::F32(v) => v.len(),
            Self::Vec2(v) => v.len(),
            Self::Vec3(v) => v.len(),
            Self::Vec4(v) => v.len(),
            Self::U32(v) => v.len(),
        }
    }

    /// Returns true when the stream holds no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends one value decoded from its component bit patterns.
    fn push_bits(&mut self, bits: &[u32]) {
        let f = |i: usize| f32::from_bits(bits[i]);
        match self {
            Self::F32(v) => v.push(f(0)),
            Self::Vec2(v) => v.push([f(0), f(1)]),
            Self::Vec3(v) => v.push([f(0), f(1), f(2)]),
            Self::Vec4(v) => v.push([f(0), f(1), f(2), f(3)]),
            Self::U32(v) => v.push(bits[0]),
        }
    }
}

/// One extra render-vertex stream emitted by extraction.
///
/// A stream is identified by its source layer's domain and name, like the
/// layer itself: layers with one name in two domains are distinct.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeStream {
    /// Domain of the source layer.
    pub domain: Domain,
    /// Name of the source layer.
    pub name: &'static str,
    /// Values, one per render vertex.
    pub values: AttributeBuffer,
}

/// A carried attribute bound to its source layer for one extraction.
#[derive(Copy, Clone, Debug)]
pub(super) struct BoundAttribute<'a> {
    request: ExtractAttribute,
    /// `None` when the layer is absent or holds another value type.
    layer: Option<&'a Layer>,
}

impl<'a> BoundAttribute<'a> {
    pub(super) fn bind(mesh: &'a Mesh, request: ExtractAttribute) -> Self {
        let layer = mesh
            .attrs()
            .layer(request.domain, request.name)
            .filter(|layer| layer_kind(layer) == Some(request.kind()));
        Self { request, layer }
    }

    pub(super) fn is_bound(&self) -> bool {
        self.layer.is_some()
    }

    pub(super) fn empty_stream(&self) -> AttributeStream {
        AttributeStream {
            domain: self.request.domain,
            name: self.request.name,
            values: AttributeBuffer::empty(self.request.kind()),
        }
    }

    /// Appends the corner's value bits to `out`; returns false when the
    /// missing value was used.
    pub(super) fn push_corner_bits(
        &self,
        corner: CornerId,
        vertex: VertexId,
        face: FaceId,
        out: &mut Vec<u32>,
    ) -> bool {
        let id = match self.request.domain {
            Domain::HalfEdge => corner.as_id(),
            Domain::Vertex => vertex.as_id(),
            Domain::Face => face.as_id(),
        };
        if let Some(layer) = self.layer
            && push_layer_bits(layer, id, out)
        {
            return true;
        }
        self.request.missing.push_bits(out);
        false
    }
}

pub(super) fn push_stream_bits(stream: &mut AttributeStream, bits: &[u32]) {
    stream.values.push_bits(bits);
}

fn layer_kind(layer: &Layer) -> Option<AttributeKind> {
    match layer.kind() {
        LayerKind::F32 => Some(AttributeKind::F32),
        LayerKind::Vec2 => Some(AttributeKind::Vec2),
        LayerKind::Vec3 => Some(AttributeKind::Vec3),
        LayerKind::Vec4 => Some(AttributeKind::Vec4),
        LayerKind::U32 => Some(AttributeKind::U32),
        // Bool layers are not render streams; binding treats them as a
        // type mismatch.
        LayerKind::Bool => None,
    }
}

fn push_layer_bits(layer: &Layer, id: Id, out: &mut Vec<u32>) -> bool {
    fn floats<const N: usize>(value: Option<&[f32; N]>, out: &mut Vec<u32>) -> bool {
        value.is_some_and(|v| {
            out.extend(v.iter().map(|c| c.to_bits()));
            true
        })
    }
    match layer {
        Layer::DenseF32(l) => floats(l.get(id).map(core::array::from_ref), out),
        Layer::SparseF32(l) => floats(l.get(id).map(core::array::from_ref), out),
        Layer::DenseVec2(l) => floats(l.get(id), out),
        Layer::SparseVec2(l) => floats(l.get(id), out),
        Layer::DenseVec3(l) => floats(l.get(id), out),
        Layer::SparseVec3(l) => floats(l.get(id), out),
        Layer::DenseVec4(l) => floats(l.get(id), out),
        Layer::SparseVec4(l) => floats(l.get(id), out),
        Layer::DenseU32(l) => l.get(id).is_some_and(|v| {
            out.push(*v);
            true
        }),
        Layer::SparseU32(l) => l.get(id).is_some_and(|v| {
            out.push(*v);
            true
        }),
        // Unreachable: `BoundAttribute::bind` never binds a bool layer.
        Layer::DenseBool(_) | Layer::SparseBool(_) => false,
    }
}
