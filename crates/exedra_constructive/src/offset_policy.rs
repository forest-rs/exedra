// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;
use core::ops::Range;
use kurbo::Point;

use crate::discretize::{DiscretizePolicy, discretize_loop_with_counts, loop_edge_counts};
use crate::profile::{Loop2, Profile2, ProfileError};

/// Absolute recipe-unit accuracy controls and whole-operation work limits.
///
/// Fitting tolerance is a target for Kurbo, not a certified error bound.
/// Checking tolerance bounds the chords used by result checks, not the
/// separation of the original continuous curves. Contacts finer than this
/// resolution can remain unresolved. No budget silently coarsens either target.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct OffsetPolicy {
    /// Requested cubic fitting tolerance, finite and positive, in recipe units.
    pub fit_tolerance: f64,
    /// Maximum positional enclosure for a numerically trimmed join, in recipe units.
    pub trim_tolerance: f64,
    /// Maximum interval boxes/contractions examined across cubic-adjacent joins.
    pub max_trim_steps: u64,
    /// Chord tolerance for source/result checks, finite and positive, in recipe units.
    pub check_tolerance: f64,
    /// Allowed shortfall from the offset distance in sampled clearance checks,
    /// in recipe units. Must be finite and at least `2 * check_tolerance + fit_tolerance + trim_tolerance`.
    /// For a nonzero offset, must also be smaller than its absolute distance.
    pub undercut_slack: f64,
    /// Maximum authored segments inspected across the outer loop and all holes.
    pub max_source_segments: u32,
    /// Maximum output segments, including fitted pieces and corner inserts.
    pub max_result_segments: u32,
    /// Maximum calls to Kurbo's cubic offset fitter. One call is one source cubic;
    /// the pinned fitter internally bounds recursion at depth eight. This counts
    /// calls, not internal iterations or elapsed CPU time.
    pub max_cubic_fits: u32,
    /// Total edges allocated across all check samplings, including repeated loops.
    pub max_check_edges: u32,
    /// Maximum reserved edge-pair visits for intersection, containment and
    /// clearance checks. Full upper bounds are charged before each check,
    /// even when bounding boxes or early exits avoid some actual comparisons.
    pub max_check_pairs: u64,
}

impl Default for OffsetPolicy {
    fn default() -> Self {
        Self {
            fit_tolerance: 0.001,
            trim_tolerance: 0.001,
            max_trim_steps: 65536,
            check_tolerance: 0.01,
            undercut_slack: 0.05,
            max_source_segments: 4096,
            max_result_segments: 16384,
            max_cubic_fits: 4096,
            max_check_edges: 65536,
            max_check_pairs: 16_000_000,
        }
    }
}

impl OffsetPolicy {
    /// Validates accuracy scalars. Zero work budgets are valid and refuse any
    /// operation needing that resource. Returns [`ProfileError::InvalidOffsetPolicy`]
    /// for invalid scalars or insufficient clearance slack.
    pub fn validate(&self) -> Result<(), ProfileError> {
        let minimum_slack = 2.0 * self.check_tolerance + self.fit_tolerance + self.trim_tolerance;
        if !self.fit_tolerance.is_finite()
            || self.fit_tolerance <= 0.0
            || !self.trim_tolerance.is_finite()
            || self.trim_tolerance <= 0.0
            || !self.check_tolerance.is_finite()
            || self.check_tolerance <= 0.0
            || !self.undercut_slack.is_finite()
            || !minimum_slack.is_finite()
            || self.undercut_slack < minimum_slack
        {
            return Err(ProfileError::InvalidOffsetPolicy);
        }
        Ok(())
    }
}

/// Resource whose whole-operation offset budget was exhausted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OffsetBudget {
    /// Authored source segments.
    SourceSegments,
    /// Emitted result segments.
    ResultSegments,
    /// Cubic fitter calls.
    CubicFits,
    /// Interval intersection boxes/contractions for numerical corner trimming.
    TrimSteps,
    /// Allocated check edges.
    CheckEdges,
    /// Reserved edge-pair visits.
    CheckPairs,
}

/// How one output run was produced, independently of its resulting segment kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OffsetMethod {
    /// Copied unchanged by a zero-distance operation, including any source cubics.
    Identity,
    /// Analytic line/arc offset, trim, or authored corner construction in f64.
    Analytic,
    /// Kurbo cubic fitting; requested tolerance is recorded in the policy.
    /// No continuous error certificate is claimed, even if the output is a line.
    Fitted,
    /// Line/arc offset with a numerically trimmed endpoint; see [`OffsetResult::trims`].
    Trimmed,
}

/// Source correspondence for a contiguous run of output segments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffsetRun {
    /// Hole index, or `None` for the outer loop.
    pub hole: Option<usize>,
    /// Source segment index; `None` identifies inserted corner geometry.
    pub source: Option<u32>,
    /// Half-open segment range within the output loop.
    pub result: Range<u32>,
    /// Analytic construction, fitting, or identity copying.
    pub method: OffsetMethod,
}

