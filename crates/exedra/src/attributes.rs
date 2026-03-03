// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typed attribute layers across mesh domains.

use alloc::vec;
use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::Id;

/// Attribute domain.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Domain {
    /// Vertex-domain attributes.
    Vertex,
    /// Face-domain attributes.
    Face,
    /// Half-edge/corner-domain attributes.
    HalfEdge,
}

/// Typed attribute key scoped by domain and name.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct AttrKey<T> {
    domain: Domain,
    name: &'static str,
    marker: PhantomData<fn() -> T>,
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

/// Internal concrete storage variants used by [`Attributes`].
#[doc(hidden)]
#[derive(Clone, Debug)]
pub enum Layer {
    DenseVec3(DenseLayer<[f32; 3]>),
    DenseVec2(DenseLayer<[f32; 2]>),
    DenseF32(DenseLayer<f32>),
    DenseU32(DenseLayer<u32>),
    DenseBool(DenseLayer<bool>),
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

/// Attribute layer registration error.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AttrError {
    /// Layer name/domain already exists with a different type.
    TypeMismatch,
    /// Layer name/domain already exists with the same type.
    AlreadyExists,
}

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
                Layer::DenseVec3(layer) => layer.ensure_len(cap),
                Layer::DenseVec2(layer) => layer.ensure_len(cap),
                Layer::DenseF32(layer) => layer.ensure_len(cap),
                Layer::DenseU32(layer) => layer.ensure_len(cap),
                Layer::DenseBool(layer) => layer.ensure_len(cap),
                Layer::SparseVec3(_)
                | Layer::SparseVec2(_)
                | Layer::SparseF32(_)
                | Layer::SparseU32(_)
                | Layer::SparseBool(_) => {}
            }
        }
    }

    /// Defines a dense typed layer.
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
            domain: key.domain(),
            name: key.name(),
            layer: T::dense_new(len, default),
        });
        Ok(())
    }

    /// Defines a sparse typed layer.
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
            domain: key.domain(),
            name: key.name(),
            layer: T::sparse_new(),
        });
        Ok(())
    }

    /// Returns dense layer by typed key.
    #[must_use]
    pub fn dense<T: LayerValue>(&self, key: AttrKey<T>) -> Option<&DenseLayer<T>> {
        let entry = self.find_dense_entry(key.domain(), key.name())?;
        T::dense_ref(&entry.layer)
    }

    /// Returns mutable dense layer by typed key.
    #[must_use]
    pub fn dense_mut<T: LayerValue>(&mut self, key: AttrKey<T>) -> Option<&mut DenseLayer<T>> {
        let entry = self.find_dense_entry_mut(key.domain(), key.name())?;
        T::dense_mut(&mut entry.layer)
    }

    /// Returns sparse layer by typed key.
    #[must_use]
    pub fn sparse<T: LayerValue>(&self, key: AttrKey<T>) -> Option<&SparseLayer<T>> {
        let entry = self.find_sparse_entry(key.domain(), key.name())?;
        T::sparse_ref(&entry.layer)
    }

    /// Returns mutable sparse layer by typed key.
    #[must_use]
    pub fn sparse_mut<T: LayerValue>(&mut self, key: AttrKey<T>) -> Option<&mut SparseLayer<T>> {
        let entry = self.find_sparse_entry_mut(key.domain(), key.name())?;
        T::sparse_mut(&mut entry.layer)
    }

    fn find_dense_entry(&self, domain: Domain, name: &'static str) -> Option<&Entry> {
        self.dense
            .iter()
            .find(|entry| entry.domain == domain && entry.name == name)
    }

    fn find_dense_entry_mut(&mut self, domain: Domain, name: &'static str) -> Option<&mut Entry> {
        self.dense
            .iter_mut()
            .find(|entry| entry.domain == domain && entry.name == name)
    }

    fn find_sparse_entry(&self, domain: Domain, name: &'static str) -> Option<&Entry> {
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
                Layer::DenseVec3(layer) => layer.len(),
                Layer::DenseVec2(layer) => layer.len(),
                Layer::DenseF32(layer) => layer.len(),
                Layer::DenseU32(layer) => layer.len(),
                Layer::DenseBool(layer) => layer.len(),
                Layer::SparseVec3(_)
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
}

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
