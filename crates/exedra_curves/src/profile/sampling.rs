// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Edge counts for circular sweeps under a chord tolerance.

/// Edge-count constraints for one circular sweep.
///
/// Minimum topology, maximum work, and optional angular alignment are
/// independent choices. Set [`Self::edge_multiple`] to `1` when no alignment
/// is required, or for example `4` when a full circle must include cardinal
/// axes. The multiple is caller-selected and is never imposed on profile arcs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CircularEdgeConstraints {
    /// Minimum permitted edge count.
    pub min_edges: u32,
    /// Maximum permitted edge count (the finite work budget).
    pub max_edges: u32,
    /// Required edge-count multiple; `1` means no additional alignment.
    pub edge_multiple: u32,
}

impl CircularEdgeConstraints {
    /// Creates constraints without an additional edge-count multiple.
    #[must_use]
    pub const fn new(min_edges: u32, max_edges: u32) -> Self {
        Self {
            min_edges,
            max_edges,
            edge_multiple: 1,
        }
    }

    /// Requires the result to be a multiple of `edge_multiple`.
    ///
    /// A zero multiple is rejected by [`circular_edge_count`].
    #[must_use]
    pub const fn with_edge_multiple(mut self, edge_multiple: u32) -> Self {
        self.edge_multiple = edge_multiple;
        self
    }
}

/// Why no circular edge count satisfies the request.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CircularSamplingError {
    /// The chord tolerance is zero, negative, or non-finite.
    InvalidTolerance,
    /// An edge-count bound or multiple is zero, or the minimum exceeds the
    /// maximum.
    InvalidEdgeBounds,
    /// The radius or sweep is zero, negative, non-finite, or the sweep
    /// exceeds one full turn.
    InvalidCircularGeometry,
    /// The requested accuracy, topology, or alignment constraints need more
    /// edges than the finite work budget.
    ToleranceBudgetExceeded {
        /// Smallest edge count that satisfies accuracy, topology, and any
        /// requested edge-count multiple.
        required: u32,
        /// Maximum edge count permitted by the caller.
        maximum: u32,
    },
    /// The edge count required by the requested accuracy cannot fit in `u32`.
    EdgeCountOverflow,
}

impl core::fmt::Display for CircularSamplingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTolerance => write!(f, "chord tolerance must be finite and positive"),
            Self::InvalidEdgeBounds => {
                write!(
                    f,
                    "edge-count bounds must be nonzero with minimum <= maximum"
                )
            }
            Self::InvalidCircularGeometry => write!(
                f,
                "circle radius and sweep must be finite and positive, with sweep <= tau"
            ),
            Self::ToleranceBudgetExceeded { required, maximum } => write!(
                f,
                "subdivision constraints require {required} edges but the budget allows {maximum}"
            ),
            Self::EdgeCountOverflow => {
                write!(f, "required edge count exceeds the u32 count domain")
            }
        }
    }
}

impl core::error::Error for CircularSamplingError {}

/// Returns the smallest circular edge count that satisfies a chord tolerance
/// and explicit count constraints.
///
/// The count uses the cancellation-resistant identity
/// `acos(1 - x) = 2 asin(sqrt(x / 2))`. This remains stable when tolerance is
/// tiny relative to radius. A requested multiple rounds upward only; a work
/// budget never silently coarsens the result.
///
/// # Errors
///
/// Returns a typed error for invalid tolerance, circle geometry, constraints,
/// an unrepresentable required count, or a maximum below the required count.
pub fn circular_edge_count(
    radius: f64,
    sweep: f64,
    chord_tolerance: f64,
    constraints: CircularEdgeConstraints,
) -> Result<u32, CircularSamplingError> {
    if !(chord_tolerance.is_finite() && chord_tolerance > 0.0) {
        return Err(CircularSamplingError::InvalidTolerance);
    }
    if !(radius.is_finite()
        && radius > 0.0
        && sweep.is_finite()
        && sweep > 0.0
        && sweep <= core::f64::consts::TAU)
    {
        return Err(CircularSamplingError::InvalidCircularGeometry);
    }
    if constraints.min_edges == 0
        || constraints.max_edges == 0
        || constraints.edge_multiple == 0
        || constraints.min_edges > constraints.max_edges
    {
        return Err(CircularSamplingError::InvalidEdgeBounds);
    }

    // sqrt(tol / (2r)), factored so tol/r cannot underflow before sqrt.
    let ratio_root = libm::sqrt(chord_tolerance) / libm::sqrt(radius) / core::f64::consts::SQRT_2;
    if ratio_root == 0.0 {
        return Err(CircularSamplingError::EdgeCountOverflow);
    }
    let needed = if ratio_root >= 1.0 {
        1.0
    } else {
        let per_edge_angle = 4.0 * libm::asin(ratio_root);
        libm::ceil(sweep / per_edge_angle)
    };
    if !needed.is_finite() || needed > f64::from(u32::MAX) {
        return Err(CircularSamplingError::EdgeCountOverflow);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "needed is a finite nonnegative ceil within the u32 domain"
    )]
    let needed = needed as u32;
    let mut required = needed.max(1).max(constraints.min_edges);
    required = round_up_to_multiple(required, constraints.edge_multiple)
        .ok_or(CircularSamplingError::EdgeCountOverflow)?;

    // Guard a count that landed infinitesimally below the exact threshold due
    // to floating rounding. Incrementing by the requested multiple preserves
    // alignment and makes successful outcomes conservative.
    while !circular_sagitta_within(radius, sweep, chord_tolerance, required) {
        required = required
            .checked_add(constraints.edge_multiple)
            .ok_or(CircularSamplingError::EdgeCountOverflow)?;
    }
    if required > constraints.max_edges {
        return Err(CircularSamplingError::ToleranceBudgetExceeded {
            required,
            maximum: constraints.max_edges,
        });
    }
    Ok(required)
}

