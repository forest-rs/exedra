// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fingerprint-keyed evaluation cache for incremental regeneration.
//!
//! [`EvalCache`] is a caller-owned store of tessellated bodies keyed by
//! `(node fingerprint, world placement bits, policy fingerprint)`. The
//! recipe's Merkle fingerprints make invalidation exact by construction: a
//! parameter edit changes the fingerprints of exactly the nodes on the
//! edited path, so those keys simply never hit again — there is no
//! separate dirty-propagation step to get wrong, and no way to reuse a
//! stale entry. [`evaluate_with_cache`](crate::evaluate::evaluate_with_cache)
//! consults the cache at every body-producing node and is bit-identical to
//! the pure [`evaluate`](crate::evaluate::evaluate) by contract (tested).
//!
//! The world placement participates in the key because tessellation bakes
//! the composed world placement into f64 vertex construction *before* the
//! single f32 narrowing; caching local-space bodies and re-transforming on
//! hit would round twice and break bit-identity.
//!
//! Eviction is deterministic: entries carry a `(generation, sequence)`
//! stamp (generation advances once per evaluation, sequence per touch) and
//! the least recently touched entry is evicted first when the explicit
//! entry capacity is exceeded. No hash-order iteration is ever observable.
//!
//! Everything is measured (introspection tenet): hits, misses,
//! insertions, evictions, live entries, and approximate retained bytes.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;

use hashbrown::HashMap;

use crate::tessellate::{EvalPolicy, TessellatedBody};

/// Identity of everything in an [`EvalPolicy`] that can change tessellation
/// output, folded with [`crate::EVAL_SCHEMA_VERSION`] so schema bumps
/// invalidate caches explicitly. Must be extended whenever [`EvalPolicy`]
/// grows a field.
#[must_use]
pub fn policy_fingerprint(policy: &EvalPolicy) -> u64 {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(&crate::EVAL_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&policy.discretize.chord_tolerance.to_bits().to_le_bytes());
    bytes.extend_from_slice(&policy.discretize.max_segment_edges.to_le_bytes());
    bytes.extend_from_slice(&policy.discretize.min_arc_edges.to_le_bytes());
    bytes.extend_from_slice(&policy.sharp_sin_threshold.to_bits().to_le_bytes());
    match policy.planar_face_refinement {
        None => bytes.push(0),
        Some(refinement) => {
            bytes.push(1);
            bytes.extend_from_slice(&refinement.max_radius_edge_ratio.to_bits().to_le_bytes());
            bytes.extend_from_slice(&refinement.max_steiner_points.to_le_bytes());
            bytes.push(match refinement.boundary_splits {
                exedra_triangulate::BoundarySplits::Allowed => 0,
                exedra_triangulate::BoundarySplits::Forbidden => 1,
            });
        }
    }
    let mut hash: u128 = 0x6C62_272E_07BB_0142_62B8_2175_6295_C58D;
    for b in bytes {
        hash ^= u128::from(b);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013B);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "xor-folding a 128-bit hash to 64 bits is the intended narrowing"
    )]
    {
        (hash ^ (hash >> 64)) as u64
    }
}

/// One cache key: node content, world placement, policy.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CacheKey {
    /// The node's Merkle content fingerprint.
    pub node: u128,
    /// Exact bits of the incoming world placement (row-major).
    pub world: [u64; 12],
    /// [`policy_fingerprint`] of the evaluation policy.
    pub policy: u64,
}

/// Introspection counters accumulated across a cache's lifetime.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct EvalCacheCounters {
    /// Lookups that returned a cached body.
    pub hits: u64,
    /// Lookups that found nothing.
    pub misses: u64,
    /// Bodies inserted after a miss.
    pub insertions: u64,
    /// Entries evicted by the capacity policy.
    pub evictions: u64,
}

struct Slot {
    body: Rc<TessellatedBody>,
    approx_bytes: u64,
    stamp: (u64, u64),
}

/// A caller-owned, fingerprint-keyed store of tessellated bodies.
///
/// See the [module docs](self) for the key design and eviction contract.
pub struct EvalCache {
    entries: HashMap<CacheKey, Slot>,
    /// Least-recently-touched order: `(generation, sequence) -> key`.
    order: BTreeMap<(u64, u64), CacheKey>,
    generation: u64,
    sequence: u64,
    capacity: usize,
    bytes_retained: u64,
    counters: EvalCacheCounters,
}

impl EvalCache {
    /// Default entry capacity used by [`EvalCache::new`].
    pub const DEFAULT_CAPACITY: usize = 1024;

