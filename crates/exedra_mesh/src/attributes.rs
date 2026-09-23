// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typed attribute storage across mesh domains.
//!
//! This module defines Exedra's attribute model:
//! - [`AttrKey<T>`]: typed key (domain + stable name),
//! - [`DenseLayer<T>`]: per-slot values with default fill,
//! - [`SparseLayer<T>`]: sparse overrides keyed by stable slot index,
//! - [`Attributes`]: registry and storage for all mesh-domain layers.
//!
//! Typical flow:
//! 1. Define a typed key (or use a built-in from [`attr`](crate::attr)).
//! 2. Register a layer with [`Attributes::define_dense`] or
//!    [`Attributes::define_sparse`].
//! 3. Access the layer through typed lookups (`dense*` / `sparse*`).
//! 4. Keep capacities synced to mesh slot counts via
//!    [`Attributes::sync_capacities`].
//!
//! Use [`Domain`] to choose where an attribute lives:
//! - [`Domain::Vertex`]: per-vertex data (for example weights or custom flags),
//! - [`Domain::Face`]: per-face data (for example region/material IDs),
//! - [`Domain::HalfEdge`]: per-corner/per-directed-edge data (for example UVs,
//!   seams, or sharpness).
//!
//! # Examples
//!
//! Registering and writing a custom face-region dense layer:
//! ```rust
//! use exedra_mesh::attributes::{AttrKey, Attributes, Domain};
//! use exedra_mesh::{FaceId, Id};
//! use core::num::NonZeroU32;
//!
//! let mut attrs = Attributes::new();
//! let custom_region = AttrKey::<u32>::new(Domain::Face, "custom.region");
//! attrs.define_dense(custom_region, 0)?;
//!
//! // Dense layers are indexed by stable slot index (`id.index()`).
//! attrs.sync_capacities(0, 4, 0);
//! let face = FaceId::from(Id::new(2, NonZeroU32::MIN));
//! attrs.dense_mut(custom_region).unwrap().set(face.as_id(), 7);
//! assert_eq!(attrs.dense(custom_region).unwrap().get(face.as_id()), Some(&7));
//! # Ok::<(), exedra_mesh::attributes::AttrError>(())
//! ```
//!
//! Registering and writing a sparse per-corner UV layer:
//! ```rust
//! use exedra_mesh::attributes::{AttrKey, Attributes, Domain};
//! use exedra_mesh::{CornerId, Id};
//! use core::num::NonZeroU32;
//!
//! let mut attrs = Attributes::new();
//! let custom_uv = AttrKey::<[f32; 2]>::new(Domain::HalfEdge, "custom.uv");
//! attrs.define_sparse(custom_uv)?;
//!
//! let corner = CornerId::from(Id::new(3, NonZeroU32::MIN));
//! attrs.sparse_mut(custom_uv).unwrap().set(corner.as_id(), [0.25, 0.75]);
//! assert_eq!(attrs.sparse(custom_uv).unwrap().get(corner.as_id()), Some(&[0.25, 0.75]));
//! # Ok::<(), exedra_mesh::attributes::AttrError>(())
//! ```
//!
//! Domain capacities are synchronized from mesh slot counts (not live counts),
//! so stable IDs remain indexable even after tombstones appear.
//!
//! Built-in keys (for example vertex position, corner UV, face region, seam,
//! sharpness) are declared in [`attr`](crate::attr).

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

use crate::Id;

/// Attribute domain.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Domain {
    /// Vertex-domain attributes (`VertexId` slots).
    Vertex,
    /// Face-domain attributes (`FaceId` slots).
    Face,
    /// Half-edge/corner-domain attributes (`HalfEdgeId`/`CornerId` slots).
    ///
    /// In Exedra, corners are represented by half-edge IDs.
    HalfEdge,
}

#[doc(hidden)]
pub mod sealed {
    /// Closes value and element traits to this crate's implementations.
    pub trait Sealed {}
}

/// An element handle that addresses one attribute [`Domain`].
///
/// Implemented for [`VertexId`](crate::VertexId), [`FaceId`](crate::FaceId),
/// and [`HalfEdgeId`](crate::HalfEdgeId) (corners). Generic attribute
/// operations use it to reject an ID from another domain instead of writing
/// whichever element shares its slot. The trait is sealed.
pub trait AttributeElement: sealed::Sealed + Copy {
    /// The domain this handle addresses.
    const DOMAIN: Domain;

    /// Returns the untyped slot ID.
    fn id(self) -> Id;
}

impl sealed::Sealed for crate::VertexId {}
impl AttributeElement for crate::VertexId {
    const DOMAIN: Domain = Domain::Vertex;
    fn id(self) -> Id {
        self.as_id()
    }
}

impl sealed::Sealed for crate::FaceId {}
impl AttributeElement for crate::FaceId {
    const DOMAIN: Domain = Domain::Face;
    fn id(self) -> Id {
        self.as_id()
    }
}

impl sealed::Sealed for crate::HalfEdgeId {}
impl AttributeElement for crate::HalfEdgeId {
    const DOMAIN: Domain = Domain::HalfEdge;
    fn id(self) -> Id {
        self.as_id()
    }
}

