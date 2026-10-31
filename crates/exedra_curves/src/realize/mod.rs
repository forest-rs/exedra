// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Realization: from an analytic curve to drawable output, planned once
//! and updated per field.
//!
//! The rest of the crate evaluates, samples, frames and sweeps curves one
//! call at a time. A drawing consumer (a scene renderer, an editor
//! animating hair or cables) instead keeps each curve and redraws it
//! whenever some of its inputs change. This module is that layer, in
//! three steps:
//!
//! 1. A [`CurveDescription`] is the analytic input: a [`Centerline`] (a
//!    cubic chain of [`Span`]s, or an owned [`NurbsCurve`]), widths and
//!    optional ribbon normals as [`Attribute`]s, and whether the curve
//!    closes.
//! 2. A [`RealizationPlan`] is built from it once. It owns the description
//!    and everything derived from it, kept in three layers that change for
//!    different reasons:
//!    - **topology**, from the knots, span indices, basis, closure and
//!      [`PlanOptions`]: each span's Bezier extraction matrix, its pieces
//!      (runs of spans between [breaks](crate::PreparedNurbs::breaks)), the
//!      stations where frames are kept, and which spans each control
//!      shapes. Built by [`RealizationPlan::new`] and never again: a
//!      change of topology is a new plan.
//!    - **controls**, from the points and weights: each span's Bezier
//!      controls (the extraction matrix applied, no knot arithmetic), its
//!      bounds, its joints' creases and gaps, the points and analytic
//!      tangents at the stations, and the rotation-minimizing frames.
//!      [`RealizationPlan::set_points`] and
//!      [`RealizationPlan::set_weights`] redo this only for the spans the
//!      changed controls shape, and reframe only from the first changed
//!      station on: frames are carried along the curve, so those before
//!      it cannot change.
//!    - **attributes**, from the widths and normals: their Bezier
//!      coefficients per span and their values at the stations.
//!      [`RealizationPlan::set_widths`] and
//!      [`RealizationPlan::set_normals`] touch nothing else.
//!
//!    Every update returns a [`PlanWork`] counting what it redid, and the
//!    plan's [`Generations`] say which layers changed since a consumer
//!    last looked. Nothing is cached anywhere else and nothing is computed
//!    lazily: a plan's derived state is current when the update returns.
//! 3. A realization reads a plan into a caller's buffers:
//!    - **CPU meshes**: [`RealizationPlan::tube_into`] and
//!      [`RealizationPlan::ribbon_into`] sweep the plan's frames with the
//!      crate's [`tube_into`](crate::tube_into) and
//!      [`ribbon_into`](crate::ribbon_into), piece by piece, into one
//!      [`SweptMesh`](crate::SweptMesh);
//!    - **GPU data**: [`RealizationPlan::gpu_into`] writes a [`GpuCurve`],
//!      plain `f32` and `u32` arrays a shader tessellates (the layout is
//!      documented there), and reports which element ranges it changed
//!      ([`GpuReport`]), so a renderer uploads only those.
//!
//! # Stations
//!
//! A plan samples each span a fixed [`PlanOptions::per_span`] times, at
//! `u = k / per_span`, and each open piece also at its end, so the station
//! count, and with it every mesh's vertex and index counts, depends on
//! topology alone: moving controls never resizes a mesh or invalidates an
//! index buffer. A cubic chain's stations are [`stations`](crate::stations)'
//! exactly, and its points, tangents and widths are
//! [`sample`](crate::sample())'s and [`sample_derivative`](crate::sample_derivative)'s,
//! bit for bit, so a plan's tube is the one the crate's functions sweep
//! directly. A NURBS curve's stations fall where
//! [`ValidNurbs::sample`](crate::ValidNurbs::sample) places them, each
//! piece ending at its break, and are evaluated on the plan's Bezier
//! controls at their own `u` rather than at the knot parameter, which a
//! knot interval far from zero cannot represent at every `u`; points agree
//! with [`PreparedNurbs`](crate::PreparedNurbs)' to rounding, and the
//! GPU data evaluates at the same `u`.
//! Tolerance-driven stations would make the count depend on the points,
//! and are not offered here.
//!
//! # Joints
//!
//! Where one span ends and the next starts (and where a closed curve's
//! last span meets its first) the plan compares the one-sided ends:
//!
//! - a **crease** is a joint whose one-sided tangents differ by more than
//!   `2^-26` radians, or where either is zero, as at a Bezier chain's
//!   unaligned handles;
//! - a **gap** is a joint whose one-sided points are farther apart than
//!   `2^-26` of their largest coordinate's magnitude, as where a caller's
//!   spans do not share their joint control;
//! - a **break** is structural: a NURBS knot of full multiplicity, where
//!   the curve may jump. Each side is its own piece, framed and swept
//!   separately.
//!
//! Creases and gaps are counted ([`PlanReport`]) and flagged per span in
//! the GPU data, never repaired: stations do not move and the frames
//! carry across them, as
//! [`rotation_minimizing_frames`](crate::rotation_minimizing_frames)
//! carries them across any station.
//!
//! # Attributes
//!
//! An [`Attribute::Vertex`] value per control follows the curve's own
//! basis: a cubic chain's [`CubicBasis`], or a NURBS curve's B-spline
//! basis functions without its weights, so weights shape positions only
//! and a width stays a polynomial a shader evaluates directly. A
//! [`Attribute::Varying`] value per joint is linear across each span, as
//! [`sample_linear`](crate::sample_linear) interpolates it. Widths are
//! full widths, a tube's diameter (`OpenUSD`'s `widths`): a tube's radius
//! is half the width.
//!
//! ```
//! use exedra_curves::realize::{Attribute, Centerline, CurveDescription, PlanOptions, RealizationPlan};
//! use exedra_curves::{CubicBasis, SweptMesh, Workspace};
//!
//! let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 1.0, 0.0], [3.0, 1.0, 0.0], [4.0, 0.0, 0.0]];
//! let description = CurveDescription {
//!     centerline: Centerline::Cubic {
//!         basis: CubicBasis::CatmullRom,
//!         points: points.clone(),
//!         spans: vec![[0, 1, 2, 3], [1, 2, 3, 4]],
//!     },
//!     widths: Attribute::Constant(0.1),
//!     normals: None,
//!     closed: false,
//! };
//! let mut workspace = Workspace::new();
//! let mut plan = RealizationPlan::new(description, PlanOptions::new(8), &mut workspace).unwrap();
//! let mut mesh = SweptMesh::default();
//! plan.tube_into(6, &mut workspace, &mut mesh).unwrap();
//! assert_eq!(mesh.report.vertices, 17 * 6);
//!
//! // Move the last control: only the span it shapes is evaluated again.
//! let mut moved = points;
//! moved[4] = [4.0, -1.0, 0.0];
//! let work = plan.set_points(&moved, &mut workspace).unwrap();
//! assert_eq!(work.extracted_spans, 0);
//! assert_eq!(work.changed_controls, 1);
//! assert_eq!(work.bezier_spans, 1);
//! assert_eq!(work.evaluated_stations, 9);
//! ```

