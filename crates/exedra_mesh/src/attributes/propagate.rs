// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Untyped propagation of caller-defined layers for topology kernels.
//!
//! Built-in layers keep their dedicated propagation code; everything here
//! skips [`attr::RESERVED`](crate::attr::RESERVED) keys.

use alloc::vec::Vec;

use super::{Attributes, DenseLayer, Domain, Entry, Layer, Propagation, SparseLayer};
use crate::Id;

/// One stored value, typed by its layer.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Sample {
    F32(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    U32(u32),
    Bool(bool),
}

/// Values of every caller-defined layer at one element, captured before an
/// edit deletes it and restored onto the element that replaces it.
#[derive(Clone, Debug, Default)]
pub(crate) struct CallerValues {
    values: Vec<(&'static str, Captured)>,
}

/// One captured layer value.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Captured {
    /// The value, or `None` for a sparse gap or dense default.
    Value(Option<Sample>),
    /// Two merged sources disagreed; nothing can be carried.
    Conflict,
}

impl CallerValues {
    /// Merges the values of two elements an edit fuses into one: agreeing
    /// layers carry their value, disagreeing ones carry nothing and count.
    pub(crate) fn agreed(a: &Self, b: &Self) -> Self {
        let values = a
            .values
            .iter()
            .map(|&(name, value)| {
                let other = b
                    .values
                    .iter()
                    .find(|(other, _)| *other == name)
                    .map(|&(_, value)| value);
                let merged = if other == Some(value) {
                    value
                } else {
                    Captured::Conflict
                };
                (name, merged)
            })
            .collect();
        Self { values }
    }
}

/// What one propagation step could not carry.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Carry {
    /// Values the step cleared instead of carrying: layers with
    /// [`Propagation::Unspecified`], and merged sources that disagreed.
    pub(crate) unpropagated: u64,
}

trait Value: Clone + PartialEq {
    fn blend(a: &Self, wa: f32, b: &Self, wb: f32) -> Option<Self>;
    fn sample(&self) -> Sample;
    fn from_sample(sample: Sample) -> Option<Self>;
}

impl Value for f32 {
    fn blend(a: &Self, wa: f32, b: &Self, wb: f32) -> Option<Self> {
        Some(a * wa + b * wb)
    }
    fn sample(&self) -> Sample {
        Sample::F32(*self)
    }
    fn from_sample(sample: Sample) -> Option<Self> {
        match sample {
            Sample::F32(v) => Some(v),
            _ => None,
        }
    }
}

macro_rules! impl_vector_value {
    ($n:literal, $variant:ident) => {
        impl Value for [f32; $n] {
            fn blend(a: &Self, wa: f32, b: &Self, wb: f32) -> Option<Self> {
                Some(core::array::from_fn(|i| a[i] * wa + b[i] * wb))
            }
            fn sample(&self) -> Sample {
                Sample::$variant(*self)
            }
            fn from_sample(sample: Sample) -> Option<Self> {
                match sample {
                    Sample::$variant(v) => Some(v),
                    _ => None,
                }
            }
        }
    };
}

impl_vector_value!(2, Vec2);
impl_vector_value!(3, Vec3);
impl_vector_value!(4, Vec4);

impl Value for u32 {
    fn blend(_: &Self, _: f32, _: &Self, _: f32) -> Option<Self> {
        None
    }
    fn sample(&self) -> Sample {
        Sample::U32(*self)
    }
    fn from_sample(sample: Sample) -> Option<Self> {
        match sample {
            Sample::U32(v) => Some(v),
            _ => None,
        }
    }
}

impl Value for bool {
    fn blend(_: &Self, _: f32, _: &Self, _: f32) -> Option<Self> {
        None
    }
    fn sample(&self) -> Sample {
        Sample::Bool(*self)
    }
    fn from_sample(sample: Sample) -> Option<Self> {
        match sample {
            Sample::Bool(v) => Some(v),
            _ => None,
        }
    }
}

/// Uniform access to dense and sparse storage.
trait Store {
    type V: Value;
    /// The stored value; dense layers always have one.
    fn value(&self, id: Id) -> Option<Self::V>;
    /// Whether the slot carries information: a sparse value, or a dense value
    /// other than the default.
    fn present(&self, id: Id) -> bool;
    /// Writes `value`; `None` clears (sparse gap or dense default).
    fn put(&mut self, id: Id, value: Option<Self::V>);
}

impl<T: Value> Store for DenseLayer<T> {
    type V = T;
    fn value(&self, id: Id) -> Option<T> {
        self.get(id).cloned()
    }
    fn present(&self, id: Id) -> bool {
        self.get(id).is_some_and(|v| *v != self.default)
    }
    fn put(&mut self, id: Id, value: Option<T>) {
        let value = value.unwrap_or_else(|| self.default.clone());
        let _ = self.set(id, value);
    }
}

impl<T: Value> Store for SparseLayer<T> {
    type V = T;
    fn value(&self, id: Id) -> Option<T> {
        self.get(id).cloned()
    }
    fn present(&self, id: Id) -> bool {
        self.get(id).is_some()
    }
    fn put(&mut self, id: Id, value: Option<T>) {
        match value {
            Some(value) => self.set(id, value),
            None => {
                let _ = self.remove(id);
            }
        }
    }
}