fn round_up_to_multiple(value: u32, multiple: u32) -> Option<u32> {
    let remainder = value % multiple;
    if remainder == 0 {
        Some(value)
    } else {
        value.checked_add(multiple - remainder)
    }
}

fn circular_sagitta_within(radius: f64, sweep: f64, tolerance: f64, edges: u32) -> bool {
    // sqrt(sagitta) = sqrt(2r) * sin(sweep / 4n). Compare after dividing by
    // sqrt(2r) to avoid forming either 2r or a tiny tolerance/radius ratio.
    let allowed_sine = libm::sqrt(tolerance) / libm::sqrt(radius) / core::f64::consts::SQRT_2;
    libm::sin(sweep / (4.0 * f64::from(edges))) <= allowed_sine
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circular_count_is_stable_monotone_and_caller_aligned() {
        let constraints = CircularEdgeConstraints::new(8, u32::MAX).with_edge_multiple(4);
        let representative = circular_edge_count(1.0, core::f64::consts::TAU, 5.0e-4, constraints)
            .expect("representative circle count");
        assert_eq!(representative, 100);
        assert_eq!(representative % 4, 0, "caller alignment is honored");

        let mut previous = 0;
        for tolerance in [1.0, 0.1, 1.0e-3, 1.0e-6, 1.0e-12] {
            let edges = circular_edge_count(2.0, core::f64::consts::TAU, tolerance, constraints)
                .expect("representable count");
            assert!(
                edges >= previous,
                "tightening tolerance reduced {previous} edges to {edges}"
            );
            previous = edges;
        }
    }

    #[test]
    fn circular_count_reports_budget_overflow_and_invalid_inputs() {
        assert!(matches!(
            circular_edge_count(
                1.0,
                core::f64::consts::PI,
                1.0e-9,
                CircularEdgeConstraints::new(1, 4),
            ),
            Err(CircularSamplingError::ToleranceBudgetExceeded { maximum: 4, .. })
        ));
        assert_eq!(
            circular_edge_count(
                1.0,
                core::f64::consts::TAU,
                1.0e-20,
                CircularEdgeConstraints::new(1, u32::MAX),
            ),
            Err(CircularSamplingError::EdgeCountOverflow)
        );
        for (radius, sweep, tolerance) in [
            (0.0, 1.0, 0.1),
            (1.0, 0.0, 0.1),
            (1.0, core::f64::consts::TAU + 0.1, 0.1),
            (f64::INFINITY, 1.0, 0.1),
        ] {
            assert_eq!(
                circular_edge_count(
                    radius,
                    sweep,
                    tolerance,
                    CircularEdgeConstraints::new(1, 100),
                ),
                Err(CircularSamplingError::InvalidCircularGeometry)
            );
        }
        assert_eq!(
            circular_edge_count(
                1.0,
                1.0,
                0.1,
                CircularEdgeConstraints::new(1, 100).with_edge_multiple(0),
            ),
            Err(CircularSamplingError::InvalidEdgeBounds)
        );
    }
}