    /// Creates a cache with [`EvalCache::DEFAULT_CAPACITY`] entries.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Creates a cache holding at most `capacity` entries (minimum 1).
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            generation: 0,
            sequence: 0,
            capacity: capacity.max(1),
            bytes_retained: 0,
            counters: EvalCacheCounters::default(),
        }
    }

    /// Advances the recency generation; called once per evaluation.
    pub(crate) fn begin_generation(&mut self) {
        self.generation += 1;
        self.sequence = 0;
    }

    /// Looks up a body, refreshing its recency stamp on hit.
    pub(crate) fn get(&mut self, key: &CacheKey) -> Option<Rc<TessellatedBody>> {
        if let Some(slot) = self.entries.get_mut(key) {
            self.counters.hits += 1;
            self.order.remove(&slot.stamp);
            self.sequence += 1;
            slot.stamp = (self.generation, self.sequence);
            self.order.insert(slot.stamp, *key);
            Some(Rc::clone(&slot.body))
        } else {
            self.counters.misses += 1;
            None
        }
    }

    /// Inserts a body after a miss, evicting least-recently-touched
    /// entries beyond capacity.
    pub(crate) fn insert(&mut self, key: CacheKey, body: Rc<TessellatedBody>) {
        let approx_bytes = approx_body_bytes(&body);
        self.sequence += 1;
        let stamp = (self.generation, self.sequence);
        if let Some(previous) = self.entries.insert(
            key,
            Slot {
                body,
                approx_bytes,
                stamp,
            },
        ) {
            self.order.remove(&previous.stamp);
            self.bytes_retained = self.bytes_retained.saturating_sub(previous.approx_bytes);
        }
        self.order.insert(stamp, key);
        self.bytes_retained += approx_bytes;
        self.counters.insertions += 1;
        while self.entries.len() > self.capacity {
            let Some((&oldest, &victim)) = self.order.iter().next() else {
                break;
            };
            self.order.remove(&oldest);
            if let Some(slot) = self.entries.remove(&victim) {
                self.bytes_retained = self.bytes_retained.saturating_sub(slot.approx_bytes);
                self.counters.evictions += 1;
            }
        }
    }

    /// Number of live entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the cache holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Approximate bytes retained by cached bodies (topology and source
    /// maps estimated, attribute layers excluded — a documented lower
    /// bound for budgeting, not an allocator measurement).
    #[must_use]
    pub fn bytes_retained(&self) -> u64 {
        self.bytes_retained
    }

    /// Lifetime counters.
    #[must_use]
    pub fn counters(&self) -> EvalCacheCounters {
        self.counters
    }

    /// Drops every entry (counters are preserved).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.bytes_retained = 0;
    }
}

impl Default for EvalCache {
    fn default() -> Self {
        Self::new()
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the entry map renders as its length; slots are internal"
)]
impl core::fmt::Debug for EvalCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EvalCache")
            .field("entries", &self.entries.len())
            .field("capacity", &self.capacity)
            .field("bytes_retained", &self.bytes_retained)
            .field("counters", &self.counters)
            .finish()
    }
}

/// Rough retained-size estimate: positions plus half-edge topology plus
/// the source map's own approximation.
fn approx_body_bytes(body: &TessellatedBody) -> u64 {
    let vertices = body.mesh.vertices().count() as u64;
    let faces = body.mesh.faces().count() as u64;
    let corners: u64 = body
        .mesh
        .faces()
        .map(|face| body.mesh.face_loop(face).count() as u64)
        .sum();
    vertices * 16 + faces * 8 + corners * 16 + body.source_map.stats().approx_bytes as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discretize::DiscretizePolicy;

    #[test]
    fn policy_fingerprint_separates_policies_and_folds_schema() {
        let a = policy_fingerprint(&EvalPolicy::default());
        let b = policy_fingerprint(&EvalPolicy {
            discretize: DiscretizePolicy {
                chord_tolerance: 0.5,
                ..DiscretizePolicy::default()
            },
            ..EvalPolicy::default()
        });
        assert_ne!(a, b, "chord tolerance must change the fingerprint");
        assert_eq!(a, policy_fingerprint(&EvalPolicy::default()), "stable");
        let c = policy_fingerprint(&EvalPolicy {
            planar_face_refinement: Some(exedra_triangulate::RefineParams::default()),
            ..EvalPolicy::default()
        });
        assert_ne!(a, c, "planar-face refinement must change the fingerprint");
    }
}
