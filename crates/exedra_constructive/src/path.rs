// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spatial sweep paths and bounded sampling, independent of mesh construction.
//!
//! Each span bounds centerline chord deviation and tangent variation. These
//! are separate from profile discretization, frame-transport integration error,
//! swept-surface error, and the later f32 mesh boundary. In particular a wide
//! asymmetric section can need a smaller tangent-angle limit even when its
//! centerline already meets the requested chord tolerance.

use alloc::vec::Vec;
use exedra_math::{add, cross, dot, scale, sub};

/// Authored endpoint connectivity of a controlled sweep path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PathClosure {
    /// A spatial rail with distinct endpoints and optional caps.
    #[default]
    Open,
    /// A cyclic path in the plane through its first station.
    ///
    /// Polyline stations omit the repeated endpoint; curved segments explicitly
    /// terminate at the exact starting point. The closing run and corner
    /// are explicit; caps must be [`crate::ir::CapMode::None`]. Only f64 rounding-scale
    /// deviation from the plane is accepted, without projecting the points.
    ClosedPlanar {
        /// Finite nonzero plane normal in path-local coordinates.
        /// Its magnitude and sign do not change section orientation, which
        /// remains controlled by `section_x` and traversal direction.
        normal: [f64; 3],
    },
}

/// Join behavior at authored curve segment boundaries, including a closed seam.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PathJoin {
    /// Require analytic tangent continuity; no corner is inferred or repaired.
    #[default]
    Smooth,
    /// Join sections on the tangent-bisector plane at an authored corner.
    Miter {
        /// Maximum section-plane stretch, `1 / cos(turn / 2)`, finite and >= 1.
        /// Evaluation refuses larger corners instead of beveling them.
        limit: f64,
    },
}

/// One authored segment, starting at the preceding endpoint (or path start).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PathSegment3 {
    /// Straight segment.
    Line {
        /// Endpoint in path-local coordinates.
        to: [f64; 3],
    },
    /// Circular arc obtained by rotating the start about an authored axis.
    /// The axis need not pass through the circle's center plane: its origin
    /// is any point on the rotation axis. No projection onto a world plane.
    Arc {
        /// Any point on the rotation axis, in path-local coordinates.
        axis_origin: [f64; 3],
        /// Nonzero finite axis direction; magnitude is irrelevant.
        axis: [f64; 3],
        /// Signed right-handed rotation in radians, `0 < abs(sweep) <= tau`.
        sweep: f64,
    },
    /// Spatial cubic Bezier segment.
    Cubic {
        /// First control point, in path-local coordinates.
        control1: [f64; 3],
        /// Second control point, in path-local coordinates.
        control2: [f64; 3],
        /// Endpoint in path-local coordinates.
        to: [f64; 3],
    },
}

/// Accuracy and finite work budgets for a curved sweep path.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PathDiscretizePolicy {
    /// Maximum centerline chord deviation in path-local recipe units.
    pub chord_tolerance: f64,
    /// Maximum angle between any two analytic tangents within one span,
    /// in radians, finite and in `(0, pi/2]`. Not a swept-surface error bound.
    pub max_tangent_angle: f64,
    /// Maximum accepted spans per source segment, in `1..=65535`.
    pub max_segment_edges: u32,
    /// Maximum spans in the whole path, in `1..=65535`. This also keeps
    /// every span addressable by [`crate::tessellate::Feature::SweepWall`].
    pub max_path_edges: u32,
}

impl Default for PathDiscretizePolicy {
    fn default() -> Self {
        Self {
            chord_tolerance: 0.01,
            max_tangent_angle: 0.05,
            max_segment_edges: 4096,
            max_path_edges: 65535,
        }
    }
}

impl PathDiscretizePolicy {
    /// Checks scalar accuracy and work bounds before any geometry traversal.
    pub fn validate(&self) -> Result<(), PathDiscretizeError> {
        if !self.chord_tolerance.is_finite()
            || self.chord_tolerance <= 0.0
            || !self.max_tangent_angle.is_finite()
            || self.max_tangent_angle <= 0.0
            || self.max_tangent_angle > core::f64::consts::FRAC_PI_2
            || !(1..=65535).contains(&self.max_segment_edges)
            || !(1..=65535).contains(&self.max_path_edges)
        {
            return Err(PathDiscretizeError::InvalidPolicy);
        }
        Ok(())
    }
}