/// Typed attribute key scoped by domain and name.
///
/// Keys are `Copy`, `Eq`, and `Hash` for every value type: they hold only the
/// domain and name.
#[derive(Debug)]
pub struct AttrKey<T> {
    domain: Domain,
    name: &'static str,
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for AttrKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for AttrKey<T> {}

impl<T> PartialEq for AttrKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.domain == other.domain && self.name == other.name
    }
}

impl<T> Eq for AttrKey<T> {}

impl<T> core::hash::Hash for AttrKey<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.domain.hash(state);
        self.name.hash(state);
    }
}

impl<T> AttrKey<T> {
    /// Creates a new typed key.
    #[must_use]
    pub const fn new(domain: Domain, name: &'static str) -> Self {
        Self {
            domain,
            name,
            marker: PhantomData,
        }
    }

    /// Returns the key domain.
    #[must_use]
    pub const fn domain(&self) -> Domain {
        self.domain
    }

    /// Returns the key name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

/// Dense attribute layer with a default fill value.
#[derive(Clone, Debug)]
pub struct DenseLayer<T> {
    values: Vec<T>,
    default: T,
}

impl<T: Clone> DenseLayer<T> {
    /// Creates a dense layer of `len` values filled with `default`.
    #[must_use]
    pub fn with_len(len: usize, default: T) -> Self {
        Self {
            values: vec![default.clone(); len],
            default,
        }
    }

    /// Returns current logical length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Extends the layer to `len` by cloning the default value.
    pub fn ensure_len(&mut self, len: usize) {
        if len > self.values.len() {
            self.values.resize(len, self.default.clone());
        }
    }

    /// Returns value reference for a stable ID slot.
    #[must_use]
    pub fn get(&self, id: Id) -> Option<&T> {
        self.values.get(id.index() as usize)
    }

    /// Returns the value new and cleared slots hold.
    #[must_use]
    pub fn default(&self) -> &T {
        &self.default
    }

    /// Sets value for a stable ID slot.
    ///
    /// Returns `true` when set succeeds.
    pub fn set(&mut self, id: Id, value: T) -> bool {
        let Some(slot) = self.values.get_mut(id.index() as usize) else {
            return false;
        };
        *slot = value;
        true
    }
}

/// Sparse attribute layer keyed by stable ID slot index.
#[derive(Clone, Debug, Default)]
pub struct SparseLayer<T> {
    values: Vec<(u32, T)>,
}

impl<T> SparseLayer<T> {
    /// Creates an empty sparse layer.
    #[must_use]
    pub const fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// Returns value reference for a stable ID slot.
    #[must_use]
    pub fn get(&self, id: Id) -> Option<&T> {
        self.values
            .binary_search_by_key(&id.index(), |(index, _)| *index)
            .ok()
            .map(|position| &self.values[position].1)
    }

    /// Inserts or updates value for a stable ID slot.
    pub fn set(&mut self, id: Id, value: T) {
        match self
            .values
            .binary_search_by_key(&id.index(), |(index, _)| *index)
        {
            Ok(position) => self.values[position] = (id.index(), value),
            Err(position) => self.values.insert(position, (id.index(), value)),
        }
    }

    /// Removes a value for a stable ID slot.
    ///
    /// Returns the removed value when present.
    pub fn remove(&mut self, id: Id) -> Option<T> {
        self.values
            .binary_search_by_key(&id.index(), |(index, _)| *index)
            .ok()
            .map(|position| self.values.remove(position).1)
    }
}

/// Value type of a stored attribute layer.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum LayerKind {
    /// `f32` values.
    F32,
    /// `[f32; 2]` values.
    Vec2,
    /// `[f32; 3]` values.
    Vec3,
    /// `[f32; 4]` values.
    Vec4,
    /// `u32` values.
    U32,
    /// `bool` values.
    Bool,
}

/// One stored attribute value as canonical 32-bit words.
///
/// Float components are their bit patterns, `u32` is itself, and `bool` is
/// `0` or `1`. Returned by [`Attributes::value_words`] for content
/// fingerprints and other consumers that must read layers of any type.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct ValueWords {
    kind: LayerKind,
    words: [u32; 4],
    len: u8,
}

impl ValueWords {
    fn new(kind: LayerKind, value: &[u32]) -> Self {
        let mut words = [0; 4];
        words[..value.len()].copy_from_slice(value);
        Self {
            kind,
            words,
            len: u8::try_from(value.len()).expect("at most four words"),
        }
    }

    /// Returns the layer's value kind.
    #[must_use]
    pub const fn kind(&self) -> LayerKind {
        self.kind
    }

    /// Returns the value's words in component order.
    #[must_use]
    pub fn words(&self) -> &[u32] {
        &self.words[..usize::from(self.len)]
    }
}

