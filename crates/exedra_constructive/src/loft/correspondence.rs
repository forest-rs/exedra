// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact profile subdivision with explicit seams and perimeter landmarks.

use crate::profile::{LineArcError, Loop2, Profile2, ProfileError, Seg2};
use alloc::{boxed::Box, vec, vec::Vec};
use exedra_profile::{Loop, LoopRef, Profile, ProfilePolicy, SegmentRef};

mod metric;
use metric::Metric;

/// An authored point on a profile loop; parameter zero is the segment start.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoundaryPoint {
    /// Original segment index within the selected loop.
    pub segment: usize,
    /// Fraction in `[0,1]`, linear on lines and angular on circular arcs.
    pub parameter: f64,
}
impl BoundaryPoint {
    /// A point at a source segment's parameter fraction.
    #[must_use]
    pub const fn new(segment: usize, parameter: f64) -> Self {
        Self { segment, parameter }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Landmarks {
    loop_ref: LoopRef,
    points: Vec<BoundaryPoint>,
}

/// One borrowed line/arc profile and its explicit correspondence intent.
///
/// Without landmarks, each loop starts at its authored seam and corresponds by
/// normalized perimeter distance. With landmarks, the first point is its seam;
/// subsequent points follow traversal order. Landmark `i` matches landmark `i`
/// in every section. Distance is normalized independently between each pair.
#[derive(Clone, Debug, PartialEq)]
pub struct CorrespondenceProfile<'a> {
    /// Original profile, retained unchanged by preparation.
    pub profile: &'a Profile2,
    landmarks: Vec<Landmarks>,
}
impl<'a> CorrespondenceProfile<'a> {
    /// Uses the authored seams and normalized perimeter correspondence.
    #[must_use]
    pub fn new(profile: &'a Profile2) -> Self {
        Self {
            profile,
            landmarks: Vec::new(),
        }
    }
    /// Sets a loop's cyclic landmark list. Validation is deferred to preparation.
    /// Every section must supply the same number of landmarks for this loop.
    /// A repeated loop declaration is refused, not overwritten.
    #[must_use]
    pub fn with_landmarks(
        mut self,
        loop_ref: LoopRef,
        points: impl IntoIterator<Item = BoundaryPoint>,
    ) -> Self {
        self.landmarks.push(Landmarks {
            loop_ref,
            points: points.into_iter().collect(),
        });
        self
    }
    /// Authored landmarks for a loop; `None` means the default seam.
    #[must_use]
    pub fn landmarks(&self, loop_ref: LoopRef) -> Option<&[BoundaryPoint]> {
        self.landmarks
            .iter()
            .find(|m| m.loop_ref == loop_ref)
            .map(|m| m.points.as_slice())
    }
}

/// Geometric validation and finite output work limits for correspondence.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CorrespondencePolicy {
    /// Arrangement/separation and per-profile segment limits, for input and output.
    pub profile: ProfilePolicy,
    /// Largest sum of output segment counts across every section and loop.
    pub max_total_segments: u32,
}
impl CorrespondencePolicy {
    /// Uses the supplied profile policy and a total budget of one million segments.
    #[must_use]
    pub const fn new(profile: ProfilePolicy) -> Self {
        Self {
            profile,
            max_total_segments: 1_000_000,
        }
    }
}

/// Original source interval of one exact output profile segment.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ProfileSegmentSpan {
    /// Original loop and segment, before splitting or changing the seam.
    pub source: SegmentRef,
    /// Increasing original segment fractions, including authoritative endpoints.
    pub parameter: [f64; 2],
}

/// One coordinated profile with an original-source interval for every segment.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchedProfile {
    /// Exactly subdivided profile; tags remain those of the original pieces.
    pub profile: Profile2,
    /// Parallel segment origins: outer loop first, then holes in authored order.
    pub segments: Vec<Vec<ProfileSegmentSpan>>,
}

/// Work and numerical normalization evidence for profile preparation.
#[derive(Clone, Debug, PartialEq)]
pub struct CorrespondenceReport {
    /// Total original segment count across all sections and loops.
    pub input_segments: usize,
    /// Total output segment count, bounded by the caller policy.
    pub output_segments: usize,
    /// Number of loops per section.
    pub loops: usize,
    /// Largest normalized-distance adjustment when merging roundoff-close knots.
    /// Every contributing authored point is preserved at its exact source fraction.
    pub max_knot_adjustment: f64,
}

/// Coordinated geometry and traceable source intervals, ready for ordinary lofts.
#[derive(Clone, Debug, PartialEq)]
pub struct CorrespondenceResult {
    /// Output profiles in input section order.
    pub profiles: Vec<MatchedProfile>,
    /// Preparation work and normalization evidence.
    pub report: CorrespondenceReport,
}