/// One sampled interval, addressing the original authored path segment.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PathSpan {
    /// Source segment index, independent of sampling density.
    pub segment: u32,
    /// Inclusive start/end parameters in `[0, 1]`, strictly increasing.
    pub parameter: [f64; 2],
    /// Conservative centerline chord bound, including f64 rounding headroom.
    pub chord_bound: f64,
    /// Conservative tangent variation bound in radians.
    pub tangent_angle_bound: f64,
}

/// An authored tangent discontinuity, separate from smooth sampling stations.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PathCorner {
    /// Station index; zero denotes a sharp closed seam.
    pub station: u32,
    /// Source segment beginning at the corner, zero at a closed seam.
    pub segment: u32,
    /// Unsigned angle between incoming and outgoing analytic tangents, in radians.
    pub turn_angle: f64,
}

/// Original path-local sampling policy and per-span evidence.
///
/// Retained with tessellated bodies and their instances as source provenance.
/// Placement does not convert these bounds into world-space accuracy claims;
/// in particular, nonuniform scaling changes distances and tangent angles.
#[derive(Clone, Debug, PartialEq)]
pub struct PathSampling {
    /// Policy under which these spans were accepted.
    pub policy: PathDiscretizePolicy,
    /// Authored path connectivity; preserved as original construction evidence.
    pub closure: PathClosure,
    /// Authored corner behavior; sampled interiors remain smooth.
    pub joins: PathJoin,
    /// Authored discontinuities in ascending station order. Sampling creates none.
    pub corners: Vec<PathCorner>,
    /// One entry per sweep wall band, in traversal order.
    pub spans: Vec<PathSpan>,
}

impl PathSampling {
    pub(crate) fn approx_bytes(&self) -> usize {
        size_of::<Self>()
            + self.spans.len() * size_of::<PathSpan>()
            + self.corners.len() * size_of::<PathCorner>()
    }
}

/// A section station on an analytic path.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PathStation {
    /// Position in path-local coordinates.
    pub point: [f64; 3],
    /// Outgoing unit analytic tangent in traversal direction.
    pub tangent: [f64; 3],
    /// Incoming unit analytic tangent. Authored joins can differ by rounding;
    /// larger discontinuities require mitered corners.
    /// At an open start it equals the outgoing tangent.
    pub incoming_tangent: [f64; 3],
}

/// A sampled path; station count is span count plus one. A closed path
/// includes its repeated first station for span checks; mesh emission shares it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscretizedPath {
    /// Shared stations, including exact authored line/cubic endpoints.
    pub stations: Vec<PathStation>,
    /// Source correspondence and approximation evidence.
    pub sampling: PathSampling,
}

/// Typed inability to sample a path under the requested contract.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PathDiscretizeError {
    /// Invalid tolerance, tangent-angle bound, or edge budget.
    InvalidPolicy,
    /// Empty, nonfinite-start, or an open path whose endpoint equals its start.
    InvalidPath,
    /// Miter limit is nonfinite or less than one.
    InvalidJoin,
    /// Closed planar path has an unusable normal.
    InvalidPlane,
    /// A closed path does not terminate at its exact authored start.
    NotClosed {
        /// Authored start point.
        start: [f64; 3],
        /// Realized final endpoint; no snapping is applied.
        end: [f64; 3],
    },
    /// Authored controls, endpoints, or an arc axis leave the declared plane.
    NonPlanar {
        /// Offending source segment index.
        segment: usize,
    },
    /// Segment has invalid coordinates, axis, angle, or coincident line endpoints.
    InvalidSegment {
        /// Offending source segment index.
        segment: usize,
    },
    /// A tangent cannot be defined at an evaluated parameter.
    StationaryTangent {
        /// Offending source segment index.
        segment: usize,
        /// Parameter at which the derivative vanishes or is numerically unusable.
        parameter: f64,
    },
    /// Adjacent analytic tangent directions disagree by more than 1e-10 radians.
    /// Use an explicit miter join policy for sharp corners; no tangent is guessed.
    DiscontinuousTangent {
        /// Source segment beginning at the discontinuity.
        segment: usize,
    },
    /// Subdivision could not satisfy accuracy/tangent constraints within the segment budget.
    SegmentBudgetExceeded {
        /// Source segment still requiring subdivision.
        segment: usize,
        /// Maximum spans allowed for that segment.
        maximum: u32,
    },
    /// Total path sampling would exceed the global span budget.
    PathBudgetExceeded {
        /// Maximum spans allowed in the whole path.
        maximum: u32,
    },
    /// Coordinates or requested precision cannot be realized reliably in f64.
    /// Cubic subdivision also refuses depths beyond 32 instead of unbounded work.
    NumericLimit {
        /// Offending source segment index.
        segment: usize,
    },
}