/// Work charged to a successful offset operation.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct OffsetWork {
    /// Source segments inspected.
    pub source_segments: u64,
    /// Result segments emitted.
    pub result_segments: u64,
    /// Calls to the cubic fitter.
    pub cubic_fits: u64,
    /// Interval intersection boxes/contractions for numerical trimming.
    pub trim_steps: u64,
    /// Edges allocated for checks, including repeated samplings.
    pub check_edges: u64,
    /// Reserved upper bound on check edge-pair visits.
    pub check_pairs: u64,
}

/// Isolated join between finite offset pieces beside a fitted cubic.
#[derive(Clone, Debug, PartialEq)]
pub struct OffsetTrim {
    /// Hole index, or `None` for the outer loop.
    pub hole: Option<usize>,
    /// Source segment before the join; the following source segment wraps cyclically.
    pub corner: u32,
    /// Local fitted-piece index on each side (zero for an analytic line/arc).
    pub pieces: [u32; 2],
    /// Retained parameters on those pieces, before trimming, in `[0, 1]`.
    pub parameters: [f64; 2],
    /// Parameter enclosures used to prove that retained start/end cuts are ordered.
    pub parameter_bounds: [[f64; 2]; 2],
    /// Shared emitted join point in recipe units.
    pub point: [f64; 2],
    /// Positional enclosure radius used to accept the fitted-curve intersection.
    /// This does not bound the upstream cubic fit's error.
    pub position_bound: f64,
    /// Largest endpoint/adjacent-control-point displacement used to share the join.
    pub endpoint_adjustment: f64,
}

/// Offset geometry and operation-local approximation evidence.
///
/// The profile hashes normally. Evidence describes this derivation, is not
/// encoded into the profile, and does not certify exact continuous topology.
#[derive(Clone, Debug)]
pub struct OffsetResult {
    /// Resulting profile, with ordinary winding and closure invariants.
    pub profile: Profile2,
    /// Accuracy and work policy applied to this operation.
    pub policy: OffsetPolicy,
    /// Actual charges, each no greater than its policy limit.
    pub work: OffsetWork,
    /// Output runs in loop/segment order, covering every output segment once.
    pub runs: Vec<OffsetRun>,
    /// Numerical intersections used by cubic-adjacent inside corners.
    pub trims: Vec<OffsetTrim>,
}

#[derive(Default)]
pub(super) struct OffsetContext {
    pub policy: Option<OffsetPolicy>,
    pub work: OffsetWork,
    pub runs: Vec<OffsetRun>,
    pub trims: Vec<OffsetTrim>,
}

impl OffsetContext {
    pub(super) fn charge(&mut self, budget: OffsetBudget, amount: u64) -> Result<(), ProfileError> {
        let policy = match (self.policy, budget) {
            (Some(policy), _) => policy,
            (None, OffsetBudget::TrimSteps) => OffsetPolicy::default(),
            (None, _) => return Ok(()),
        };
        let (used, maximum) = match budget {
            OffsetBudget::SourceSegments => (
                &mut self.work.source_segments,
                u64::from(policy.max_source_segments),
            ),
            OffsetBudget::ResultSegments => (
                &mut self.work.result_segments,
                u64::from(policy.max_result_segments),
            ),
            OffsetBudget::CubicFits => {
                (&mut self.work.cubic_fits, u64::from(policy.max_cubic_fits))
            }
            OffsetBudget::TrimSteps => (&mut self.work.trim_steps, policy.max_trim_steps),
            OffsetBudget::CheckEdges => (
                &mut self.work.check_edges,
                u64::from(policy.max_check_edges),
            ),
            OffsetBudget::CheckPairs => (&mut self.work.check_pairs, policy.max_check_pairs),
        };
        let next = used
            .checked_add(amount)
            .filter(|next| *next <= maximum)
            .ok_or(ProfileError::OffsetBudgetExceeded { budget, maximum })?;
        *used = next;
        Ok(())
    }

    pub(super) fn record(
        &mut self,
        hole: Option<usize>,
        source: Option<u32>,
        result: Range<u32>,
        method: OffsetMethod,
    ) {
        if self.policy.is_some() && !result.is_empty() {
            self.runs.push(OffsetRun {
                hole,
                source,
                result,
                method,
            });
        }
    }

    pub(super) fn sample(&mut self, source: &Loop2) -> Result<Vec<Point>, ProfileError> {
        let policy = self.policy.expect("explicit check policy");
        let remaining = u64::from(policy.max_check_edges) - self.work.check_edges;
        if remaining < source.segs().len() as u64 {
            return Err(ProfileError::OffsetBudgetExceeded {
                budget: OffsetBudget::CheckEdges,
                maximum: u64::from(policy.max_check_edges),
            });
        }
        let sampling = DiscretizePolicy {
            chord_tolerance: policy.check_tolerance,
            max_segment_edges: policy.max_check_edges,
            min_arc_edges: 1,
        };
        let counts =
            loop_edge_counts(source, &sampling).map_err(ProfileError::OffsetCheckSampling)?;
        self.charge(
            OffsetBudget::CheckEdges,
            counts.iter().map(|&n| u64::from(n)).sum(),
        )?;
        let sampled = discretize_loop_with_counts(source, &sampling, &counts)
            .map_err(ProfileError::OffsetCheckSampling)?;
        Ok(sampled
            .points
            .into_iter()
            .map(|p| Point::new(p[0], p[1]))
            .collect())
    }
}
