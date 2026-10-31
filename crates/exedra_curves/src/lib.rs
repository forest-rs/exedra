// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic curve geometry: cubic curve evaluation and explicit
//! sampling.
//!
//! The crate knows curves as mathematics and nothing of any scene format: a
//! caller (a USD reader, a procedural generator) decides which controls
//! form which spans, how its per-control data is laid out, and how finely
//! to sample; this crate evaluates.
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
//! - **Reusable buffers**: every allocating function has an `_into` form
//!   that fills a caller's buffers, so a caller that keeps them allocates
//!   nothing once they have grown.
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
//! joints from those rules; the evaluation here is the same mathematics for
//! every format and for procedural generators.
//!
//! # Contracts
//!
//! - **Explicit counts.** Samples per span are inputs. Nothing adapts them
//!   to curvature or screen size.
//! - **Stated parameters.** Every derivative is with respect to the
//!   parameter its function names: a cubic span's `u`.
//! - **Inputs are checked where they enter.** Sampling propagates a
//!   non-finite value to the samples that use it rather than scanning on
//!   every call; a reader checks authored data once, where it reads it.
//!
//! Not provided: adaptive or tolerance-driven sampling and arc length. Each
//! would be a new, explicit option rather than a change of these functions'
//! results.
//!
//! # Determinism
//!
//! Evaluation and sampling use only additions, subtractions,
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
mod sample;

pub use basis::{CubicBasis, CurveEvaluation};
pub use sample::{
    ControlOutOfRange, Span, Station, sample, sample_derivative, sample_derivative_into,
    sample_into, sample_linear, sample_linear_into, stations, stations_into,
};
