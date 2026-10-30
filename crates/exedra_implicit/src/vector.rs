// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vector arithmetic over numbers, for formulas.

use crate::number::Number;

/// `p - c`.
pub(crate) fn offset<N: Number>(p: [N; 3], c: [f64; 3]) -> [N; 3] {
    [p[0] - c[0], p[1] - c[1], p[2] - c[2]]
}

/// `v · direction`.
pub(crate) fn along<N: Number>(v: [N; 3], direction: [f64; 3]) -> N {
    v[0] * direction[0] + v[1] * direction[1] + v[2] * direction[2]
}

/// `v - direction * t`.
pub(crate) fn less<N: Number>(v: [N; 3], direction: [f64; 3], t: N) -> [N; 3] {
    [
        v[0] - t * direction[0],
        v[1] - t * direction[1],
        v[2] - t * direction[2],
    ]
}

/// `|v|`, with a bounded gradient at zero.
pub(crate) fn length<N: Number>(v: [N; 3]) -> N {
    v[0].hypot(v[1]).hypot(v[2])
}

/// `max(x, 0)`, joined smoothly: the outside part of an exact distance,
/// which is not a crease.
pub(crate) fn positive<N: Number>(x: N) -> N {
    x.select(N::constant(0.0), x)
}

/// `min(x, 0)`, joined smoothly: the inside part of an exact distance.
///
/// At zero it is `x`, so points on the surface keep its gradient. Above
/// zero it is `x * 0`, which keeps `x`'s creases: the edges of an exact
/// distance are marked from outside the solid too, where the inside part
/// no longer shapes the value.
pub(crate) fn negative<N: Number>(x: N) -> N {
    (-x).select(x * 0.0, x)
}

/// `clamp(x, 0, 1)`, joined smoothly.
pub(crate) fn unit<N: Number>(x: N) -> N {
    x.select(N::constant(0.0), (x - 1.0).select(x, N::constant(1.0)))
}

/// The exact distance to a box from its signed offsets `q` past each face.
/// Faces meet in creases.
pub(crate) fn box_distance<N: Number, const D: usize>(q: [N; D]) -> N {
    let outside = q
        .iter()
        .map(|&q| positive(q))
        .reduce(N::hypot)
        .unwrap_or(N::constant(0.0));
    let inside = q.into_iter().reduce(N::max).unwrap_or(N::constant(0.0));
    outside + negative(inside)
}

/// The lesser of each component.
pub(crate) fn min3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    core::array::from_fn(|i| a[i].min(b[i]))
}

/// The greater of each component.
pub(crate) fn max3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    core::array::from_fn(|i| a[i].max(b[i]))
}