/// An invalid explicit landmark declaration.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LandmarkError {
    /// The selected profile loop does not exist.
    MissingLoop,
    /// The same loop was declared twice.
    DuplicateLoop,
    /// A list is empty, a segment is absent, or its parameter is outside `[0,1]`.
    Point,
    /// Points repeat or do not follow cyclic traversal from the first landmark.
    Order,
    /// Sections have different landmark counts on this loop.
    Count {
        /// Count on section zero.
        expected: usize,
        /// Count on the offending section.
        actual: usize,
    },
}

/// A source-indexed refusal; preparation returns no partially coordinated result.
#[derive(Clone, Debug, PartialEq)]
pub enum CorrespondenceError {
    /// A separation or work limit is invalid.
    InvalidPolicy,
    /// At least two profiles are required.
    Sections,
    /// Input profile conversion or arrangement validation failed.
    Profile {
        /// Input section.
        section: usize,
        /// Original profile diagnosis.
        error: LineArcError,
    },
    /// Hole counts do not agree; holes correspond in authored order.
    LoopCount {
        /// Input section.
        section: usize,
        /// Expected loops, including the outer boundary.
        expected: usize,
        /// Actual loops.
        actual: usize,
    },
    /// Explicit landmark intent is invalid.
    Landmarks {
        /// Input section.
        section: usize,
        /// Selected loop.
        loop_ref: LoopRef,
        /// Landmark index, when local to one point.
        point: Option<usize>,
        /// Invalid intent.
        reason: LandmarkError,
    },
    /// A finite caller work limit would be exceeded.
    Budget {
        /// Budgeted unit.
        storage: &'static str,
        /// Required entries.
        required: usize,
        /// Caller limit.
        maximum: u32,
    },
    /// Finite coordinates/distances cannot preserve distinct source cuts.
    Numeric {
        /// Input section.
        section: usize,
        /// Selected loop.
        loop_ref: LoopRef,
        /// Original cuts implicated in a collapsed interval, when available.
        points: Option<[BoundaryPoint; 2]>,
    },
    /// Exact splitting could not be represented as a valid authored profile.
    Output {
        /// Input section.
        section: usize,
        /// Original authoring diagnosis.
        error: ProfileError,
    },
    /// The prepared line/arc arrangement fails the supplied profile policy.
    OutputValidation {
        /// Input section.
        section: usize,
        /// Prepared profile diagnosis; use the reported output policy.
        error: LineArcError,
        /// Original source intervals for a prepared boundary contact.
        sources: Option<Box<[ProfileSegmentSpan; 2]>>,
    },
}
impl core::fmt::Display for CorrespondenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "loft profile correspondence: {self:?}")
    }
}
impl core::error::Error for CorrespondenceError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Profile { error, .. } | Self::OutputValidation { error, .. } => Some(error),
            Self::Output { error, .. } => Some(error),
            _ => None,
        }
    }
}

fn loop_ref(index: usize) -> LoopRef {
    if index == 0 {
        LoopRef::Outer
    } else {
        LoopRef::Hole(index - 1)
    }
}
fn loop_index(loop_ref: LoopRef) -> usize {
    match loop_ref {
        LoopRef::Outer => 0,
        LoopRef::Hole(i) => i.saturating_add(1),
    }
}
fn ring(profile: &Profile, index: usize) -> &Loop {
    if index == 0 {
        profile.outer()
    } else {
        &profile.holes()[index - 1]
    }
}
fn authored_ring(profile: &Profile2, index: usize) -> &Loop2 {
    if index == 0 {
        profile.outer()
    } else {
        &profile.holes()[index - 1]
    }
}

#[derive(Copy, Clone)]
struct Event {
    progress: f64,
    section: usize,
    point: BoundaryPoint,
}

