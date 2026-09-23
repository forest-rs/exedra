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

/// True when `weight` can select or contribute to a capture.
fn positive(weight: f32) -> bool {
    weight.is_finite() && weight > 0.0
}

/// Captures one layer's value from weighted `sources` under `rule`.
///
/// Only sources with a finite positive weight take part; when none has one,
/// every source does, in the given order. [`Propagation::Copy`] takes the
/// heaviest such source (the first among equals);
/// [`Propagation::Interpolate`] averages those that hold a value (a dense
/// layer's default included, as long as some participating source carries
/// information), with their weights renormalized; [`Propagation::Unspecified`] keeps the value `Copy`
/// would, or else the first present one, so restoring can count it as lost;
/// [`Propagation::Clear`] captures nothing.
fn capture_in<S: Store>(store: &S, rule: Propagation, sources: &[(Id, f32)]) -> Option<Sample> {
    let any_positive = sources.iter().any(|&(_, weight)| positive(weight));
    let takes_part = |weight: f32| !any_positive || positive(weight);
    // The heaviest participating source, first among equals.
    let heaviest = sources
        .iter()
        .filter(|&&(_, weight)| takes_part(weight))
        .fold(None::<(Id, f32)>, |best, &(id, weight)| match best {
            Some((_, best_weight))
                if weight.partial_cmp(&best_weight) != Some(core::cmp::Ordering::Greater) =>
            {
                best
            }
            _ => Some((id, weight)),
        })
        .map(|(id, _)| id);
    match rule {
        Propagation::Clear => None,
        Propagation::Unspecified => heaviest
            .filter(|id| store.present(*id))
            .or_else(|| {
                sources
                    .iter()
                    .filter(|&&(_, weight)| takes_part(weight))
                    .map(|&(id, _)| id)
                    .find(|id| store.present(*id))
            })
            .and_then(|id| store.value(id))
            .map(|v| v.sample()),
        Propagation::Copy => heaviest
            .filter(|id| store.present(*id))
            .and_then(|id| store.value(id))
            .map(|v| v.sample()),
        Propagation::Interpolate => {
            let mut first = None;
            let mut acc: Option<(S::V, f32)> = None;
            for &(id, weight) in sources {
                if !takes_part(weight) {
                    continue;
                }
                let Some(value) = store.value(id) else {
                    continue;
                };
                if first.is_none() {
                    first = Some(value.clone());
                }
                if !positive(weight) {
                    continue;
                }
                acc = Some(match acc {
                    None => (value, weight),
                    Some((mean, total)) => {
                        let sum = total + weight;
                        let blended =
                            S::V::blend(&mean, total / sum, &value, weight / sum).unwrap_or(mean);
                        (blended, sum)
                    }
                });
            }
            let value = acc.map(|(mean, _)| mean).or(first)?;
            // A dense layer's default carries no information.
            let any_present = sources
                .iter()
                .any(|&(id, weight)| takes_part(weight) && store.present(id));
            any_present.then(|| value.sample())
        }
    }
}

/// An empty layer with `layer`'s storage and value type, sized `len`.
fn empty_like(layer: &Layer, len: usize) -> Layer {
    match layer {
        Layer::DenseF32(l) => Layer::DenseF32(DenseLayer::with_len(len, l.default)),
        Layer::DenseVec2(l) => Layer::DenseVec2(DenseLayer::with_len(len, l.default)),
        Layer::DenseVec3(l) => Layer::DenseVec3(DenseLayer::with_len(len, l.default)),
        Layer::DenseVec4(l) => Layer::DenseVec4(DenseLayer::with_len(len, l.default)),
        Layer::DenseU32(l) => Layer::DenseU32(DenseLayer::with_len(len, l.default)),
        Layer::DenseBool(l) => Layer::DenseBool(DenseLayer::with_len(len, l.default)),
        Layer::SparseF32(_) => Layer::SparseF32(SparseLayer::new()),
        Layer::SparseVec2(_) => Layer::SparseVec2(SparseLayer::new()),
        Layer::SparseVec3(_) => Layer::SparseVec3(SparseLayer::new()),
        Layer::SparseVec4(_) => Layer::SparseVec4(SparseLayer::new()),
        Layer::SparseU32(_) => Layer::SparseU32(SparseLayer::new()),
        Layer::SparseBool(_) => Layer::SparseBool(SparseLayer::new()),
    }
}