/// How topology edits carry a caller-defined layer's values onto the elements
/// they create or rebuild.
///
/// Built-in layers ([`attr::RESERVED`](crate::attr::RESERVED)) keep their own
/// rules, chosen per edit through [`PropagatePolicy`](crate::PropagatePolicy).
/// Every other layer follows the rule declared with
/// [`Mesh::set_layer_propagation`](crate::Mesh::set_layer_propagation).
/// Deleted elements always lose their values, whatever the rule, so a
/// recycled slot never inherits a stale one.
///
/// Half-edge layers are treated as corner data: a half-edge's value belongs
/// to the corner at its destination vertex within its face. Edge-keyed
/// layers stored on one canonical half-edge per undirected edge, like
/// [`attr::EDGE_SEAM`](crate::attr::EDGE_SEAM), are outside this contract;
/// kernels may move or drop such values without counting them.
#[derive(Copy, Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum Propagation {
    /// No rule declared. Edits clear the values they cannot carry and count
    /// them in [`ChangeSet::unpropagated_attribute_values`](crate::ChangeSet::unpropagated_attribute_values).
    ///
    /// Under every rule, an element that fuses two sources (a face merged by
    /// [`op::dissolve_edges`](crate::op::dissolve_edges)) carries a value only
    /// when both agree, compared by value (so a NaN never agrees); a
    /// disagreement is cleared and counted.
    #[default]
    Unspecified,
    /// New and rebuilt elements get no value (a sparse gap, or the dense
    /// default). Intentional, so never counted.
    Clear,
    /// New and rebuilt elements copy the value of their source element, for
    /// example the corner at the same vertex of the original face.
    Copy,
    /// Like [`Self::Copy`], but where an edit places an element between two
    /// sources (the vertex and corners inserted by
    /// [`op::split_edge`](crate::op::split_edge)) the value is their blend.
    /// Float layers only. When only one blend source has a value, that value
    /// is carried, whichever side it is on; with neither, the element copies
    /// its source.
    Interpolate,
}

/// Internal concrete storage variants used by [`Attributes`].
#[doc(hidden)]
#[derive(Clone, Debug)]
pub enum Layer {
    DenseVec4(DenseLayer<[f32; 4]>),
    DenseVec3(DenseLayer<[f32; 3]>),
    DenseVec2(DenseLayer<[f32; 2]>),
    DenseF32(DenseLayer<f32>),
    DenseU32(DenseLayer<u32>),
    DenseBool(DenseLayer<bool>),
    SparseVec4(SparseLayer<[f32; 4]>),
    SparseVec3(SparseLayer<[f32; 3]>),
    SparseVec2(SparseLayer<[f32; 2]>),
    SparseF32(SparseLayer<f32>),
    SparseU32(SparseLayer<u32>),
    SparseBool(SparseLayer<bool>),
}

/// Internal mapping between Rust value types and concrete layer storage.
///
/// This trait is public to satisfy public method bounds on [`Attributes`], but
/// it is not intended for downstream implementations.
pub trait LayerValue: Clone + 'static {
    /// Creates a dense layer variant.
    fn dense_new(len: usize, default: Self) -> Layer;
    /// Downcasts a dense layer reference for this value type.
    fn dense_ref(layer: &Layer) -> Option<&DenseLayer<Self>>;
    /// Downcasts a mutable dense layer reference for this value type.
    fn dense_mut(layer: &mut Layer) -> Option<&mut DenseLayer<Self>>;
    /// Creates a sparse layer variant.
    fn sparse_new() -> Layer;
    /// Downcasts a sparse layer reference for this value type.
    fn sparse_ref(layer: &Layer) -> Option<&SparseLayer<Self>>;
    /// Downcasts a mutable sparse layer reference for this value type.
    fn sparse_mut(layer: &mut Layer) -> Option<&mut SparseLayer<Self>>;
}

impl Layer {
    pub(crate) fn kind(&self) -> LayerKind {
        match self {
            Self::DenseF32(_) | Self::SparseF32(_) => LayerKind::F32,
            Self::DenseVec2(_) | Self::SparseVec2(_) => LayerKind::Vec2,
            Self::DenseVec3(_) | Self::SparseVec3(_) => LayerKind::Vec3,
            Self::DenseVec4(_) | Self::SparseVec4(_) => LayerKind::Vec4,
            Self::DenseU32(_) | Self::SparseU32(_) => LayerKind::U32,
            Self::DenseBool(_) | Self::SparseBool(_) => LayerKind::Bool,
        }
    }
}

macro_rules! impl_layer_value {
    ($ty:ty, $dense_variant:ident, $sparse_variant:ident) => {
        impl LayerValue for $ty {
            fn dense_new(len: usize, default: Self) -> Layer {
                Layer::$dense_variant(DenseLayer::with_len(len, default))
            }

            fn dense_ref(layer: &Layer) -> Option<&DenseLayer<Self>> {
                match layer {
                    Layer::$dense_variant(value) => Some(value),
                    _ => None,
                }
            }

            fn dense_mut(layer: &mut Layer) -> Option<&mut DenseLayer<Self>> {
                match layer {
                    Layer::$dense_variant(value) => Some(value),
                    _ => None,
                }
            }

            fn sparse_new() -> Layer {
                Layer::$sparse_variant(SparseLayer::new())
            }

            fn sparse_ref(layer: &Layer) -> Option<&SparseLayer<Self>> {
                match layer {
                    Layer::$sparse_variant(value) => Some(value),
                    _ => None,
                }
            }

            fn sparse_mut(layer: &mut Layer) -> Option<&mut SparseLayer<Self>> {
                match layer {
                    Layer::$sparse_variant(value) => Some(value),
                    _ => None,
                }
            }
        }
    };
}

