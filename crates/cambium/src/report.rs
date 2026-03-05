// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator reports: deterministic counters plus best-effort timings.

use alloc::vec::Vec;

use crate::artifact::Artifacts;

/// Per-domain element counts.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ElementCounts {
    /// Vertex count.
    pub vertices: u64,
    /// Half-edge count.
    pub half_edges: u64,
    /// Face count.
    pub faces: u64,
    /// Corner count.
    pub corners: u64,
}

/// Fixed deterministic counter set for common operator metrics.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct SmallCounters {
    /// Faces visited/processed.
    pub faces_processed: u64,
    /// Corner-domain writes performed by the operator.
    pub corners_written: u64,
    /// Edge-domain attribute writes (canonical undirected edges).
    pub edges_written: u64,
    /// Corners skipped because destination already had value.
    pub corners_skipped_existing: u64,
    /// Selection canonicalization passes performed.
    pub selections_canonicalized: u64,
    /// Vertices deleted in this run (commit-path only).
    pub deleted_vertices: u64,
}

/// Deterministic stats payload for one operator run.
///
/// You typically read this from [`OpReport::stats`] after running an operator
/// through [`crate::OperatorRunner`].
///
/// # Example
/// ```rust
/// use cambium::Stats;
///
/// let mut stats = Stats::default();
/// stats.counters.faces_processed = 3;
/// stats.counters.corners_written = 12;
///
/// assert_eq!(stats.counters.faces_processed, 3);
/// assert_eq!(stats.counters.corners_written, 12);
/// ```
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct Stats {
    /// Elements touched (read and/or written).
    pub elements_touched: ElementCounts,
    /// Elements created.
    pub elements_created: ElementCounts,
    /// Elements deleted.
    pub elements_deleted: ElementCounts,
    /// Common fixed counters.
    pub counters: SmallCounters,
}

/// Named timing bucket.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TimeBucket {
    /// Stable bucket name.
    pub name: &'static str,
    /// Aggregated nanoseconds.
    pub nanos: u64,
}

/// Best-effort timing collection with deterministic bucket retention.
///
/// Bucket ordering is insertion order. Overflow keeps earlier buckets and
/// deterministically drops new distinct bucket names.
///
/// You usually read this from [`OpReport::timings`], populated by
/// [`crate::OperatorRunner`] during `apply_in_place`/`preview_on_clone`.
///
/// # Example
/// ```rust
/// use cambium::Timings;
///
/// let mut timings = Timings::new(2);
/// timings.add("select", 100);
/// timings.add("select", 50);
/// timings.add("attrs", 25);
///
/// let buckets = timings.iter().collect::<Vec<_>>();
/// assert_eq!(buckets.len(), 2);
/// assert_eq!(buckets[0].name, "select");
/// assert_eq!(buckets[0].nanos, 150);
/// assert_eq!(buckets[1].name, "attrs");
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Timings {
    max_buckets: usize,
    buckets: Vec<TimeBucket>,
}

impl Timings {
    /// Default maximum number of timing buckets.
    pub const DEFAULT_MAX_BUCKETS: usize = 32;

    /// Creates a timing collection with an explicit bucket limit.
    #[must_use]
    pub fn new(max_buckets: usize) -> Self {
        Self {
            max_buckets,
            buckets: Vec::new(),
        }
    }

    /// Returns the maximum retained bucket count.
    #[must_use]
    pub const fn max_buckets(&self) -> usize {
        self.max_buckets
    }

    /// Returns retained bucket count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    /// Returns true when no buckets are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Returns retained buckets in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = &TimeBucket> {
        self.buckets.iter()
    }

    /// Adds timing to a bucket.
    ///
    /// Existing buckets are additive. New buckets are dropped when at capacity.
    pub fn add(&mut self, name: &'static str, nanos: u64) {
        if let Some(existing) = self.buckets.iter_mut().find(|bucket| bucket.name == name) {
            existing.nanos = existing.nanos.saturating_add(nanos);
            return;
        }
        if self.buckets.len() >= self.max_buckets {
            return;
        }
        self.buckets.push(TimeBucket { name, nanos });
    }
}

impl Default for Timings {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_BUCKETS)
    }
}

/// Operator execution report payload.
///
/// Returned by all [`EditOperator`](crate::EditOperator) implementations and
/// included in [`OpResult`](crate::OpResult) /
/// [`PreviewResult`](crate::PreviewResult).
///
/// In typical usage, obtain this as `result.report` from
/// [`crate::OperatorRunner::apply_in_place`] or `preview.report` from
/// [`crate::OperatorRunner::preview_on_clone`].
#[derive(Clone, Debug)]
pub struct OpReport {
    /// Stable operator name.
    pub name: &'static str,
    /// Deterministic counters.
    pub stats: Stats,
    /// Best-effort timing buckets.
    pub timings: Timings,
    /// Bounded debug artifacts.
    pub artifacts: Artifacts,
}

impl OpReport {
    /// Creates a new report with caller-supplied bounded artifacts.
    #[must_use]
    pub fn new(name: &'static str, artifacts: Artifacts) -> Self {
        Self {
            name,
            stats: Stats::default(),
            timings: Timings::default(),
            artifacts,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{OpReport, Timings};
    use crate::artifact::Artifacts;

    #[test]
    fn timings_accumulate_existing_bucket() {
        let mut timings = Timings::new(4);
        timings.add("build", 5);
        timings.add("build", 7);
        let collected = timings.iter().copied().collect::<Vec<_>>();
        assert_eq!(
            collected,
            vec![super::TimeBucket {
                name: "build",
                nanos: 12
            }]
        );
    }

    #[test]
    fn timings_drop_new_bucket_when_full() {
        let mut timings = Timings::new(2);
        timings.add("a", 1);
        timings.add("b", 2);
        timings.add("c", 3);
        assert_eq!(timings.len(), 2);
        let names = timings.iter().map(|bucket| bucket.name).collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn timings_existing_bucket_still_accumulates_when_full() {
        let mut timings = Timings::new(1);
        timings.add("a", 5);
        timings.add("b", 6);
        timings.add("a", 7);
        let bucket = timings.iter().next().expect("bucket should exist");
        assert_eq!(bucket.name, "a");
        assert_eq!(bucket.nanos, 12);
    }

    #[test]
    fn op_report_default_components_are_initialized() {
        let report = OpReport::new("op.test", Artifacts::new(2, 128));
        assert_eq!(report.name, "op.test");
        assert!(report.timings.is_empty());
        assert!(report.artifacts.is_empty());
    }
}