impl core::fmt::Display for PathDiscretizeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPolicy => write!(f, "invalid sweep path sampling policy"),
            Self::InvalidPath => {
                write!(f, "invalid sweep curve path or open endpoint connectivity")
            }
            Self::InvalidJoin => write!(f, "invalid curve path miter limit"),
            Self::InvalidPlane => write!(f, "invalid closed curve path plane"),
            Self::NotClosed { start, end } => write!(
                f,
                "closed curve path ends at {end:?}, not its authored start {start:?}"
            ),
            Self::NonPlanar { segment } => {
                write!(f, "curve path segment {segment} leaves its declared plane")
            }
            Self::InvalidSegment { segment } => write!(f, "invalid path segment {segment}"),
            Self::StationaryTangent { segment, parameter } => write!(
                f,
                "path segment {segment} has no usable tangent at {parameter}"
            ),
            Self::DiscontinuousTangent { segment } => {
                write!(f, "path tangent is discontinuous before segment {segment}")
            }
            Self::SegmentBudgetExceeded { segment, maximum } => {
                write!(f, "path segment {segment} needs more than {maximum} spans")
            }
            Self::PathBudgetExceeded { maximum } => {
                write!(f, "path needs more than {maximum} spans")
            }
            Self::NumericLimit { segment } => {
                write!(f, "path segment {segment} exceeds f64 realization limits")
            }
        }
    }
}
impl core::error::Error for PathDiscretizeError {}

// Bounds must not underflow to zero (or overflow prematurely) when the
// same recipe is expressed in very small or large units.
fn norm(v: [f64; 3]) -> f64 {
    libm::hypot(libm::hypot(v[0], v[1]), v[2])
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let largest = v.iter().map(|x| x.abs()).fold(0.0_f64, f64::max);
    if largest == 0.0 || !largest.is_finite() || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let v = v.map(|x| x / largest);
    Some(scale(v, 1.0 / norm(v)))
}

fn midpoint(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    add(scale(a, 0.5), scale(b, 0.5))
}

fn split(p: [[f64; 3]; 4]) -> ([[f64; 3]; 4], [[f64; 3]; 4]) {
    let a = midpoint(p[0], p[1]);
    let b = midpoint(p[1], p[2]);
    let c = midpoint(p[2], p[3]);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let m = midpoint(d, e);
    ([p[0], a, d, m], [m, e, c, p[3]])
}

fn tangent(v: [f64; 3], segment: usize, parameter: f64) -> Result<[f64; 3], PathDiscretizeError> {
    unit(v).ok_or(PathDiscretizeError::StationaryTangent { segment, parameter })
}

// Fixed headroom covers up to 32 midpoint subdivisions. Below it we cannot
// distinguish tighter analytic bounds from coordinate rounding, so refuse.
fn headroom(
    points: &[[f64; 3]],
    segment: usize,
    tolerance: f64,
) -> Result<f64, PathDiscretizeError> {
    let scale = points
        .iter()
        .flatten()
        .map(|x| x.abs())
        .fold(0.0_f64, f64::max);
    let ulp = f64::from_bits(scale.to_bits() + 1) - scale;
    let margin = 128.0 * ulp;
    if !margin.is_finite() || margin >= tolerance {
        return Err(PathDiscretizeError::NumericLimit { segment });
    }
    Ok(margin)
}