impl_layer_value!([f32; 4], DenseVec4, SparseVec4);
impl_layer_value!([f32; 3], DenseVec3, SparseVec3);
impl_layer_value!([f32; 2], DenseVec2, SparseVec2);
impl_layer_value!(f32, DenseF32, SparseF32);
impl_layer_value!(u32, DenseU32, SparseU32);
impl_layer_value!(bool, DenseBool, SparseBool);

#[derive(Clone, Debug)]
struct Entry {
    domain: Domain,
    name: &'static str,
    layer: Layer,
    propagation: Propagation,
}

/// Attribute storage for all mesh domains.
#[derive(Clone, Debug, Default)]
pub struct Attributes {
    vertex_capacity: usize,
    face_capacity: usize,
    half_edge_capacity: usize,
    dense: Vec<Entry>,
    sparse: Vec<Entry>,
}

/// Attribute layer registration error returned by [`Attributes`] definition APIs.
///
/// You receive this from [`Attributes::define_dense`] and
/// [`Attributes::define_sparse`] when a requested `(domain, name)` cannot be
/// registered with the provided type, and from
/// [`Mesh::define_dense_layer`](crate::Mesh::define_dense_layer) and
/// [`Mesh::define_sparse_layer`](crate::Mesh::define_sparse_layer) also
/// for reserved keys.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AttrError {
    /// `(domain, name)` already exists with a different Rust value type.
    TypeMismatch,
    /// `(domain, name)` already exists with the same Rust value type.
    AlreadyExists,
    /// `(domain, name)` is a built-in layer listed in
    /// [`attr::RESERVED`](crate::attr::RESERVED), which owns its registration.
    Reserved,
    /// No layer is registered under `(domain, name)`.
    NotDefined,
    /// [`Propagation::Interpolate`] was requested for a `u32` or `bool`
    /// layer, whose values cannot be blended.
    NotInterpolable,
}

impl fmt::Display for AttrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::TypeMismatch => "attribute layer exists with different type",
            Self::AlreadyExists => "attribute layer already exists",
            Self::Reserved => "attribute layer is reserved",
            Self::NotDefined => "attribute layer is not defined",
            Self::NotInterpolable => "attribute layer values cannot be interpolated",
        };
        f.write_str(text)
    }
}

impl core::error::Error for AttrError {}

impl Attributes {
    /// Creates a new attribute store with required built-ins registered.
    #[must_use]
    pub fn new() -> Self {
        let mut attrs = Self::default();
        let position = attrs.define_dense(crate::attr::VERTEX_POSITION, [0.0, 0.0, 0.0]);
        debug_assert!(position.is_ok(), "built-in VERTEX_POSITION must be unique");
        let region = attrs.define_dense(crate::attr::FACE_REGION, 0);
        debug_assert!(region.is_ok(), "built-in FACE_REGION must be unique");
        attrs
    }

    /// Returns domain capacity used for dense layers.
    #[must_use]
    pub const fn domain_capacity(&self, domain: Domain) -> usize {
        match domain {
            Domain::Vertex => self.vertex_capacity,
            Domain::Face => self.face_capacity,
            Domain::HalfEdge => self.half_edge_capacity,
        }
    }

    /// Syncs dense-layer capacities to domain slot counts.
    pub fn sync_capacities(&mut self, vertex: usize, face: usize, half_edge: usize) {
        self.vertex_capacity = vertex;
        self.face_capacity = face;
        self.half_edge_capacity = half_edge;

        for entry in &mut self.dense {
            let cap = match entry.domain {
                Domain::Vertex => vertex,
                Domain::Face => face,
                Domain::HalfEdge => half_edge,
            };
            match &mut entry.layer {
                Layer::DenseVec4(layer) => layer.ensure_len(cap),
                Layer::DenseVec3(layer) => layer.ensure_len(cap),
                Layer::DenseVec2(layer) => layer.ensure_len(cap),
                Layer::DenseF32(layer) => layer.ensure_len(cap),
                Layer::DenseU32(layer) => layer.ensure_len(cap),
                Layer::DenseBool(layer) => layer.ensure_len(cap),
                Layer::SparseVec4(_)
                | Layer::SparseVec3(_)
                | Layer::SparseVec2(_)
                | Layer::SparseF32(_)
                | Layer::SparseU32(_)
                | Layer::SparseBool(_) => {}
            }
        }
    }

    /// Defines a dense typed layer.
    ///
    /// Returns [`AttrError`] when the layer cannot be registered for the key.
    pub fn define_dense<T: LayerValue>(
        &mut self,
        key: AttrKey<T>,
        default: T,
    ) -> Result<(), AttrError> {
        if let Some(existing) = self.find_dense_entry(key.domain(), key.name()) {
            return if T::dense_ref(&existing.layer).is_some() {
                Err(AttrError::AlreadyExists)
            } else {
                Err(AttrError::TypeMismatch)
            };
        }
        if self.find_sparse_entry(key.domain(), key.name()).is_some() {
            return Err(AttrError::TypeMismatch);
        }
        let len = self.domain_capacity(key.domain());
        self.dense.push(Entry {
            propagation: Propagation::Unspecified,
            domain: key.domain(),
            name: key.name(),
            layer: T::dense_new(len, default),
        });
        Ok(())
    }

