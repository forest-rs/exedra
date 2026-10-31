// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scratch storage reused across calls.

use alloc::vec::Vec;

use crate::measure::Refined;
use crate::vector::WeightedPoint;

/// Scratch storage for the crate's `_into` functions, for curves in `N`
/// dimensions: plane curves for `N = 2`, space curves for the default
/// `N = 3`, which framing and sweeping also take.
///
/// A caller that evaluates many curves, or the same curves every frame,
/// keeps one `Workspace` and its output buffers and passes them to each
/// call: once they have grown to the largest curve's needs, nothing more
/// is allocated. The workspace holds no results; each call overwrites the
/// parts it uses and any call may grow it. The allocating functions use
/// fresh scratch of their own.
#[derive(Clone, Debug, Default)]
pub struct Workspace<const N: usize = 3> {
    /// Weighted NURBS controls for de Boor's scheme and blossoming.
    pub(crate) homogeneous: Vec<WeightedPoint<N>>,
    /// De Casteljau's scheme for evaluating a prepared span.
    pub(crate) evaluation: Vec<WeightedPoint<N>>,
    /// Chord directions of a framed curve, `None` where points repeat.
    pub(crate) directions: Vec<Option<[f64; 3]>>,
    /// The first direction leaving each framed point.
    pub(crate) outgoing: Vec<Option<[f64; 3]>>,
    /// A tube ring's `(cos, sin)` directions, for `ring_sides` sides.
    pub(crate) ring: Vec<(f64, f64)>,
    /// The side count `ring` holds, 0 before the first tube.
    pub(crate) ring_sides: u32,
    /// Adaptive sampling's current samples.
    pub(crate) refined: Vec<Refined<N>>,
    /// Adaptive sampling's next round.
    pub(crate) refining: Vec<Refined<N>>,
    /// The Bezier control polygon of the interval being tested.
    pub(crate) polygon: Vec<[f64; N]>,
}

impl<const N: usize> Workspace<N> {
    /// An empty workspace; it grows on first use.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            homogeneous: Vec::new(),
            evaluation: Vec::new(),
            directions: Vec::new(),
            outgoing: Vec::new(),
            ring: Vec::new(),
            ring_sides: 0,
            refined: Vec::new(),
            refining: Vec::new(),
            polygon: Vec::new(),
        }
    }
}
