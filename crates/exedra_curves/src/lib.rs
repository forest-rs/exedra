// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic curve geometry: cubic and rational B-spline evaluation,
//! explicit sampling, rotation-minimizing frames, tubes and ribbons, and
//! arc length and tolerance-driven sampling.
//!
//! The crate knows curves as mathematics and nothing of any scene format: a
//! caller (a USD reader, a procedural generator) decides which controls
//! form which spans, how its per-control data is laid out, and how finely
//! to sample; this crate evaluates, frames, sweeps and measures.
//!
//! - **Cubic bases** ([`CubicBasis`]): Bezier, uniform B-spline,
//!   Catmull-Rom and Hermite, as their explicit basis matrices (the
//!   `RenderMan` table; [`CubicBasis::matrix`]), evaluated with first and
//!   second derivatives ([`CurveEvaluation`]) for controls of any dimension
//!   so positions, widths and colors follow one span alike.
//! - **Spans and sampling** ([`Span`], [`stations`], [`sample()`],
//!   [`sample_derivative`], [`sample_linear`]): a span is four control
//!   indices; a chain of spans is sampled a stated number of times per
//!   span, at stations a caller can list before evaluating, so every value
//!   sampled at the same stations lines up.
//! - **NURBS** ([`Nurbs`], [`ValidNurbs`], [`PreparedNurbs`]): rational
//!   B-splines of any order in any dimension, space curves by default and
//!   plane curves for profiles and trims, validated once, evaluated with
//!   derivatives by de Boor's algorithm over [weighted
//!   controls](WeightedPoint), sampled a stated number of times per
//!   nonempty knot span with the jumps at full-multiplicity knots sampled on
//!   both sides and marked, and prepared as per-span Bezier controls for
//!   repeated evaluation. A [`Nurbs`] borrows its
//!   caller's buffers, so rebuilding curves allocates nothing; a
//!   [`NurbsCurve`] owns its own, for curves kept as values.
//! - **Frames** ([`rotation_minimizing_frames`],
//!   [`rotation_minimizing_frames_along`]): the double-reflection transport
//!   shared with Exedra's sweeps (`exedra_math::path::double_reflection`),
//!   along estimated or supplied tangents, with repeated points, cusps and
//!   a closed loop's signed seam twist reported.
//! - **Sweeps** ([`tube`], [`ribbon`]): triangles around or along the
//!   frames at a stated side count, with a finite, nonnegative radius or
//!   width per station and a tube's surface normals following its taper, as
//!   plain buffers and a [`SweepReport`] of their counts.
//! - **Measuring** ([`CubicBasis::arc_length`], [`chain_lengths`],
//!   [`PreparedNurbs::lengths`], [`adaptive_stations`],
//!   [`PreparedNurbs::adaptive_sample`]): arc length by stated
//!   Gauss-Legendre quadrature with the parameter at a length, and sampling
//!   refined to a chord or angle [`Tolerance`] under a cap, reported.
//! - **Reusable buffers**: every allocating function has an `_into` form
//!   that fills a caller's buffers, and those that need scratch take a
//!   [`Workspace`], so a caller that keeps them allocates nothing once they
//!   have grown. Functions that take no workspace keep a NURBS curve's
//!   scratch on the stack up to order 8, and allocate it past that.
//!
//! ```
//! use exedra_curves::{CubicBasis, rotation_minimizing_frames, sample, stations, tube};
//!
//! // One Catmull-Rom span through (1, 0, 0) and (2, 0, 0).
//! let controls = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]];
//! let at = stations(1, 4, false);
//! let points = sample(CubicBasis::CatmullRom, &controls, &[[0, 1, 2, 3]], &at).unwrap();
//! assert_eq!(points.len(), 5);
//! assert_eq!(points[0], [1.0, 0.0, 0.0]);
//! assert_eq!(points[4], [2.0, 0.0, 0.0]);
//!
//! let (frames, _) = rotation_minimizing_frames(&points, false, None);
//! let mesh = tube(&frames, &[0.1; 5], 6, false).unwrap();
//! assert_eq!(mesh.report.vertices, 5 * 6);
//! assert_eq!(mesh.report.triangles, 4 * 6 * 2);
//! ```
//!
//! # Scope
//!
//! A file format's indexing rules are its reader's, not this crate's: which
//! controls form a span (`OpenUSD`'s segment counts and `vstep`, periodic
//! wrapping, pinned end conditions) and how per-control data follows a
//! curve (primvar interpolation classes). The reader builds [`Span`]s and
//! joints from those rules; the evaluation, framing and sweeping here are
//! the same mathematics for every format and for procedural generators.
//!
//! # Contracts
//!
//! - **Explicit counts.** Samples per span and sides per ring are inputs.
//!   Nothing adapts them unless a caller asks for tolerance-driven
//!   sampling, which caps its samples past each span's own ends, and
//!   reports whether the cap stopped it or an interval too narrow to split
//!   still failed.
//! - **Stated parameters.** Every derivative is with respect to the
//!   parameter its function names: a cubic span's `u`, or a NURBS curve's
//!   `t`.
//! - **Degeneracy is reported, not repaired.** Repeated points, cusps,
//!   collapsed rings, ribbon normals parallel to the tangent and a closed
//!   loop's signed seam twist are counted in [`FrameReport`] and
//!   [`SweepReport`]; nothing is moved, dropped or untwisted.
//! - **Parallel means within `2^-26` radians.** A seed, ribbon normal or
//!   transported normal is made perpendicular to a tangent only when its
//!   perpendicular part exceeds `sqrt(EPSILON)` of its length; below that
//!   the part is rounding, and the documented fallback applies. Two
//!   directions reversing to within the same angle are a cusp.
//! - **Sizes are finite and nonnegative.** Interpolated widths can
//!   undershoot (a Catmull-Rom chain of nonnegative widths does); a sweep
//!   refuses such a size ([`SweepError::Size`]) rather than choosing a
//!   repair for the caller. Zero is swept and reported.
//! - **Counts are planned first.** A sweep checks its vertex, triangle and
//!   index counts before any allocation or trigonometry.
//! - **Inputs are checked where they enter.** [`Nurbs::validate`] checks a
//!   curve's knots, weights and controls once, and the sweeps check their
//!   sizes; sampling and framing propagate a non-finite value rather than
//!   scanning on every call.
//! - **NURBS are validated once.** [`Nurbs::validate`] returns the
//!   [`ValidNurbs`] that evaluates and samples, with finite knots, weights
//!   and weighted controls; an interior knot of multiplicity `order` or
//!   more is valid and leaves the curve discontinuous, and sampling takes
//!   both sides of it and lists the break ([`NurbsSamples::breaks`]), so no
//!   caller joins across a jump unknowingly.
//! - **Estimates are labelled.** Arc lengths are quadrature estimates.
//!   Adaptive sampling's tolerances are bounds, up to rounding, taken from
//!   each interval's Bezier control polygon.
//!
//! Not provided: end caps and distributing a closed loop's seam twist. Each
//! would be a new, explicit option rather than a change of these functions'
//! results.
//!
//! # Determinism
//!
//! Evaluation, sampling, measuring, frames, ribbons, and a tube's counts,
//! indices and normal weights use only additions, subtractions,
//! multiplications, divisions and square roots, in a documented fixed
//! order. IEEE 754 rounds each of those correctly and Rust does not fuse
//! them, so equal inputs give equal bits on every platform and under either
//! feature.
//!
//! A tube's ring directions are the exception: they come from
//! `exedra_math`'s `sin_cos`, which is not correctly rounded. With the
//! `libm` feature that is libm's pure-Rust implementation, the same bits
//! everywhere; with `std` alone it is the platform's, whose precision Rust
//! leaves unspecified, so ring positions and normals can differ in their
//! last bits between platforms or toolchains. An angle [`Tolerance`] is
//! turned into its cosine the same way, once per call.
//!
//! A faster path that changes the operation order (prepared power
//! coefficients, reassociated SIMD reductions, fused multiply-adds) is to
//! be a separate, explicit choice with its own error contract, not a change
//! to these functions' results.
//!
//! # Features
//!
//! - `std` (default): `exedra_math` arithmetic through the standard
//!   library.
//! - `libm`: the same through `libm`, for `no_std` builds.

#![no_std]
extern crate alloc;

mod basis;
mod frames;
mod knots;
mod measure;
mod nurbs;
mod sample;
mod sweep;
mod vector;
mod workspace;

pub use basis::{CubicBasis, CurveEvaluation};
pub use frames::{
    Frame, FrameReport, TangentCount, rotation_minimizing_frames, rotation_minimizing_frames_along,
    rotation_minimizing_frames_into,
};
pub use measure::{
    AdaptiveReport, Tolerance, adaptive_stations, adaptive_stations_into, chain_lengths,
    chain_lengths_into, chain_station_at_length,
};
pub use nurbs::{Nurbs, NurbsCurve, NurbsError, NurbsSamples, PreparedNurbs, ValidNurbs};
pub use sample::{
    ControlOutOfRange, Span, Station, sample, sample_derivative, sample_derivative_into,
    sample_into, sample_linear, sample_linear_into, stations, stations_into,
};
pub use sweep::{SweepError, SweepReport, SweptMesh, ribbon, ribbon_into, tube, tube_into};
pub use vector::WeightedPoint;
pub use workspace::Workspace;
