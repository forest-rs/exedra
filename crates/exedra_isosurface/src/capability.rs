// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What a field's values promise, independently of evaluating them.
//!
//! [`ScalarField`](crate::ScalarField) fixes only the sign convention:
//! negative inside, positive outside. Whether values are distances, and where
//! the inside can be, are separate claims that a field states through
//! [`ScalarField::capabilities`](crate::ScalarField::capabilities).
//! Combinators propagate only the claims their arithmetic justifies, and a
//! field that states nothing promises nothing.

use crate::Aabb2;
use crate::spatial::Aabb;

/// How a field's values relate to distance.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Metric {
    /// The magnitude of every value is the distance to the zero set.
    ExactDistance,
    /// Values change by at most this factor times the distance between
    /// points, so `|value| / L` bounds the distance to the zero set from
    /// below. The factor is finite and positive.
    Lipschitz(f32),
    /// No relation to distance is claimed; only the sign is meaningful.
    Unknown,
}

impl Metric {
    /// The Lipschitz factor this claim implies: one for an exact distance.
    #[must_use]
    pub fn lipschitz(self) -> Option<f32> {
        match self {
            Self::ExactDistance => Some(1.0),
            Self::Lipschitz(factor) => Some(factor),
            Self::Unknown => None,
        }
    }

    /// The claim for a pointwise minimum, maximum or convex blend of two
    /// fields: the larger factor, no longer an exact distance.
    #[must_use]
    pub fn combined(self, other: Self) -> Self {
        match (self.lipschitz(), other.lipschitz()) {
            (Some(a), Some(b)) => Self::Lipschitz(a.max(b)),
            _ => Self::Unknown,
        }
    }

    /// The claim after multiplying values by a finite positive `factor`.
    ///
    /// The product is rounded up, so the bound never falls below the true
    /// one, nor underflows to zero.
    #[must_use]
    pub fn scaled(self, factor: f32) -> Self {
        let Some(lipschitz) = self.lipschitz() else {
            return Self::Unknown;
        };
        // Two `f32` significands multiply exactly in `f64`.
        let product = f64::from(lipschitz) * f64::from(factor);
        #[expect(clippy::cast_possible_truncation, reason = "rounded up below")]
        let mut bound = product as f32;
        if f64::from(bound) < product {
            bound = bound.next_up();
        }
        if bound.is_finite() && bound > 0.0 {
            Self::Lipschitz(bound)
        } else {
            Self::Unknown
        }
    }
}

/// Where a field's interior, its negative region, can lie.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Support<B> {
    /// The interior is empty.
    Empty,
    /// The interior lies within this box, established from the field's own
    /// parameters with outward rounding.
    Bounded(B),
    /// The interior lies within this box by the provider's assertion, which
    /// nothing here checks.
    Asserted(B),
    /// The interior extends without bound, as for a half space. Operations
    /// that can remove interior, such as intersecting or shrinking, cannot
    /// keep this claim.
    Unbounded,
    /// Nothing is known.
    Unknown,
}

/// A field's metric and support claims.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Capabilities<B> {
    /// How values relate to distance.
    pub metric: Metric,
    /// Where the interior can lie.
    pub support: Support<B>,
}

/// Claims of a three-dimensional field.
pub type FieldCapabilities = Capabilities<Aabb>;

/// Claims of a two-dimensional profile field.
pub type ProfileCapabilities = Capabilities<Aabb2>;

impl<B> Capabilities<B> {
    /// No claims: the default for fields that state nothing.
    pub const UNKNOWN: Self = Self {
        metric: Metric::Unknown,
        support: Support::Unknown,
    };

    /// An exact distance whose interior lies within `bounds`.
    #[must_use]
    pub const fn exact(bounds: B) -> Self {
        Self {
            metric: Metric::ExactDistance,
            support: Support::Bounded(bounds),
        }
    }
}