macro_rules! each_layer {
    ($layer:expr, $store:ident => $body:expr) => {
        match $layer {
            Layer::DenseF32($store) => $body,
            Layer::DenseVec2($store) => $body,
            Layer::DenseVec3($store) => $body,
            Layer::DenseVec4($store) => $body,
            Layer::DenseU32($store) => $body,
            Layer::DenseBool($store) => $body,
            Layer::SparseF32($store) => $body,
            Layer::SparseVec2($store) => $body,
            Layer::SparseVec3($store) => $body,
            Layer::SparseVec4($store) => $body,
            Layer::SparseU32($store) => $body,
            Layer::SparseBool($store) => $body,
        }
    };
}

fn propagate_in<S: Store>(
    store: &mut S,
    rule: Propagation,
    target: Id,
    source: Id,
    blend: Option<[(Id, f32); 2]>,
) -> bool {
    match rule {
        Propagation::Clear => {
            store.put(target, None);
            false
        }
        Propagation::Unspecified => {
            // Anything a declared rule could have carried is lost: the
            // source, a blend endpoint, or the target's own value.
            let endpoints =
                blend.is_some_and(|[(a, _), (b, _)]| store.present(a) || store.present(b));
            let lost = store.present(source) || store.present(target) || endpoints;
            store.put(target, None);
            lost
        }
        Propagation::Copy => {
            let value = store.value(source);
            store.put(target, value);
            false
        }
        Propagation::Interpolate => {
            // Blend both endpoints; with one, carry that one whichever side it
            // is on; with neither, fall back to the source.
            let value = match blend {
                Some([(a, wa), (b, wb)]) => match (store.value(a), store.value(b)) {
                    (Some(va), Some(vb)) => S::V::blend(&va, wa, &vb, wb).or(Some(va)),
                    (Some(v), None) | (None, Some(v)) => Some(v),
                    (None, None) => store.value(source),
                },
                None => store.value(source),
            };
            store.put(target, value);
            false
        }
    }
}

fn restore_in<S: Store>(store: &mut S, rule: Propagation, target: Id, captured: Captured) -> bool {
    let Captured::Value(sample) = captured else {
        store.put(target, None);
        return rule != Propagation::Clear;
    };
    let value = sample.and_then(S::V::from_sample);
    match rule {
        Propagation::Clear => {
            store.put(target, None);
            false
        }
        Propagation::Unspecified => {
            let lost = value.is_some() || store.present(target);
            store.put(target, None);
            lost
        }
        Propagation::Copy | Propagation::Interpolate => {
            store.put(target, value);
            false
        }
    }
}

fn is_caller(entry: &Entry, domain: Domain) -> bool {
    entry.domain == domain && !crate::attr::is_reserved(entry.domain, entry.name)
}

impl Attributes {
    fn caller_entries_mut(&mut self, domain: Domain) -> impl Iterator<Item = &mut Entry> + '_ {
        self.dense
            .iter_mut()
            .chain(self.sparse.iter_mut())
            .filter(move |entry| is_caller(entry, domain))
    }

    /// Clears every caller-defined value at a deleted element's slot and
    /// returns how many carried information.
    pub(crate) fn clear_caller_values(&mut self, domain: Domain, id: Id) -> u64 {
        let mut cleared = 0;
        for entry in self.caller_entries_mut(domain) {
            let present = each_layer!(&mut entry.layer, store => {
                let present = store.present(id);
                store.put(id, None);
                present
            });
            cleared += u64::from(present);
        }
        cleared
    }

    /// Carries every caller-defined layer of `domain` onto `target` from
    /// `source`, or from the weighted `blend` of two elements for
    /// [`Propagation::Interpolate`].
    pub(crate) fn propagate_caller(
        &mut self,
        domain: Domain,
        target: Id,
        source: Id,
        blend: Option<[(Id, f32); 2]>,
    ) -> Carry {
        let mut carry = Carry::default();
        for entry in self.caller_entries_mut(domain) {
            let rule = entry.propagation;
            let lost = each_layer!(&mut entry.layer, store => {
                propagate_in(store, rule, target, source, blend)
            });
            carry.unpropagated += u64::from(lost);
        }
        carry
    }

    /// Captures every caller-defined value of `domain` at `id`.
    pub(crate) fn capture_caller(&self, domain: Domain, id: Id) -> CallerValues {
        let values = self
            .dense
            .iter()
            .chain(self.sparse.iter())
            .filter(|entry| is_caller(entry, domain))
            .map(|entry| {
                let sample = each_layer!(&entry.layer, store => {
                    store.present(id).then(|| store.value(id)).flatten().map(|v| v.sample())
                });
                (entry.name, Captured::Value(sample))
            })
            .collect();
        CallerValues { values }
    }

    /// Restores captured values onto `target` according to each layer's rule.
    pub(crate) fn restore_caller(
        &mut self,
        domain: Domain,
        target: Id,
        captured: &CallerValues,
    ) -> Carry {
        let mut carry = Carry::default();
        for entry in self.caller_entries_mut(domain) {
            let rule = entry.propagation;
            let value = captured
                .values
                .iter()
                .find(|(name, _)| *name == entry.name)
                .map_or(Captured::Value(None), |&(_, value)| value);
            let lost = each_layer!(&mut entry.layer, store => {
                restore_in(store, rule, target, value)
            });
            carry.unpropagated += u64::from(lost);
        }
        carry
    }
}