/// Splits line/arc profiles into equal segment structures, preserving their shapes.
///
/// Each loop uses authored traversal and hole order. Default seams correspond
/// by normalized perimeter distance; explicit landmarks normalize each interval
/// separately. Every original vertex and landmark is retained. No reversal,
/// geometric seam guessing, flattening, or packing is performed.
///
/// Roundoff-close normalized knots (within `64*EPSILON`) share one output knot,
/// while every contributing source point stays exact. Distinct points from the
/// same section cannot be merged. The report records the largest adjustment.
/// Result segment indices belong to the prepared profiles; `segments` maps them
/// back to original source intervals and tags. Preparation is explicit and can
/// feed mesh or native construction without changing the original profiles.
/// Align nearby corners explicitly with landmarks when perimeter matching would
/// otherwise introduce pieces smaller than the supplied separation policy.
///
/// ```
/// use exedra_constructive::{builders, loft::{
///     correspond_profiles, CorrespondencePolicy, CorrespondenceProfile,
/// }};
/// use exedra_profile::ProfilePolicy;
///
/// let circle = builders::circle(10.0)?;
/// let rectangle = builders::rect_centered(20.0, 12.0)?;
/// let matched = correspond_profiles(
///     &[CorrespondenceProfile::new(&circle), CorrespondenceProfile::new(&rectangle)],
///     &CorrespondencePolicy::new(ProfilePolicy::new(1e-7)),
/// )?;
/// assert_eq!(matched.profiles[0].profile.outer().segs().len(),
///            matched.profiles[1].profile.outer().segs().len());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
/// Returns source-indexed arrangement, landmark, numerical or budget refusals.
/// Cubic and policy-defined pieces retain their typed line/arc conversion error.
pub fn correspond_profiles(
    inputs: &[CorrespondenceProfile<'_>],
    policy: &CorrespondencePolicy,
) -> Result<CorrespondenceResult, CorrespondenceError> {
    if !policy.profile.min_separation.is_finite()
        || policy.profile.min_separation <= 0.0
        || policy.profile.max_segments < 2
        || policy.max_total_segments == 0
    {
        return Err(CorrespondenceError::InvalidPolicy);
    }
    if inputs.len() < 2 {
        return Err(CorrespondenceError::Sections);
    }
    let mut input_segments = 0_usize;
    for (section, input) in inputs.iter().enumerate() {
        let count = core::iter::once(input.profile.outer())
            .chain(input.profile.holes())
            .map(|r| r.segs().len())
            .sum::<usize>();
        if count > policy.profile.max_segments as usize {
            return Err(CorrespondenceError::Budget {
                storage: "input profile segments",
                required: count,
                maximum: policy.profile.max_segments,
            });
        }
        input_segments = input_segments
            .checked_add(count)
            .ok_or(CorrespondenceError::Budget {
                storage: "input segments",
                required: usize::MAX,
                maximum: policy.max_total_segments,
            })?;
        if input_segments > policy.max_total_segments as usize {
            return Err(CorrespondenceError::Budget {
                storage: "input segments",
                required: input_segments,
                maximum: policy.max_total_segments,
            });
        }
        for (i, m) in input.landmarks.iter().enumerate() {
            let error = |reason| CorrespondenceError::Landmarks {
                section,
                loop_ref: m.loop_ref,
                point: None,
                reason,
            };
            if loop_index(m.loop_ref) > input.profile.holes().len() {
                return Err(error(LandmarkError::MissingLoop));
            }
            if input.landmarks[..i]
                .iter()
                .any(|a| a.loop_ref == m.loop_ref)
            {
                return Err(error(LandmarkError::DuplicateLoop));
            }
            if m.points.len() > policy.profile.max_segments as usize {
                return Err(CorrespondenceError::Budget {
                    storage: "landmarks per loop",
                    required: m.points.len(),
                    maximum: policy.profile.max_segments,
                });
            }
        }
    }
    let native = inputs
        .iter()
        .enumerate()
        .map(|(section, p)| {
            p.profile
                .to_line_arc(&policy.profile)
                .map_err(|error| CorrespondenceError::Profile { section, error })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let loops = native[0].holes().len() + 1;
    for (section, p) in native.iter().enumerate().skip(1) {
        if p.holes().len() + 1 != loops {
            return Err(CorrespondenceError::LoopCount {
                section,
                expected: loops,
                actual: p.holes().len() + 1,
            });
        }
    }
    let mut report = CorrespondenceReport {
        input_segments,
        output_segments: 0,
        loops,
        max_knot_adjustment: 0.0,
    };
    let mut output_loops: Vec<Vec<Loop2>> = (0..inputs.len())
        .map(|_| Vec::with_capacity(loops))
        .collect();
    let mut origins: Vec<Vec<Vec<ProfileSegmentSpan>>> = (0..inputs.len())
        .map(|_| Vec::with_capacity(loops))
        .collect();
    let mut per_profile = 0_usize;
    for loop_id in 0..loops {
        let lr = loop_ref(loop_id);
        let mut metrics = inputs
            .iter()
            .zip(&native)
            .enumerate()
            .map(|(section, (p, n))| Metric::new(ring(n, loop_id), p.landmarks(lr), section, lr))
            .collect::<Result<Vec<_>, _>>()?;
        let count = metrics[0].anchors.len();
        for metric in &metrics[1..] {
            if metric.anchors.len() != count {
                return Err(CorrespondenceError::Landmarks {
                    section: metric.section,
                    loop_ref: lr,
                    point: None,
                    reason: LandmarkError::Count {
                        expected: count,
                        actual: metric.anchors.len(),
                    },
                });
            }
        }
        let mut pieces: Vec<Vec<Seg2>> = (0..inputs.len()).map(|_| Vec::new()).collect();
        let mut source_spans: Vec<Vec<ProfileSegmentSpan>> =
            (0..inputs.len()).map(|_| Vec::new()).collect();
        let mut events = Vec::with_capacity(metrics.iter().map(|m| m.arcs.len() + 2).sum());
        let mut overrides = vec![None; inputs.len()];
        let mut resolved = vec![BoundaryPoint::new(0, 0.0); inputs.len()];
        let mut previous = resolved.clone();
        for interval in 0..count {
            events.clear();
            for metric in &mut metrics {
                let (a, b) = (metric.anchor(interval), metric.anchor(interval + 1));
                events.push(Event {
                    progress: 0.0,
                    section: metric.section,
                    point: a.point,
                });
                events.push(Event {
                    progress: 1.0,
                    section: metric.section,
                    point: b.point,
                });
                while metric.remaining > 0 {
                    let d = metric.boundary_distance();
                    if d > b.distance {
                        break;
                    }
                    if d < a.distance {
                        return Err(metric.numeric());
                    }
                    events.push(Event {
                        progress: (d - a.distance) / (b.distance - a.distance),
                        section: metric.section,
                        point: BoundaryPoint::new(metric.boundary, 0.0),
                    });
                    metric.boundary = (metric.boundary + 1) % metric.arcs.len();
                    metric.remaining -= 1;
                }
            }
            events.sort_unstable_by(|a, b| {
                a.progress
                    .total_cmp(&b.progress)
                    .then(a.section.cmp(&b.section))
            });
            let mut cursor = 0;
            let mut have_previous = false;
            while cursor < events.len() {
                let progress = events[cursor].progress;
                let mut end = cursor + 1;
                while end < events.len() && events[end].progress - progress <= 64.0 * f64::EPSILON {
                    end += 1;
                }
                overrides.fill(None);
                for event in &events[cursor..end] {
                    if let Some(p) = overrides[event.section]
                        && p != event.point
                    {
                        return Err(metrics[event.section].interval_error(p, event.point));
                    }
                    overrides[event.section] = Some(event.point);
                    report.max_knot_adjustment =
                        report.max_knot_adjustment.max(event.progress - progress);
                }
                for (i, metric) in metrics.iter().enumerate() {
                    let (a, b) = (metric.anchor(interval), metric.anchor(interval + 1));
                    resolved[i] = if let Some(p) = overrides[i] {
                        p
                    } else {
                        metric.location(a.distance + progress * (b.distance - a.distance))?
                    };
                }
                if have_previous {
                    per_profile =
                        per_profile
                            .checked_add(1)
                            .ok_or(CorrespondenceError::Budget {
                                storage: "profile segments",
                                required: usize::MAX,
                                maximum: policy.profile.max_segments,
                            })?;
                    if per_profile > policy.profile.max_segments as usize {
                        return Err(CorrespondenceError::Budget {
                            storage: "profile segments",
                            required: per_profile,
                            maximum: policy.profile.max_segments,
                        });
                    }
                    let total = per_profile.saturating_mul(inputs.len());
                    if total > policy.max_total_segments as usize {
                        return Err(CorrespondenceError::Budget {
                            storage: "output segments",
                            required: total,
                            maximum: policy.max_total_segments,
                        });
                    }
                    report.output_segments = total;
                    for (i, metric) in metrics.iter().enumerate() {
                        let (piece, span) = metric.piece(
                            authored_ring(inputs[i].profile, loop_id),
                            previous[i],
                            resolved[i],
                        )?;
                        pieces[i].push(piece);
                        source_spans[i].push(span);
                    }
                }
                previous.clone_from_slice(&resolved);
                have_previous = true;
                cursor = end;
            }
        }
        for (i, (pieces, spans)) in pieces.into_iter().zip(source_spans).enumerate() {
            output_loops[i].push(
                Loop2::new(pieces)
                    .map_err(|error| CorrespondenceError::Output { section: i, error })?,
            );
            origins[i].push(spans);
        }
    }
    let profiles = output_loops
        .into_iter()
        .zip(origins)
        .enumerate()
        .map(|(section, (mut loops, segments))| {
            let outer = loops.remove(0);
            let profile = Profile2::new(outer, loops)
                .map_err(|error| CorrespondenceError::Output { section, error })?;
            profile.to_line_arc(&policy.profile).map_err(|error| {
                let sources =
                    if let LineArcError::Profile(exedra_profile::ProfileError::Contact {
                        first,
                        second,
                    }) = error
                    {
                        Some(Box::new([
                            segments[loop_index(first.loop_ref)][first.segment],
                            segments[loop_index(second.loop_ref)][second.segment],
                        ]))
                    } else {
                        None
                    };
                CorrespondenceError::OutputValidation {
                    section,
                    error,
                    sources,
                }
            })?;
            Ok(MatchedProfile { profile, segments })
        })
        .collect::<Result<_, _>>()?;
    Ok(CorrespondenceResult { profiles, report })
}

#[cfg(test)]
mod tests;