mod gpu;
mod plan;

use alloc::vec::Vec;

pub use gpu::{GpuCurve, GpuReport, GpuSpan};
pub use plan::{Piece, RealizationPlan};

use crate::{CubicBasis, FrameReport, NurbsCurve, Span, SweepError};

/// A curve's analytic centerline.
#[derive(Clone, Debug, PartialEq)]
pub enum Centerline {
    /// A chain of cubic spans: each [`Span`] names four of `points`,
    /// weighted by `basis` (see [`crate::sample()`]). The caller's indexing
    /// rules (stepping, wrapping, end conditions) are already in `spans`.
    Cubic {
        /// The basis every span is evaluated in.
        basis: CubicBasis,
        /// The controls.
        points: Vec<[f64; 3]>,
        /// The spans, in order along the curve, at least one.
        spans: Vec<Span>,
    },
    /// A rational B-spline, already validated. Its nonempty knot spans are
    /// the plan's spans; a knot of full multiplicity starts a new piece.
    Nurbs(NurbsCurve),
}

impl Centerline {
    /// How many controls the centerline has.
    #[must_use]
    pub fn controls(&self) -> usize {
        match self {
            Self::Cubic { points, .. } => points.len(),
            Self::Nurbs(curve) => curve.points().len(),
        }
    }
}