    /// Defines a sparse typed layer.
    ///
    /// Returns [`AttrError`] when the layer cannot be registered for the key.
    pub fn define_sparse<T: LayerValue>(&mut self, key: AttrKey<T>) -> Result<(), AttrError> {
        if let Some(existing) = self.find_sparse_entry(key.domain(), key.name()) {
            return if T::sparse_ref(&existing.layer).is_some() {
                Err(AttrError::AlreadyExists)
            } else {
                Err(AttrError::TypeMismatch)
            };
        }
        if self.find_dense_entry(key.domain(), key.name()).is_some() {
            return Err(AttrError::TypeMismatch);
        }
        self.sparse.push(Entry {
            propagation: Propagation::Unspecified,
            domain: key.domain(),
            name: key.name(),
            layer: T::sparse_new(),
        });
        Ok(())
    }

    /// Returns dense layer by typed key.
    ///
    /// Returns `None` when no dense layer is registered for this key, or when
    /// a layer exists under the same `(domain, name)` with a different type.
    #[must_use]
    pub fn dense<T: LayerValue>(&self, key: AttrKey<T>) -> Option<&DenseLayer<T>> {
        let entry = self.find_dense_entry(key.domain(), key.name())?;
        T::dense_ref(&entry.layer)
    }

    /// Returns mutable dense layer by typed key.
    ///
    /// Returns `None` under the same conditions as [`Self::dense`].
    #[must_use]
    pub fn dense_mut<T: LayerValue>(&mut self, key: AttrKey<T>) -> Option<&mut DenseLayer<T>> {
        let entry = self.find_dense_entry_mut(key.domain(), key.name())?;
        T::dense_mut(&mut entry.layer)
    }

    /// Returns sparse layer by typed key.
    ///
    /// Returns `None` when no sparse layer is registered for this key, or when
    /// a layer exists under the same `(domain, name)` with a different type.
    #[must_use]
    pub fn sparse<T: LayerValue>(&self, key: AttrKey<T>) -> Option<&SparseLayer<T>> {
        let entry = self.find_sparse_entry(key.domain(), key.name())?;
        T::sparse_ref(&entry.layer)
    }

    /// Returns mutable sparse layer by typed key.
    ///
    /// Returns `None` under the same conditions as [`Self::sparse`].
    #[must_use]
    pub fn sparse_mut<T: LayerValue>(&mut self, key: AttrKey<T>) -> Option<&mut SparseLayer<T>> {
        let entry = self.find_sparse_entry_mut(key.domain(), key.name())?;
        T::sparse_mut(&mut entry.layer)
    }

    /// Returns the propagation rule of the caller-defined layer registered
    /// under `(domain, name)`.
    ///
    /// Returns `None` for built-in keys ([`attr::RESERVED`](crate::attr::RESERVED)),
    /// whose rules come from [`PropagatePolicy`](crate::PropagatePolicy), and
    /// for unregistered layers.
    #[must_use]
    pub fn propagation(&self, domain: Domain, name: &str) -> Option<Propagation> {
        if crate::attr::is_reserved(domain, name) {
            return None;
        }
        self.find_dense_entry(domain, name)
            .or_else(|| self.find_sparse_entry(domain, name))
            .map(|entry| entry.propagation)
    }

    /// Sets the propagation rule of a registered layer.
    pub(crate) fn set_propagation(
        &mut self,
        domain: Domain,
        name: &str,
        propagation: Propagation,
    ) -> Result<(), AttrError> {
        let entry = self
            .dense
            .iter_mut()
            .chain(self.sparse.iter_mut())
            .find(|entry| entry.domain == domain && entry.name == name)
            .ok_or(AttrError::NotDefined)?;
        if propagation == Propagation::Interpolate
            && matches!(entry.layer.kind(), LayerKind::U32 | LayerKind::Bool)
        {
            return Err(AttrError::NotInterpolable);
        }
        entry.propagation = propagation;
        Ok(())
    }

    /// Returns the value kind of the layer registered under `(domain, name)`.
    #[must_use]
    pub fn layer_kind(&self, domain: Domain, name: &str) -> Option<LayerKind> {
        self.layer(domain, name).map(Layer::kind)
    }

