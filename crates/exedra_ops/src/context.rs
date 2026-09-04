// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator execution context, reusable scratch, and timing clock.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cell::RefCell;

use exedra_mesh::{CornerId, FaceId, HalfEdgeId, Id, NumericPolicy, VertexId};
#[cfg(all(not(target_arch = "wasm32"), feature = "std"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::policy::PolicySet;
#[cfg(any(target_arch = "wasm32", feature = "std"))]
use crate::timing::duration_nanos_u64;
use crate::{CacheDirtySet, DiagnosticsSink, Timings};

/// Reusable typed scratch buffers for hot loops.
///
/// `clear()` resets logical lengths while retaining vector allocations.
#[derive(Clone, Debug, Default)]
pub struct Scratch {
    /// Generic integer buffer.
    pub u32s: Vec<u32>,
    /// Generic integer buffer.
    pub u64s: Vec<u64>,
    /// Generic float buffer.
    pub f32s: Vec<f32>,
    /// 2D vector buffer.
    pub vec2: Vec<[f32; 2]>,
    /// 3D vector buffer.
    pub vec3: Vec<[f32; 3]>,
    /// Vertex ID list.
    pub vertices: Vec<VertexId>,
    /// Half-edge ID list.
    pub half_edges: Vec<HalfEdgeId>,
    /// Face ID list.
    pub faces: Vec<FaceId>,
    /// Corner ID list.
    pub corners: Vec<CornerId>,
    /// Reusable map for integer-indexed staging.
    pub map_u32_u32: BTreeMap<u32, u32>,
    /// Reusable map for ID-indexed staging.
    pub map_id_u32: BTreeMap<Id, u32>,
}

impl Scratch {
    /// Clears all scratch state while retaining vector capacities.
    pub fn clear(&mut self) {
        self.u32s.clear();
        self.u64s.clear();
        self.f32s.clear();
        self.vec2.clear();
        self.vec3.clear();
        self.vertices.clear();
        self.half_edges.clear();
        self.faces.clear();
        self.corners.clear();
        self.map_u32_u32.clear();
        self.map_id_u32.clear();
    }
}

/// Best-effort timing recorder.
///
/// Timing source behavior:
/// - `wasm32`: `web_time::Instant`
/// - non-`wasm32` with `std` feature: `std::time::Instant`
/// - other `no_std`: deterministic zero timings
///
/// Threading:
/// - intentionally single-threaded via interior mutability (`RefCell`)
/// - `Clock` is not `Sync`, so [`OpContext`] is not shareable across threads
#[derive(Debug, Default)]
pub struct Clock {
    timings: RefCell<Timings>,
}

impl Clock {
    /// Creates a clock with timing bucket capacity.
    #[must_use]
    pub fn new(max_buckets: usize) -> Self {
        Self {
            timings: RefCell::new(Timings::new(max_buckets)),
        }
    }

    /// Opens a timing bucket guard.
    pub fn bucket(&self, name: &'static str) -> ClockBucket<'_> {
        ClockBucket {
            clock: self,
            name,
            #[cfg(any(target_arch = "wasm32", feature = "std"))]
            start: Instant::now(),
        }
    }

    /// Adds explicit nanos into a timing bucket.
    pub fn add_nanos(&self, name: &'static str, nanos: u64) {
        self.timings.borrow_mut().add(name, nanos);
    }

    /// Returns a snapshot of timings.
    #[must_use]
    pub fn timings(&self) -> Timings {
        self.timings.borrow().clone()
    }
}

/// RAII bucket guard.
#[derive(Debug)]
pub struct ClockBucket<'a> {
    clock: &'a Clock,
    name: &'static str,
    #[cfg(any(target_arch = "wasm32", feature = "std"))]
    start: Instant,
}

impl Drop for ClockBucket<'_> {
    fn drop(&mut self) {
        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        let nanos = duration_nanos_u64(self.start.elapsed());
        #[cfg(not(any(target_arch = "wasm32", feature = "std")))]
        let nanos = 0_u64;
        self.clock.add_nanos(self.name, nanos);
    }
}

/// Operator execution context.
///
/// Scratch discipline:
/// - `scratch` buffers are reused across operator calls
/// - callers should invoke [`Scratch::clear`] between operators
/// - data inside `scratch` should be treated as ephemeral and not retained
///
/// Owned by [`OperatorRunner`](crate::OperatorRunner) as reusable execution
/// state.
#[derive(Debug, Default)]
pub struct OpContext {
    /// Higher-level policy.
    pub policy: PolicySet,
    /// Numeric policy from Exedra Mesh.
    pub numeric: NumericPolicy,
    /// Reusable scratch buffers.
    pub scratch: Scratch,
    /// Bounded diagnostics sink.
    pub diagnostics: DiagnosticsSink,
    /// Bucketed clock.
    pub clock: Clock,
    /// Runtime cache dirty channels.
    pub cache_dirty: CacheDirtySet,
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra_mesh::Id;

    use super::{Clock, OpContext, Scratch};

    #[test]
    fn scratch_clear_resets_lengths_and_keeps_vec_capacity() {
        let mut scratch = Scratch::default();
        scratch.u32s.extend_from_slice(&[1, 2, 3, 4]);
        scratch.vec3.extend_from_slice(&[[1.0, 2.0, 3.0]; 2]);
        let cap_u32 = scratch.u32s.capacity();
        let cap_vec3 = scratch.vec3.capacity();
        scratch
            .map_id_u32
            .insert(Id::new(0, NonZeroU32::MIN), 1_u32);

        scratch.clear();

        assert!(scratch.u32s.is_empty());
        assert!(scratch.vec3.is_empty());
        assert!(scratch.map_id_u32.is_empty());
        assert_eq!(scratch.u32s.capacity(), cap_u32);
        assert_eq!(scratch.vec3.capacity(), cap_vec3);
    }

    #[test]
    fn clock_bucket_guard_accumulates_named_buckets() {
        let clock = Clock::new(8);
        {
            let _a = clock.bucket("phase.a");
            let _nested = clock.bucket("phase.a");
        }
        {
            let _b = clock.bucket("phase.b");
        }

        let timings = clock.timings();
        let buckets = timings.iter().copied().collect::<Vec<_>>();
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].name, "phase.a");
        assert_eq!(buckets[1].name, "phase.b");
    }

    #[test]
    fn op_context_default_is_constructible() {
        let ctx = OpContext::default();
        assert!(ctx.scratch.u32s.is_empty());
        assert!(ctx.diagnostics.is_empty());
        assert!(ctx.clock.timings().iter().next().is_none());
        assert!(ctx.cache_dirty.is_empty());
    }
}
