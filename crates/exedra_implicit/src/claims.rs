// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What a field's values promise, independently of evaluating them.
//!
//! A field fixes only the sign convention: negative inside, positive
//! outside. Whether values are distances, and where the inside can be, are
//! separate [`Claims`]. Built-in fields state them from their parameters,
//! combinators derive theirs from their parts' through the arithmetic they
//! apply, and a field that states nothing promises nothing.

use crate::bounds::{Aabb, Aabb2, outward};

/// How a field's values relate to distance.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Metric {
    /// The magnitude of every value is the distance to the zero set.
    ExactDistance,
    /// Values change by at most this factor times the distance between
    /// points, so `|value| / L` bounds the distance to the zero set from
    /// below. The factor is finite and positive.
    Lipschitz(f64),
    /// No relation to distance is claimed; only the sign is meaningful.
    Unknown,
}

impl Metric {
    /// The Lipschitz factor this claim implies: one for an exact distance.
    #[must_use]
    pub fn lipschitz(self) -> Option<f64> {
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

    /// The claim after multiplying values by a finite positive `factor`,
    /// rounded up so the bound never falls below the true one.
    #[must_use]
    pub fn scaled(self, factor: f64) -> Self {
        match self.lipschitz() {
            Some(lipschitz) => Self::lipschitz_up(lipschitz * factor),
            None => Self::Unknown,
        }
    }

    /// A Lipschitz claim of `factor` rounded up, or none when it is not
    /// finite and positive.
    pub(crate) fn lipschitz_up(factor: f64) -> Self {
        let factor = factor.next_up();
        if factor.is_finite() && factor > 0.0 {
            Self::Lipschitz(factor)
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

impl<B> Support<B> {
    /// The box the interior lies within, whether established or asserted.
    #[must_use]
    pub fn bounds(&self) -> Option<&B> {
        match self {
            Self::Bounded(bounds) | Self::Asserted(bounds) => Some(bounds),
            _ => None,
        }
    }

    /// Support under `map`, which carries a box to a box containing its
    /// image; a box that cannot be formed leaves nothing known.
    pub(crate) fn map<C>(self, map: impl Fn(B) -> Option<C>) -> Support<C> {
        match self {
            Self::Empty => Support::Empty,
            Self::Bounded(b) => map(b).map_or(Support::Unknown, Support::Bounded),
            Self::Asserted(b) => map(b).map_or(Support::Unknown, Support::Asserted),
            Self::Unbounded => Support::Unbounded,
            Self::Unknown => Support::Unknown,
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

    /// Support of an interior with `removed` taken away: within the original,
    /// and still unbounded only when what was removed is bounded.
    #[must_use]
    pub fn without(self, removed: &Self) -> Self {
        match (self, removed) {
            (Self::Unbounded, Self::Empty | Self::Bounded(_) | Self::Asserted(_)) => {
                Self::Unbounded
            }
            (Self::Unbounded, Self::Unbounded | Self::Unknown) => Self::Unknown,
            (support, _) => support,
        }
    }
}

/// How boxes merge and overlap, for supports of either dimension.
pub(crate) trait Region: Copy {
    fn merge(&self, other: &Self) -> Self;
    fn overlap(&self, other: &Self) -> Option<Self>;
}

impl Region for Aabb {
    fn merge(&self, other: &Self) -> Self {
        Self::merge(self, other)
    }

    fn overlap(&self, other: &Self) -> Option<Self> {
        Self::overlap(self, other)
    }
}

impl Region for Aabb2 {
    fn merge(&self, other: &Self) -> Self {
        Self::merge(self, other)
    }

    fn overlap(&self, other: &Self) -> Option<Self> {
        Self::overlap(self, other)
    }
}

#[expect(private_bounds, reason = "boxes of either dimension, nothing else")]
impl<B: Region> Support<B> {
    /// Support of the union of two interiors.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        use Support::{Asserted, Bounded, Empty, Unbounded, Unknown};
        match (self, other) {
            (Empty, other) | (other, Empty) => other,
            (Unknown, _) | (_, Unknown) => Unknown,
            (Unbounded, _) | (_, Unbounded) => Unbounded,
            (Bounded(a), Bounded(b)) => Bounded(a.merge(&b)),
            (Bounded(a) | Asserted(a), Bounded(b) | Asserted(b)) => Asserted(a.merge(&b)),
        }
    }

    /// Support of the intersection of two interiors: within either one.
    #[must_use]
    pub fn intersection(self, other: Self) -> Self {
        use Support::{Asserted, Bounded, Empty, Unbounded, Unknown};
        match (self, other) {
            (Empty, _) | (_, Empty) => Empty,
            (Bounded(a), Bounded(b)) => a.overlap(&b).map_or(Empty, Bounded),
            (Bounded(a), _) | (_, Bounded(a)) => Bounded(a),
            (Asserted(a), Asserted(b)) => a.overlap(&b).map_or(Empty, Asserted),
            (Asserted(a), _) | (_, Asserted(a)) => Asserted(a),
            // Two unbounded interiors may not even meet.
            (Unbounded | Unknown, Unbounded | Unknown) => Unknown,
        }
    }
}

impl Support<Aabb> {
    /// Support grown by `margin` on every side.
    #[must_use]
    pub fn grown(self, margin: f64) -> Self {
        self.map(|b| b.grown(margin))
    }
}

/// A field's metric and support claims.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Claims<B = Aabb> {
    /// How values relate to distance.
    pub metric: Metric,
    /// Where the interior can lie.
    pub support: Support<B>,
}

/// Claims of a two-dimensional profile.
pub type ProfileClaims = Claims<Aabb2>;

impl<B> Claims<B> {
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

/// Exact-distance claims for a field bounded by `center` and `extent`, or
/// none when its parameters are not finite.
pub(crate) fn exact_about(center: [f64; 3], extent: [f64; 3]) -> Claims {
    let min = core::array::from_fn(|i| center[i] - extent[i]);
    let max = core::array::from_fn(|i| center[i] + extent[i]);
    outward(min, max).map_or(Claims::UNKNOWN, Claims::exact)
}