    /// Returns the value stored at `id` in the layer registered under
    /// `(domain, name)`, dense or sparse.
    ///
    /// Returns `None` when no such layer exists or a sparse layer holds no
    /// value for `id`. A dense layer returns its value for any slot within its
    /// capacity, including its default.
    #[must_use]
    pub fn value_words(&self, domain: Domain, name: &str, id: Id) -> Option<ValueWords> {
        fn floats<const N: usize>(kind: LayerKind, v: Option<&[f32; N]>) -> Option<ValueWords> {
            v.map(|v| ValueWords::new(kind, &v.map(f32::to_bits)))
        }
        let layer = self.layer(domain, name)?;
        match layer {
            Layer::DenseF32(l) => floats(LayerKind::F32, l.get(id).map(core::array::from_ref)),
            Layer::SparseF32(l) => floats(LayerKind::F32, l.get(id).map(core::array::from_ref)),
            Layer::DenseVec2(l) => floats(LayerKind::Vec2, l.get(id)),
            Layer::SparseVec2(l) => floats(LayerKind::Vec2, l.get(id)),
            Layer::DenseVec3(l) => floats(LayerKind::Vec3, l.get(id)),
            Layer::SparseVec3(l) => floats(LayerKind::Vec3, l.get(id)),
            Layer::DenseVec4(l) => floats(LayerKind::Vec4, l.get(id)),
            Layer::SparseVec4(l) => floats(LayerKind::Vec4, l.get(id)),
            Layer::DenseU32(l) => l.get(id).map(|v| ValueWords::new(LayerKind::U32, &[*v])),
            Layer::SparseU32(l) => l.get(id).map(|v| ValueWords::new(LayerKind::U32, &[*v])),
            Layer::DenseBool(l) => l
                .get(id)
                .map(|v| ValueWords::new(LayerKind::Bool, &[u32::from(*v)])),
            Layer::SparseBool(l) => l
                .get(id)
                .map(|v| ValueWords::new(LayerKind::Bool, &[u32::from(*v)])),
        }
    }

    /// Returns the layer registered under `(domain, name)`, dense or sparse.
    pub(crate) fn layer(&self, domain: Domain, name: &str) -> Option<&Layer> {
        self.find_dense_entry(domain, name)
            .or_else(|| self.find_sparse_entry(domain, name))
            .map(|entry| &entry.layer)
    }

    fn find_dense_entry(&self, domain: Domain, name: &str) -> Option<&Entry> {
        self.dense
            .iter()
            .find(|entry| entry.domain == domain && entry.name == name)
    }

    fn find_dense_entry_mut(&mut self, domain: Domain, name: &'static str) -> Option<&mut Entry> {
        self.dense
            .iter_mut()
            .find(|entry| entry.domain == domain && entry.name == name)
    }

    fn find_sparse_entry(&self, domain: Domain, name: &str) -> Option<&Entry> {
        self.sparse
            .iter()
            .find(|entry| entry.domain == domain && entry.name == name)
    }

    fn find_sparse_entry_mut(&mut self, domain: Domain, name: &'static str) -> Option<&mut Entry> {
        self.sparse
            .iter_mut()
            .find(|entry| entry.domain == domain && entry.name == name)
    }

    /// Returns dense-layer capacity mismatches against domain capacities.
    #[must_use]
    pub fn dense_capacity_mismatches(&self) -> Vec<(Domain, &'static str, usize, usize)> {
        let mut mismatches = Vec::new();
        for entry in &self.dense {
            let expected = self.domain_capacity(entry.domain);
            let actual = match &entry.layer {
                Layer::DenseVec4(layer) => layer.len(),
                Layer::DenseVec3(layer) => layer.len(),
                Layer::DenseVec2(layer) => layer.len(),
                Layer::DenseF32(layer) => layer.len(),
                Layer::DenseU32(layer) => layer.len(),
                Layer::DenseBool(layer) => layer.len(),
                Layer::SparseVec4(_)
                | Layer::SparseVec3(_)
                | Layer::SparseVec2(_)
                | Layer::SparseF32(_)
                | Layer::SparseU32(_)
                | Layer::SparseBool(_) => continue,
            };
            if actual != expected {
                mismatches.push((entry.domain, entry.name, expected, actual));
            }
        }
        mismatches
    }

    /// Returns every registered layer's `(domain, name)` key, dense layers
    /// first then sparse layers, each group in registration order.
    ///
    /// Useful for auditing which attribute layers a mesh carries, for example
    /// to reject layers a consumer does not know how to preserve.
    pub fn keys(&self) -> impl Iterator<Item = (Domain, &'static str)> + '_ {
        self.dense
            .iter()
            .chain(self.sparse.iter())
            .map(|entry| (entry.domain, entry.name))
    }

    pub(crate) fn compacted(
        &self,
        vertex_map: &[Option<Id>],
        face_map: &[Option<Id>],
        half_edge_map: &[Option<Id>],
    ) -> Self {
        let mut compacted = Self {
            vertex_capacity: count_mapped(vertex_map),
            face_capacity: count_mapped(face_map),
            half_edge_capacity: count_mapped(half_edge_map),
            dense: Vec::with_capacity(self.dense.len()),
            sparse: Vec::with_capacity(self.sparse.len()),
        };

        for entry in &self.dense {
            let map = domain_map(entry.domain, vertex_map, face_map, half_edge_map);
            compacted.dense.push(Entry {
                propagation: entry.propagation,
                domain: entry.domain,
                name: entry.name,
                layer: compact_dense_layer(
                    &entry.layer,
                    map,
                    compacted.domain_capacity(entry.domain),
                ),
            });
        }

        for entry in &self.sparse {
            let map = domain_map(entry.domain, vertex_map, face_map, half_edge_map);
            compacted.sparse.push(Entry {
                propagation: entry.propagation,
                domain: entry.domain,
                name: entry.name,
                layer: compact_sparse_layer(&entry.layer, map),
            });
        }

        compacted
    }
}

fn count_mapped(map: &[Option<Id>]) -> usize {
    map.iter().flatten().count()
}

fn domain_map<'a>(
    domain: Domain,
    vertex_map: &'a [Option<Id>],
    face_map: &'a [Option<Id>],
    half_edge_map: &'a [Option<Id>],
) -> &'a [Option<Id>] {
    match domain {
        Domain::Vertex => vertex_map,
        Domain::Face => face_map,
        Domain::HalfEdge => half_edge_map,
    }
}