/// A value carried along a curve, in one of `OpenUSD`'s curve
/// interpolation classes (`pxr/usd/usdGeom/basisCurves.h`, "Interpolation
/// of Primvars"); `uniform` is `Constant` for one curve.
#[derive(Clone, Debug, PartialEq)]
pub enum Attribute<T> {
    /// One value for the whole curve.
    Constant(T),
    /// One value per joint, linear across each span: span `s` runs from
    /// value `s` to value `s + 1`, wrapping to 0 on a closed curve, so an
    /// open curve has `spans + 1` values and a closed one `spans`. A NURBS
    /// curve's break shares its joint's one value.
    Varying(Vec<T>),
    /// One value per control, following the curve's basis.
    Vertex(Vec<T>),
}

/// A curve to realize: its centerline, widths and ribbon normals, and
/// whether it closes.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveDescription {
    /// The analytic centerline.
    pub centerline: Centerline,
    /// Full widths, finite and nonnegative: a ribbon's width, a tube's
    /// diameter.
    pub widths: Attribute<f64>,
    /// The direction a ribbon faces, made perpendicular to the tangent
    /// wherever it is used; `None` faces the frames' normals. Tubes ignore
    /// them.
    pub normals: Option<Attribute<[f64; 3]>>,
    /// Whether the curve's end joins its start: no end station, frames
    /// framed as a loop (its seam twist reported), and sweeps closed. A
    /// cubic chain's spans must already wrap; a closed curve has one
    /// piece, so a NURBS curve with a break cannot close.
    pub closed: bool,
}

/// How a [`RealizationPlan`] samples and frames its curve. Part of its
/// topology: a plan with different options is a new plan.
///
/// Non-exhaustive, so it can take more options later; start from
/// [`Self::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlanOptions {
    /// Stations per span, at `u = k / per_span`; 0 is taken as 1.
    pub per_span: u32,
    /// The seed of each piece's first normal, as for
    /// [`rotation_minimizing_frames`](crate::rotation_minimizing_frames);
    /// `None` takes the world axis least aligned with the first tangent.
    pub seed: Option<[f64; 3]>,
}

impl PlanOptions {
    /// `per_span` stations per span, with no seed.
    #[must_use]
    pub const fn new(per_span: u32) -> Self {
        Self {
            per_span,
            seed: None,
        }
    }

    /// These options with `seed` as each piece's first normal.
    #[must_use]
    pub const fn with_seed(self, seed: [f64; 3]) -> Self {
        Self {
            seed: Some(seed),
            ..self
        }
    }
}

/// The input a [`RealizeError`] concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Field {
    /// The centerline's controls.
    Points,
    /// A NURBS centerline's weights.
    Weights,
    /// The widths.
    Widths,
    /// The ribbon normals.
    Normals,
}

impl Field {
    const fn name(self) -> &'static str {
        match self {
            Self::Points => "points",
            Self::Weights => "weights",
            Self::Widths => "widths",
            Self::Normals => "normals",
        }
    }
}

/// Why a plan cannot be built or updated, or a realization made.
///
/// A plan whose update fails is left as it was before the call.
///
/// Non-exhaustive: a later check may add a variant without breaking a
/// caller that reports the error through `Display` or a fallback arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RealizeError {
    /// A cubic chain has no spans.
    NoSpans,
    /// A cubic span names a control the chain does not have.
    Control(crate::ControlOutOfRange),
    /// A field has the wrong number of values: for points and weights,
    /// one per control; for an attribute, as its [`Attribute`] class
    /// needs.
    Count {
        /// The field.
        field: Field,
        /// The values given.
        given: usize,
        /// The values needed.
        expected: usize,
    },
    /// A value is out of range: a point or weight
    /// [`crate::Nurbs::validate`] would refuse (a coordinate not finite, a
    /// weight outside `[2^-256, 2^256]`, or a weighted coordinate
    /// overflowing or leaving the normal range), a normal not finite, or a
    /// width negative or not finite.
    Value {
        /// The field.
        field: Field,
        /// The value's index in it.
        index: usize,
    },
    /// Weights were given for a cubic chain, which is polynomial.
    NotRational,
    /// A closed curve has breaks, so its end cannot join its start as one
    /// loop.
    ClosedWithBreaks {
        /// The breaks.
        breaks: usize,
    },
    /// The plan would hold more controls, Bezier controls or stations than
    /// a `u32` counts, or arrays larger than memory can address.
    TooLarge,
    /// A sweep failed, as its [`SweepError`] says; a station index there
    /// counts the plan's stations, across pieces.
    Sweep(SweepError),
    /// A span's GPU data is outside what a shader evaluates safely in
    /// `f32` ([`GpuCurve`]'s layout): a point relative to the origin, a
    /// width or a normal coefficient beyond `2^64` in magnitude, or
    /// weights differing by more than `2^32` within the span.
    F32Range {
        /// The span.
        span: usize,
    },
}