impl Support<Aabb> {
    /// Support of the union of two interiors.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        use Support::{Asserted, Bounded, Empty, Unbounded, Unknown};
        match (self, other) {
            (Empty, other) | (other, Empty) => other,
            (Unknown, _) | (_, Unknown) => Unknown,
            (Unbounded, _) | (_, Unbounded) => Unbounded,
            (Bounded(a), Bounded(b)) => Bounded(a.merge(b)),
            (Bounded(a) | Asserted(a), Bounded(b) | Asserted(b)) => Asserted(a.merge(b)),
        }
    }

    /// Support of the intersection of two interiors: within either one.
    #[must_use]
    pub fn intersection(self, other: Self) -> Self {
        use Support::{Asserted, Bounded, Empty, Unbounded, Unknown};
        match (self, other) {
            (Empty, _) | (_, Empty) => Empty,
            (Bounded(a), Bounded(b)) => overlap(a, b).map_or(Empty, Bounded),
            (Bounded(a), _) | (_, Bounded(a)) => Bounded(a),
            (Asserted(a), Asserted(b)) => overlap(a, b).map_or(Empty, Asserted),
            (Asserted(a), _) | (_, Asserted(a)) => Asserted(a),
            // Two unbounded interiors may not even meet.
            (Unbounded | Unknown, Unbounded | Unknown) => Unknown,
        }
    }

    /// Support of an interior with `removed` taken away: within the original,
    /// and still unbounded only when what was removed is bounded.
    #[must_use]
    pub fn without(self, removed: Self) -> Self {
        use Support::{Asserted, Bounded, Empty, Unbounded, Unknown};
        match (self, removed) {
            (Unbounded, Empty | Bounded(_) | Asserted(_)) => Unbounded,
            (Unbounded, Unbounded | Unknown) => Unknown,
            (support, _) => support,
        }
    }

    /// Support after shrinking the interior: within the original, but an
    /// unbounded interior may vanish.
    #[must_use]
    pub fn shrunk(self) -> Self {
        match self {
            Self::Unbounded => Self::Unknown,
            support => support,
        }
    }

    /// Support grown by `margin` on every side.
    #[must_use]
    pub fn grown(self, margin: f32) -> Self {
        self.map(|b| outward(b.min.map(|v| v - margin), b.max.map(|v| v + margin)))
    }

    /// Support moved by `offset`.
    #[must_use]
    pub fn translated(self, offset: [f32; 3]) -> Self {
        self.map(|b| {
            outward(
                core::array::from_fn(|i| b.min[i] + offset[i]),
                core::array::from_fn(|i| b.max[i] + offset[i]),
            )
        })
    }

    /// Support scaled about the origin by a positive `factor`.
    #[must_use]
    pub fn scaled(self, factor: f32) -> Self {
        self.map(|b| outward(b.min.map(|v| v * factor), b.max.map(|v| v * factor)))
    }

    /// Support under `map`, which carries a box to a box containing its
    /// image; a box that cannot be formed leaves nothing known.
    pub(crate) fn map(self, map: impl Fn(Aabb) -> Option<Aabb>) -> Self {
        match self {
            Self::Bounded(b) => map(b).map_or(Self::Unknown, Self::Bounded),
            Self::Asserted(b) => map(b).map_or(Self::Unknown, Self::Asserted),
            other => other,
        }
    }
}

/// A box from corners rounded outward by one ulp, so arithmetic that formed
/// them cannot shrink it; `None` unless finite and ordered.
pub(crate) fn outward(min: [f32; 3], max: [f32; 3]) -> Option<Aabb> {
    Aabb::new(min.map(f32::next_down), max.map(f32::next_up))
        .filter(|b| b.min.iter().chain(&b.max).all(|v| v.is_finite()))
}

/// The 2D counterpart of [`outward`].
pub(crate) fn outward2(min: [f32; 2], max: [f32; 2]) -> Option<Aabb2> {
    Aabb2::new(min.map(f32::next_down), max.map(f32::next_up))
        .filter(|b| b.min.iter().chain(&b.max).all(|v| v.is_finite()))
}

fn overlap(a: Aabb, b: Aabb) -> Option<Aabb> {
    let min = core::array::from_fn(|i| a.min[i].max(b.min[i]));
    let max = core::array::from_fn(|i| a.max[i].min(b.max[i]));
    Aabb::new(min, max)
}

/// Exact-distance claims for a field bounded by `center` and `extent`, or
/// none when its parameters are not finite.
pub(crate) fn exact_about(center: [f32; 3], extent: [f32; 3]) -> FieldCapabilities {
    let min = core::array::from_fn(|i| center[i] - extent[i]);
    let max = core::array::from_fn(|i| center[i] + extent[i]);
    outward(min, max).map_or(Capabilities::UNKNOWN, Capabilities::exact)
}