fn budget_error(
    segment: usize,
    policy: &PathDiscretizePolicy,
    remaining: u32,
) -> PathDiscretizeError {
    if remaining < policy.max_segment_edges {
        PathDiscretizeError::PathBudgetExceeded {
            maximum: policy.max_path_edges,
        }
    } else {
        PathDiscretizeError::SegmentBudgetExceeded {
            segment,
            maximum: policy.max_segment_edges,
        }
    }
}

pub(crate) fn same_tangent(a: [f64; 3], b: [f64; 3]) -> bool {
    dot(a, b) > 0.0 && norm(cross(a, b)) <= 1e-10
}

// Check authored curves, not only sampled stations: a coarse cubic sampling
// cannot hide off-plane control points, nor can a small arc hide a tilted axis.
fn validate_planar_curves(
    start: [f64; 3],
    segments: &[PathSegment3],
    normal: [f64; 3],
) -> Result<(), PathDiscretizeError> {
    let normal = unit(normal).ok_or(PathDiscretizeError::InvalidPlane)?;
    for (segment, curve) in segments.iter().enumerate() {
        let check = |p: [f64; 3]| {
            let scale = start.iter().chain(&p).map(|v| v.abs()).fold(0.0, f64::max);
            let distance = dot(sub(p, start), normal).abs();
            distance.is_finite() && distance <= 64.0 * f64::EPSILON * scale
        };
        let valid = match curve {
            PathSegment3::Line { to } => check(*to),
            PathSegment3::Cubic {
                control1,
                control2,
                to,
            } => [*control1, *control2, *to].into_iter().all(check),
            PathSegment3::Arc { axis, .. } => {
                unit(*axis).is_some_and(|axis| norm(cross(axis, normal)) <= 64.0 * f64::EPSILON)
            }
        };
        if !valid {
            return Err(PathDiscretizeError::NonPlanar { segment });
        }
    }
    Ok(())
}

struct Sampler {
    output: DiscretizedPath,
    joins: PathJoin,
}
impl Sampler {
    fn begin(&mut self, station: PathStation, segment: usize) -> Result<(), PathDiscretizeError> {
        if let Some(previous) = self.output.stations.last_mut() {
            if self.joins == PathJoin::Smooth && !same_tangent(previous.tangent, station.tangent) {
                return Err(PathDiscretizeError::DiscontinuousTangent { segment });
            }
            if !same_tangent(previous.tangent, station.tangent) {
                self.output.sampling.corners.push(PathCorner {
                    station: crate::len_u32(self.output.sampling.spans.len()),
                    segment: crate::len_u32(segment),
                    turn_angle: libm::atan2(
                        norm(cross(previous.tangent, station.tangent)),
                        dot(previous.tangent, station.tangent),
                    ),
                });
            }
            previous.tangent = station.tangent;
        } else {
            self.output.stations.push(station);
        }
        Ok(())
    }
    fn emit(&mut self, station: PathStation, span: PathSpan) -> Result<(), PathDiscretizeError> {
        let previous = self.output.stations.last().expect("segment began");
        if station.point.iter().any(|x| !x.is_finite()) || station.point == previous.point {
            return Err(PathDiscretizeError::NumericLimit {
                segment: span.segment as usize,
            });
        }
        self.output.stations.push(station);
        self.output.sampling.spans.push(span);
        Ok(())
    }
}