impl core::fmt::Display for RealizeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::NoSpans => f.write_str("the cubic chain has no spans"),
            Self::Control(error) => write!(f, "{error}"),
            Self::Count {
                field,
                given,
                expected,
            } => write!(f, "{given} {} where {expected} are needed", field.name()),
            Self::Value { field, index } => {
                write!(f, "{} value {index} is out of range", field.name())
            }
            Self::NotRational => f.write_str("a cubic chain takes no weights"),
            Self::ClosedWithBreaks { breaks } => {
                write!(f, "a closed curve has {breaks} breaks")
            }
            Self::TooLarge => f.write_str("the plan is too large to count or hold"),
            Self::Sweep(error) => write!(f, "{error}"),
            Self::F32Range { span } => write!(f, "span {span}'s GPU data does not fit f32"),
        }
    }
}

impl core::error::Error for RealizeError {}

impl From<SweepError> for RealizeError {
    fn from(error: SweepError) -> Self {
        Self::Sweep(error)
    }
}

/// What one plan call computed, in units of spans, stations and controls,
/// so a caller can see that an update redid only what it had to.
///
/// [`RealizationPlan::new`]'s is in [`PlanReport::built`]; each update
/// returns its own. An update that changed nothing returns all zeros.
///
/// Non-exhaustive, so it can count more later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanWork {
    /// Spans whose topology was built: their extraction matrix (from the
    /// knots, for NURBS), stations and controls. Nonzero only when a plan
    /// is built.
    pub extracted_spans: u64,
    /// Controls whose point or weight differed from the plan's.
    pub changed_controls: u64,
    /// Spans whose Bezier controls and bounds were recomputed.
    pub bezier_spans: u64,
    /// Stations whose point and tangent were evaluated.
    pub evaluated_stations: u64,
    /// Joints whose crease and gap were checked.
    pub checked_joints: u64,
    /// Stations framed: from the first changed station to the end of its
    /// piece, since each frame is carried from the one before it; the
    /// whole piece when one of its tangents is zero and estimated from
    /// chords.
    pub framed_stations: u64,
    /// Spans whose attribute Bezier coefficients were recomputed, summed
    /// over widths and normals.
    pub attribute_spans: u64,
    /// Stations whose attribute values were evaluated, summed over widths
    /// and normals.
    pub attribute_stations: u64,
}

/// Counters of a plan's changes, one per layer an update can change.
/// Each starts at 0 and grows by one whenever an update changes that
/// layer, never on an update that changed nothing.
///
/// A consumer that derives data of its own from a plan keeps the
/// generations it derived from, and redoes its work only when they
/// differ.
///
/// Non-exhaustive, so it can count more layers later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Generations {
    /// The points and weights, and everything derived from them: Bezier
    /// controls, bounds, joints, station points, tangents and frames.
    pub controls: u64,
    /// The widths, and their coefficients and station values.
    pub widths: u64,
    /// The ribbon normals, and their coefficients and station values.
    pub normals: u64,
}

/// The state of a plan: its sizes, what its joints and frames met, what
/// building it cost and its generations.
///
/// Non-exhaustive, so it can report more later.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlanReport {
    /// Controls per span: 4 for a cubic chain, the order for NURBS.
    pub order: u32,
    /// Spans.
    pub spans: u32,
    /// Pieces: 1 plus the breaks.
    pub pieces: u32,
    /// Stations, over all pieces.
    pub stations: u32,
    /// Joints that are [creases](self#joints).
    pub creases: u32,
    /// Joints that are [gaps](self#joints).
    pub gaps: u32,
    /// What framing met, summed over the pieces, with the first cusp
    /// counted in the plan's stations; the seam twist is the closed
    /// piece's (an open curve's is 1 and 0).
    pub frames: FrameReport,
    /// What [`RealizationPlan::new`] computed.
    pub built: PlanWork,
    /// The plan's generations.
    pub generations: Generations,
}