fn compact_dense_layer(layer: &Layer, map: &[Option<Id>], len: usize) -> Layer {
    match layer {
        Layer::DenseVec4(layer) => Layer::DenseVec4(compact_dense_values(layer, map, len)),
        Layer::DenseVec3(layer) => Layer::DenseVec3(compact_dense_values(layer, map, len)),
        Layer::DenseVec2(layer) => Layer::DenseVec2(compact_dense_values(layer, map, len)),
        Layer::DenseF32(layer) => Layer::DenseF32(compact_dense_values(layer, map, len)),
        Layer::DenseU32(layer) => Layer::DenseU32(compact_dense_values(layer, map, len)),
        Layer::DenseBool(layer) => Layer::DenseBool(compact_dense_values(layer, map, len)),
        Layer::SparseVec4(_)
        | Layer::SparseVec3(_)
        | Layer::SparseVec2(_)
        | Layer::SparseF32(_)
        | Layer::SparseU32(_)
        | Layer::SparseBool(_) => unreachable!("dense entries must store dense layers"),
    }
}

fn compact_dense_values<T: Clone>(
    layer: &DenseLayer<T>,
    map: &[Option<Id>],
    len: usize,
) -> DenseLayer<T> {
    let mut compacted = DenseLayer::with_len(len, layer.default.clone());
    for (old_index, new_id) in map.iter().enumerate() {
        let Some(new_id) = new_id else {
            continue;
        };
        let Some(value) = layer.values.get(old_index) else {
            continue;
        };
        compacted.values[new_id.index() as usize] = value.clone();
    }
    compacted
}

fn compact_sparse_layer(layer: &Layer, map: &[Option<Id>]) -> Layer {
    match layer {
        Layer::SparseVec4(layer) => Layer::SparseVec4(compact_sparse_values(layer, map)),
        Layer::SparseVec3(layer) => Layer::SparseVec3(compact_sparse_values(layer, map)),
        Layer::SparseVec2(layer) => Layer::SparseVec2(compact_sparse_values(layer, map)),
        Layer::SparseF32(layer) => Layer::SparseF32(compact_sparse_values(layer, map)),
        Layer::SparseU32(layer) => Layer::SparseU32(compact_sparse_values(layer, map)),
        Layer::SparseBool(layer) => Layer::SparseBool(compact_sparse_values(layer, map)),
        Layer::DenseVec4(_)
        | Layer::DenseVec3(_)
        | Layer::DenseVec2(_)
        | Layer::DenseF32(_)
        | Layer::DenseU32(_)
        | Layer::DenseBool(_) => unreachable!("sparse entries must store sparse layers"),
    }
}

fn compact_sparse_values<T: Clone>(layer: &SparseLayer<T>, map: &[Option<Id>]) -> SparseLayer<T> {
    let mut compacted = SparseLayer::new();
    for (old_index, value) in &layer.values {
        let Some(Some(new_id)) = map.get(*old_index as usize) else {
            continue;
        };
        compacted.set(*new_id, value.clone());
    }
    compacted
}

mod propagate;
pub(crate) use propagate::CallerValues;

#[cfg(test)]
mod tests {
    use core::num::NonZeroU32;

    use super::{AttrError, AttrKey, Attributes, Domain};
    use crate::{Id, attr};

    #[test]
    fn vertex_position_builtin_exists() {
        let attrs = Attributes::new();
        let layer = attrs
            .dense(attr::VERTEX_POSITION)
            .expect("builtin position layer");
        assert_eq!(layer.len(), 0);
    }

    #[test]
    fn dense_layers_track_domain_capacity() {
        let mut attrs = Attributes::new();
        attrs.sync_capacities(3, 2, 4);
        assert_eq!(
            attrs
                .dense(attr::VERTEX_POSITION)
                .expect("builtin position layer")
                .len(),
            3
        );

        let face_weight = AttrKey::<f32>::new(Domain::Face, "face.weight");
        assert_eq!(attrs.define_dense(face_weight, 1.0), Ok(()));
        assert_eq!(
            attrs
                .dense(face_weight)
                .expect("face weight layer should exist")
                .len(),
            2
        );
        attrs.sync_capacities(3, 5, 4);
        assert_eq!(
            attrs
                .dense(face_weight)
                .expect("face weight layer should resize")
                .len(),
            5
        );
    }