/// Samples lines, circular arcs and spatial cubics with explicit accuracy and work limits.
///
/// Arc counts satisfy both sagitta and tangent-angle bounds. Cubics subdivide
/// by midpoint until their second-difference chord bound and derivative-control
/// cone bound both fit. No successful result silently clamps required work.
/// Subdivision performs at most `2 * max_segment_edges - 1` visits per cubic,
/// with at most 32 levels. Unresolved stationary tangents may exhaust that
/// budget; they never produce a successful under-resolved span.
///
/// `Smooth` joins require tangent continuity (direction, not parameter speed);
/// the 1e-10-radian allowance is only for numerical agreement. `Miter` retains
/// both tangents at an authored corner; tessellation checks its stretch limit.
/// Closed planar curves must terminate at exactly `start`, with no inferred
/// closing segment or endpoint snap. Authored controls/axes must lie in the
/// declared plane. The returned final station repeats the first for span checks.
/// Lines/cubics preserve authored endpoints. Arcs rotate about their authored
/// axis using libm. A complete turn returns the exact starting point/tangent.
///
/// # Errors
///
/// Returns [`PathDiscretizeError`] for invalid inputs, discontinuities,
/// unusable tangents, insufficient work budgets, or numeric limitations.
///
/// Migration: existing callers add `PathClosure::Open` and `PathJoin::Smooth`
/// before the discretization policy. Sampling evidence now includes connectivity,
/// join policy and authored corner records; stations retain both tangents.
pub fn discretize_path(
    start: [f64; 3],
    segments: &[PathSegment3],
    closure: PathClosure,
    joins: PathJoin,
    policy: &PathDiscretizePolicy,
) -> Result<DiscretizedPath, PathDiscretizeError> {
    policy.validate()?;
    if let PathJoin::Miter { limit } = joins
        && (!limit.is_finite() || limit < 1.0)
    {
        return Err(PathDiscretizeError::InvalidJoin);
    }
    if segments.is_empty() || start.iter().any(|v| !v.is_finite()) {
        return Err(PathDiscretizeError::InvalidPath);
    }
    if segments.len() > policy.max_path_edges as usize {
        return Err(PathDiscretizeError::PathBudgetExceeded {
            maximum: policy.max_path_edges,
        });
    }
    if let PathClosure::ClosedPlanar { normal } = closure {
        validate_planar_curves(start, segments, normal)?;
    }
    let mut sampler = Sampler {
        joins,
        output: DiscretizedPath {
            stations: Vec::new(),
            sampling: PathSampling {
                policy: *policy,
                closure,
                joins,
                corners: Vec::new(),
                spans: Vec::new(),
            },
        },
    };
    let mut from = start;
    for (segment, curve) in segments.iter().enumerate() {
        let used = crate::len_u32(sampler.output.sampling.spans.len());
        let remaining = policy.max_path_edges - used;
        if remaining == 0 {
            return Err(PathDiscretizeError::PathBudgetExceeded {
                maximum: policy.max_path_edges,
            });
        }
        let maximum = policy.max_segment_edges.min(remaining);
        match curve {
            PathSegment3::Line { to } => {
                if to.iter().any(|v| !v.is_finite()) || *to == from {
                    return Err(PathDiscretizeError::InvalidSegment { segment });
                }
                let t = tangent(sub(*to, from), segment, 0.0)?;
                sampler.begin(
                    PathStation {
                        point: from,
                        tangent: t,
                        incoming_tangent: t,
                    },
                    segment,
                )?;
                sampler.emit(
                    PathStation {
                        point: *to,
                        tangent: t,
                        incoming_tangent: t,
                    },
                    PathSpan {
                        segment: crate::len_u32(segment),
                        parameter: [0.0, 1.0],
                        chord_bound: 0.0,
                        tangent_angle_bound: 0.0,
                    },
                )?;
            }
            PathSegment3::Arc {
                axis_origin,
                axis,
                sweep,
            } => {
                sample_arc(
                    &mut sampler,
                    from,
                    *axis_origin,
                    *axis,
                    *sweep,
                    segment,
                    policy,
                    remaining,
                )?;
            }
            PathSegment3::Cubic {
                control1,
                control2,
                to,
            } => {
                let p = [from, *control1, *control2, *to];
                if p.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(PathDiscretizeError::InvalidSegment { segment });
                }
                let margin = headroom(&p, segment, policy.chord_tolerance)?;
                let first = tangent(sub(p[1], p[0]), segment, 0.0)?;
                tangent(sub(p[3], p[2]), segment, 1.0)?;
                sampler.begin(
                    PathStation {
                        point: from,
                        tangent: first,
                        incoming_tangent: first,
                    },
                    segment,
                )?;
                let mut stack = Vec::new();
                stack.push((p, 0.0, 1.0, 0_u8));
                let mut leaves = 1_u32;
                while let Some((p, t0, t1, depth)) = stack.pop() {
                    let d = [sub(p[1], p[0]), sub(p[2], p[1]), sub(p[3], p[2])];
                    let t_start = tangent(d[0], segment, t0)?;
                    let t_end = tangent(d[2], segment, t1)?;
                    let chord = 0.75 * norm(sub(d[1], d[0])).max(norm(sub(d[2], d[1]))) + margin;
                    let mut angle = 0.0_f64;
                    for v in d {
                        if v == [0.0; 3] {
                            continue;
                        }
                        let Some(v) = unit(v) else {
                            return Err(PathDiscretizeError::NumericLimit { segment });
                        };
                        // All derivative controls in this cone imply every
                        // positive Bernstein combination stays in the cone.
                        angle =
                            angle.max(2.0 * libm::atan2(norm(cross(t_start, v)), dot(t_start, v)));
                    }
                    if angle <= policy.max_tangent_angle {
                        // With nonnegative projections of every derivative
                        // control, endpoint Bernstein weights sum to at least
                        // one half. Reserve angular error for rounded controls
                        // as well as positional error for the chord bound.
                        let lower = 0.5 * dot(t_start, d[0]).min(dot(t_start, d[2]));
                        if lower <= 2.0 * margin || !lower.is_finite() {
                            return Err(PathDiscretizeError::NumericLimit { segment });
                        }
                        angle += 4.0 * libm::asin(2.0 * margin / lower);
                    }
                    if !chord.is_finite() || !angle.is_finite() {
                        return Err(PathDiscretizeError::NumericLimit { segment });
                    }
                    if chord <= policy.chord_tolerance && angle <= policy.max_tangent_angle {
                        sampler.emit(
                            PathStation {
                                point: p[3],
                                tangent: t_end,
                                incoming_tangent: t_end,
                            },
                            PathSpan {
                                segment: crate::len_u32(segment),
                                parameter: [t0, t1],
                                chord_bound: chord,
                                tangent_angle_bound: angle,
                            },
                        )?;
                    } else {
                        if leaves == maximum {
                            return Err(budget_error(segment, policy, remaining));
                        }
                        if depth == 32 {
                            return Err(PathDiscretizeError::NumericLimit { segment });
                        }
                        leaves += 1;
                        let (a, b) = split(p);
                        let mid = (t0 + t1) * 0.5;
                        stack.push((b, mid, t1, depth + 1));
                        stack.push((a, t0, mid, depth + 1));
                    }
                }
            }
        }
        from = sampler
            .output
            .stations
            .last()
            .expect("segment emitted")
            .point;
    }
    match closure {
        PathClosure::Open if from == start => return Err(PathDiscretizeError::InvalidPath),
        PathClosure::Open => {}
        PathClosure::ClosedPlanar { .. } => {
            if from != start {
                return Err(PathDiscretizeError::NotClosed { start, end: from });
            }
            let first = sampler.output.stations[0];
            let last = sampler.output.stations.last_mut().expect("nonempty path");
            if joins == PathJoin::Smooth && !same_tangent(last.incoming_tangent, first.tangent) {
                return Err(PathDiscretizeError::DiscontinuousTangent { segment: 0 });
            }
            if !same_tangent(last.incoming_tangent, first.tangent) {
                sampler.output.sampling.corners.insert(
                    0,
                    PathCorner {
                        station: 0,
                        segment: 0,
                        turn_angle: libm::atan2(
                            norm(cross(last.incoming_tangent, first.tangent)),
                            dot(last.incoming_tangent, first.tangent),
                        ),
                    },
                );
            }
            last.tangent = first.tangent;
            let seam = *last;
            sampler.output.stations[0] = seam;
        }
    }
    Ok(sampler.output)
}