/// True when two layers share storage (dense or sparse), value type and, for
/// dense layers, default.
fn same_shape(a: &Layer, b: &Layer) -> bool {
    match (a, b) {
        (Layer::DenseF32(a), Layer::DenseF32(b)) => a.default.to_bits() == b.default.to_bits(),
        (Layer::DenseVec2(a), Layer::DenseVec2(b)) => {
            a.default.map(f32::to_bits) == b.default.map(f32::to_bits)
        }
        (Layer::DenseVec3(a), Layer::DenseVec3(b)) => {
            a.default.map(f32::to_bits) == b.default.map(f32::to_bits)
        }
        (Layer::DenseVec4(a), Layer::DenseVec4(b)) => {
            a.default.map(f32::to_bits) == b.default.map(f32::to_bits)
        }
        (Layer::DenseU32(a), Layer::DenseU32(b)) => a.default == b.default,
        (Layer::DenseBool(a), Layer::DenseBool(b)) => a.default == b.default,
        (Layer::SparseF32(_), Layer::SparseF32(_))
        | (Layer::SparseVec2(_), Layer::SparseVec2(_))
        | (Layer::SparseVec3(_), Layer::SparseVec3(_))
        | (Layer::SparseVec4(_), Layer::SparseVec4(_))
        | (Layer::SparseU32(_), Layer::SparseU32(_))
        | (Layer::SparseBool(_), Layer::SparseBool(_)) => true,
        _ => false,
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

    /// Captures every caller-defined value of `domain` from weighted
    /// `sources`, combined per layer rule (see [`capture_in`]).
    pub(crate) fn capture_weighted(&self, domain: Domain, sources: &[(Id, f32)]) -> CallerValues {
        let values = self
            .dense
            .iter()
            .chain(self.sparse.iter())
            .filter(|entry| is_caller(entry, domain))
            .map(|entry| {
                let rule = entry.propagation;
                let sample = each_layer!(&entry.layer, store => capture_in(store, rule, sources));
                (entry.name, Captured::Value(sample))
            })
            .collect();
        CallerValues { values }
    }

    /// Registers every caller-defined layer of `from` missing here, empty,
    /// with the same storage, value type, default and rule. Layers already
    /// registered here keep their own rule and values.
    ///
    /// Returns how many layers were registered. Nothing is registered when a
    /// layer exists here with another storage, value type, default or rule.
    pub(crate) fn adopt_caller_layers(&mut self, from: &Self) -> Result<usize, super::AttrError> {
        let callers = || {
            from.dense
                .iter()
                .chain(from.sparse.iter())
                .filter(|entry| !crate::attr::is_reserved(entry.domain, entry.name))
        };
        for entry in callers() {
            if let Some(existing) = self.layer(entry.domain, entry.name) {
                if !same_shape(existing, &entry.layer) {
                    return Err(super::AttrError::TypeMismatch);
                }
                if self.propagation(entry.domain, entry.name) != Some(entry.propagation) {
                    return Err(super::AttrError::RuleMismatch);
                }
            }
        }
        let mut adopted = 0;
        for entry in callers() {
            if self.layer(entry.domain, entry.name).is_some() {
                continue;
            }
            let len = self.domain_capacity(entry.domain);
            let adopted_entry = Entry {
                domain: entry.domain,
                name: entry.name,
                layer: empty_like(&entry.layer, len),
                propagation: entry.propagation,
            };
            if matches!(
                entry.layer,
                Layer::DenseF32(_)
                    | Layer::DenseVec2(_)
                    | Layer::DenseVec3(_)
                    | Layer::DenseVec4(_)
                    | Layer::DenseU32(_)
                    | Layer::DenseBool(_)
            ) {
                self.dense.push(adopted_entry);
            } else {
                self.sparse.push(adopted_entry);
            }
            adopted += 1;
        }
        Ok(adopted)
    }

    /// Writes captured values onto `target` as they are, whatever each
    /// layer's rule: for relabeling rebuilds that change no element's meaning.
    /// Layers the capture does not cover are left unchanged.
    pub(crate) fn restore_verbatim(&mut self, domain: Domain, target: Id, captured: &CallerValues) {
        for entry in self.caller_entries_mut(domain) {
            let Some(value) = captured
                .values
                .iter()
                .find(|(name, _)| *name == entry.name)
                .map(|&(_, value)| value)
            else {
                continue;
            };
            let sample = match value {
                Captured::Value(sample) => sample,
                Captured::Conflict => None,
            };
            each_layer!(&mut entry.layer, store => {
                store.put(target, sample.and_then(Value::from_sample));
            });
        }
    }

    /// Restores captured values onto `target` according to each layer's rule.
    /// Layers the capture does not cover (the source had no layer of that
    /// name) are left unchanged, so restores from several sources compose.
    pub(crate) fn restore_caller(
        &mut self,
        domain: Domain,
        target: Id,
        captured: &CallerValues,
    ) -> Carry {
        let mut carry = Carry::default();
        for entry in self.caller_entries_mut(domain) {
            let rule = entry.propagation;
            let Some(value) = captured
                .values
                .iter()
                .find(|(name, _)| *name == entry.name)
                .map(|&(_, value)| value)
            else {
                continue;
            };
            let lost = each_layer!(&mut entry.layer, store => {
                restore_in(store, rule, target, value)
            });
            carry.unpropagated += u64::from(lost);
        }
        carry
    }
}
