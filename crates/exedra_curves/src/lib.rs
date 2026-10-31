// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic curve geometry: cubic and rational B-spline evaluation,
//! explicit sampling and rotation-minimizing frames.
//!
//! The crate knows curves as mathematics and nothing of any scene format: a
//! caller (a USD reader, a procedural generator) decides which controls
//! form which spans, how its per-control data is laid out, and how finely
//! to sample; this crate evaluates and frames.
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
//!   B-splines of any order, validated once, evaluated with derivatives by
//!   de Boor's algorithm over homogeneous controls, sampled a stated number
//!   of times per nonempty knot span with the jumps at full-multiplicity
//!   knots sampled on both sides and marked, and prepared as per-span
//!   Bezier controls for repeated evaluation.
//! - **Frames** ([`rotation_minimizing_frames`],
//!   [`rotation_minimizing_frames_along`]): the double-reflection transport
//!   shared with Exedra's sweeps (`exedra_math::path::double_reflection`),
//!   along estimated or supplied tangents, with repeated points, cusps and
//!   a closed loop's signed seam twist reported.
//! - **Reusable buffers**: every allocating function has an `_into` form
//!   that fills a caller's buffers, and those that need scratch take a
//!   [`Workspace`], so a caller that keeps them allocates nothing once they
//!   have grown.
//!
//! ```
//! use exedra_curves::{CubicBasis, sample, stations};
//!
//! // One Catmull-Rom span through (1, 0, 0) and (2, 0, 0).
//! let controls = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]];
//! let at = stations(1, 4, false);
//! let points = sample(CubicBasis::CatmullRom, &controls, &[[0, 1, 2, 3]], &at).unwrap();
//! assert_eq!(points.len(), 5);
//! assert_eq!(points[0], [1.0, 0.0, 0.0]);
//! assert_eq!(points[4], [2.0, 0.0, 0.0]);
//! ```
//!
//! # Scope
//!
//! A file format's indexing rules are its reader's, not this crate's: which
//! controls form a span (`OpenUSD`'s segment counts and `vstep`, periodic
//! wrapping, pinned end conditions) and how per-control data follows a
//! curve (primvar interpolation classes). The reader builds [`Span`]s and
//! joints from those rules; the evaluation and framing here are the same
//! mathematics for every format and for procedural generators.
//!
//! # Contracts
//!
//! - **Explicit counts.** Samples per span are inputs. Nothing adapts them
//!   to curvature or screen size.
//! - **Stated parameters.** Every derivative is with respect to the
//!   parameter its function names: a cubic span's `u`, or a NURBS curve's
//!   `t`.
//! - **Degeneracy is reported, not repaired.** Repeated points, cusps and a
//!   closed loop's signed seam twist are counted in [`FrameReport`];
//!   nothing is moved, dropped or untwisted.
//! - **Parallel means within `2^-26` radians.** A seed or transported
//!   normal is made perpendicular to a tangent only when its perpendicular
//!   part exceeds `sqrt(EPSILON)` of its length; below that the part is
//!   rounding, and the documented fallback applies. Two directions
//!   reversing to within the same angle are a cusp.
//! - **Inputs are checked where they enter.** [`Nurbs::validate`] checks a
//!   curve's knots, weights and controls once; sampling and framing
//!   propagate a non-finite value rather than scanning on every call.
//! - **NURBS are validated once.** [`Nurbs::validate`] returns the
//!   [`ValidNurbs`] that evaluates and samples, with finite knots, weights
//!   and weighted controls; an interior knot of multiplicity `order` or
//!   more is valid and leaves the curve discontinuous, and sampling takes
//!   both sides of it and lists the break ([`NurbsSamples::breaks`]), so no
//!   caller joins across a jump unknowingly.
//!
//! Not provided: adaptive or tolerance-driven sampling, arc length, and
//! distributing a closed loop's seam twist. Each would be a new, explicit
//! option rather than a change of these functions' results.
//!
//! # Determinism
//!
//! Evaluation, sampling, and frames use only additions, subtractions,
//! multiplications, divisions and square roots, in a documented fixed
//! order. IEEE 754 rounds each of those correctly and Rust does not fuse
//! them, so equal inputs give equal bits on every platform and under either
//! feature.
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
mod nurbs;
mod sample;
mod workspace;

pub use basis::{CubicBasis, CurveEvaluation};
pub use frames::{
    Frame, FrameReport, TangentCount, rotation_minimizing_frames, rotation_minimizing_frames_along,
    rotation_minimizing_frames_into,
};
pub use nurbs::{Nurbs, NurbsError, NurbsSamples, PreparedNurbs, ValidNurbs};
pub use sample::{
    ControlOutOfRange, Span, Station, sample, sample_derivative, sample_derivative_into,
    sample_into, sample_linear, sample_linear_into, stations, stations_into,
};
pub use workspace::Workspace;