fn sample_arc(
    sampler: &mut Sampler,
    from: [f64; 3],
    axis_origin: [f64; 3],
    axis: [f64; 3],
    sweep: f64,
    segment: usize,
    policy: &PathDiscretizePolicy,
    remaining: u32,
) -> Result<(), PathDiscretizeError> {
    if axis_origin.iter().any(|v| !v.is_finite())
        || !sweep.is_finite()
        || sweep == 0.0
        || sweep.abs() > core::f64::consts::TAU
    {
        return Err(PathDiscretizeError::InvalidSegment { segment });
    }
    let axis = unit(axis).ok_or(PathDiscretizeError::InvalidSegment { segment })?;
    let delta = sub(from, axis_origin);
    let radial = sub(delta, scale(axis, dot(delta, axis)));
    let radius = norm(radial);
    if !radius.is_finite() || radius == 0.0 {
        return Err(PathDiscretizeError::InvalidSegment { segment });
    }
    let center = sub(from, radial);
    let side = cross(axis, radial);
    let sign = sweep.signum();
    let first_t = tangent(scale(side, sign), segment, 0.0)?;
    let extrema = [
        from,
        axis_origin,
        center,
        add(center, [radius; 3]),
        sub(center, [radius; 3]),
    ];
    let margin = headroom(&extrema, segment, policy.chord_tolerance)?;
    let maximum = policy.max_segment_edges.min(remaining);
    let count = crate::discretize::circular_edge_count(
        radius,
        sweep.abs(),
        policy.chord_tolerance - margin,
        crate::discretize::CircularEdgeConstraints::new(1, maximum),
    );
    let mut count = count.map_err(|error| match error {
        crate::discretize::DiscretizeError::ToleranceBudgetExceeded { .. }
        | crate::discretize::DiscretizeError::EdgeCountOverflow => {
            budget_error(segment, policy, remaining)
        }
        _ => PathDiscretizeError::NumericLimit { segment },
    })?;
    let angular_margin = 32.0 * f64::EPSILON * sweep.abs();
    let (angle_bound, chord_bound) = loop {
        let angle = sweep.abs() / f64::from(count) + angular_margin;
        let sine = libm::sin(angle * 0.25);
        let chord = 2.0 * radius * sine * sine + margin;
        if angle <= policy.max_tangent_angle && chord <= policy.chord_tolerance {
            break (angle, chord);
        }
        if count == maximum {
            return Err(budget_error(segment, policy, remaining));
        }
        count += 1;
    };
    sampler.begin(
        PathStation {
            point: from,
            tangent: first_t,
            incoming_tangent: first_t,
        },
        segment,
    )?;
    for i in 1..=count {
        let t = f64::from(i) / f64::from(count);
        let angle = sweep * t;
        let (s, c) = (libm::sin(angle), libm::cos(angle));
        let point = if i == count && sweep.abs() == core::f64::consts::TAU {
            from
        } else {
            add(center, add(scale(radial, c), scale(side, s)))
        };
        let direction = if i == count && sweep.abs() == core::f64::consts::TAU {
            first_t
        } else {
            tangent(
                scale(add(scale(radial, -s), scale(side, c)), sign),
                segment,
                t,
            )?
        };
        sampler.emit(
            PathStation {
                point,
                tangent: direction,
                incoming_tangent: direction,
            },
            PathSpan {
                segment: crate::len_u32(segment),
                parameter: [f64::from(i - 1) / f64::from(count), t],
                chord_bound,
                tangent_angle_bound: angle_bound,
            },
        )?;
    }
    Ok(())
}

/// Structural IR validation only; analytic tangents and realizability belong
/// to sampling, where failures retain their segment and parameter.
pub(crate) fn validate_path_structure(
    start: [f64; 3],
    segments: &[PathSegment3],
) -> Result<(), PathDiscretizeError> {
    if segments.is_empty() || start.iter().any(|v| !v.is_finite()) {
        return Err(PathDiscretizeError::InvalidPath);
    }
    for (segment, curve) in segments.iter().enumerate() {
        let valid = match curve {
            PathSegment3::Line { to } => to.iter().all(|x| x.is_finite()),
            PathSegment3::Arc {
                axis_origin,
                axis,
                sweep,
            } => {
                axis_origin.iter().chain(axis).all(|x| x.is_finite())
                    && *axis != [0.0; 3]
                    && sweep.is_finite()
                    && *sweep != 0.0
                    && sweep.abs() <= core::f64::consts::TAU
            }
            PathSegment3::Cubic {
                control1,
                control2,
                to,
            } => control1
                .iter()
                .chain(control2)
                .chain(to)
                .all(|x| x.is_finite()),
        };
        if !valid {
            return Err(PathDiscretizeError::InvalidSegment { segment });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;