    #[test]
    fn dense_layer_get_and_set() {
        let mut attrs = Attributes::new();
        attrs.sync_capacities(2, 0, 0);
        let id = Id::new(1, NonZeroU32::MIN);
        let pos = attrs
            .dense(attr::VERTEX_POSITION)
            .expect("builtin position layer")
            .get(id)
            .expect("position slot");
        assert_eq!(*pos, [0.0, 0.0, 0.0]);

        let set_ok = attrs
            .dense_mut(attr::VERTEX_POSITION)
            .expect("builtin position layer")
            .set(id, [1.0, 2.0, 3.0]);
        assert!(set_ok);
        let pos = attrs
            .dense(attr::VERTEX_POSITION)
            .expect("builtin position layer")
            .get(id)
            .expect("position slot");
        assert_eq!(*pos, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn sparse_layer_registration_and_access() {
        let mut attrs = Attributes::new();
        let seam_key = AttrKey::<bool>::new(Domain::HalfEdge, "edge.seam");
        assert_eq!(attrs.define_sparse(seam_key), Ok(()));

        let id = Id::new(4, NonZeroU32::MIN);
        assert!(
            attrs
                .sparse(seam_key)
                .expect("sparse layer should exist")
                .get(id)
                .is_none()
        );
        attrs
            .sparse_mut(seam_key)
            .expect("sparse layer should exist")
            .set(id, true);
        assert_eq!(
            attrs
                .sparse(seam_key)
                .expect("sparse layer should exist")
                .get(id),
            Some(&true)
        );
    }

    #[test]
    fn corner_uv_builtin_key_has_expected_shape() {
        assert_eq!(attr::CORNER_UV.domain(), Domain::HalfEdge);
        assert_eq!(attr::CORNER_UV.name(), "corner.uv");
    }

    #[test]
    fn vertex_sharpness_builtin_key_has_expected_shape() {
        assert_eq!(attr::VERTEX_SHARPNESS.domain(), Domain::Vertex);
        assert_eq!(attr::VERTEX_SHARPNESS.name(), "vertex.sharpness");
    }

    #[test]
    fn corner_normal_override_builtin_key_has_expected_shape() {
        assert_eq!(attr::CORNER_NORMAL_OVERRIDE.domain(), Domain::HalfEdge);
        assert_eq!(
            attr::CORNER_NORMAL_OVERRIDE.name(),
            "corner.normal_override"
        );
    }

    #[test]
    fn edge_seam_builtin_key_has_expected_shape() {
        assert_eq!(attr::EDGE_SEAM.domain(), Domain::HalfEdge);
        assert_eq!(attr::EDGE_SEAM.name(), "edge.seam");
    }

    #[test]
    fn edge_sharpness_builtin_key_has_expected_shape() {
        assert_eq!(attr::EDGE_SHARPNESS.domain(), Domain::HalfEdge);
        assert_eq!(attr::EDGE_SHARPNESS.name(), "edge.sharpness");
    }

    #[test]
    fn face_region_builtin_key_has_expected_shape() {
        assert_eq!(attr::FACE_REGION.domain(), Domain::Face);
        assert_eq!(attr::FACE_REGION.name(), "face.region");
    }

    #[test]
    fn face_region_builtin_defaults_to_untagged() {
        let mut attrs = Attributes::new();
        attrs.sync_capacities(0, 2, 0);
        let face0 = Id::new(0, NonZeroU32::MIN);
        let face1 = Id::new(1, NonZeroU32::MIN);
        let regions = attrs
            .dense(attr::FACE_REGION)
            .expect("builtin face region layer");
        assert_eq!(regions.get(face0), Some(&0));
        assert_eq!(regions.get(face1), Some(&0));
    }

    #[test]
    fn corner_uv_sparse_layer_supports_partial_coverage() {
        let mut attrs = Attributes::new();
        assert_eq!(attrs.define_sparse(attr::CORNER_UV), Ok(()));

        let a = Id::new(1, NonZeroU32::MIN);
        let b = Id::new(3, NonZeroU32::MIN);

        assert_eq!(
            attrs
                .sparse(attr::CORNER_UV)
                .expect("corner uv layer should exist")
                .get(a),
            None
        );
        assert_eq!(
            attrs
                .sparse(attr::CORNER_UV)
                .expect("corner uv layer should exist")
                .get(b),
            None
        );

        attrs
            .sparse_mut(attr::CORNER_UV)
            .expect("corner uv layer should exist")
            .set(a, [0.25, 0.75]);

        assert_eq!(
            attrs
                .sparse(attr::CORNER_UV)
                .expect("corner uv layer should exist")
                .get(a),
            Some(&[0.25, 0.75])
        );
        assert_eq!(
            attrs
                .sparse(attr::CORNER_UV)
                .expect("corner uv layer should exist")
                .get(b),
            None
        );
    }

    #[test]
    fn duplicate_key_registration_is_rejected() {
        let mut attrs = Attributes::new();
        assert_eq!(
            attrs.define_dense(attr::VERTEX_POSITION, [0.0, 0.0, 0.0]),
            Err(AttrError::AlreadyExists)
        );
        assert_eq!(
            attrs.define_sparse(AttrKey::<f32>::new(Domain::Vertex, "vertex.position")),
            Err(AttrError::TypeMismatch)
        );
    }
}
