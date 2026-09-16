// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Uniform cubic loft trajectories, bounded sampling, and local evidence.
//!
//! Correspondence is authored loop/segment order. This module never aligns
//! sections geometrically. Smooth interpolation is C1 in section-index space,
//! with centered interior tangents and one-sided endpoint secants.

use alloc::vec::Vec;
use exedra_math::{add, dot, lerp, norm, scale, sub};

/// Accuracy and finite work limits for smooth loft trajectories.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LoftSamplingPolicy {
    /// Maximum trajectory chord deviation in placed recipe units, before f32
    /// storage. Does not bound profile discretization or mesh quantization.
    pub chord_tolerance: f64,
    /// Maximum subdivisions of one authored section band, in `1..=65535`.
    pub max_band_edges: u32,
    /// Maximum sampled vertices, including shared section rings.
    pub max_vertices: u32,
}

impl Default for LoftSamplingPolicy {
    fn default() -> Self {
        Self {
            chord_tolerance: 0.01,
            max_band_edges: 4096,
            max_vertices: 1_000_000,
        }
    }
}

impl LoftSamplingPolicy {
    /// Checks accuracy and work limits before traversing geometry.
    pub fn validate(&self) -> Result<(), LoftError> {
        if !self.chord_tolerance.is_finite()
            || self.chord_tolerance <= 0.0
            || !(1..=65535).contains(&self.max_band_edges)
            || self.max_vertices == 0
        {
            return Err(LoftError::InvalidPolicy);
        }
        Ok(())
    }
}

/// A shared sampling interval for every point trajectory in one loft band.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LoftSpan {
    /// Index of the authored section at the beginning of this band.
    pub band: u16,
    /// Inclusive parameter interval in `[0, 1]` within the authored band.
    pub parameter: [f64; 2],
    /// Conservative maximum point-trajectory chord error, with f64 headroom.
    pub chord_bound: f64,
}

/// Original sampling evidence and local realization checks for a smooth loft.
///
/// Every trajectory has positive derivative projection on its band's secant.
/// Both diagonals of emitted wall quads and emitted cap triangles retain their
/// f64 orientation after f32 narrowing. This does not check distant surface
/// intersections or certify a solid. Mutating the mesh invalidates these checks.
/// Instances retain this as source evidence: subsequent placement can change
/// distances, winding, and realization accuracy without changing these bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct LoftSampling {
    /// Policy used for the original construction.
    pub policy: LoftSamplingPolicy,
    /// Number of corresponding profile points per sampled ring, including holes.
    pub section_vertices: u32,
    /// Shared sample intervals in output ring order.
    pub spans: Vec<LoftSpan>,
}

/// A smooth loft could not honor its interpolation or sampling contract.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoftError {
    /// Tolerance must be positive and finite, and budgets must be nonzero.
    InvalidPolicy,
    /// Smooth lofts require 2 through 65536 corresponding sections.
    InvalidSections,
    /// Required sampling exceeds a per-band or total vertex work budget.
    BudgetExceeded,
    /// Finite construction, positive span, or requested accuracy is unrepresentable.
    NumericLimit,
    /// A cubic trajectory cannot establish positive forward motion in its band.
    Foldover {
        /// Authored section band.
        band: usize,
        /// Flattened corresponding profile point.
        vertex: usize,
    },
}

impl core::fmt::Display for LoftError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPolicy => f.write_str("invalid smooth-loft sampling policy"),
            Self::InvalidSections => {
                f.write_str("smooth loft needs 2 through 65536 corresponding sections")
            }
            Self::BudgetExceeded => f.write_str("smooth-loft sampling budget exceeded"),
            Self::NumericLimit => f.write_str("smooth-loft construction exceeds numeric limits"),
            Self::Foldover { band, vertex } => write!(
                f,
                "smooth-loft band {band} point {vertex} cannot establish forward motion"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for LoftError {}

pub(crate) struct SampledLoft {
    pub rings: Vec<Vec<[f64; 3]>>,
    pub evidence: LoftSampling,
}

fn controls(sections: &[Vec<[f64; 3]>], band: usize, vertex: usize) -> [[f64; 3]; 4] {
    let tangent = |i: usize| {
        if i == 0 {
            sub(sections[1][vertex], sections[0][vertex])
        } else if i + 1 == sections.len() {
            sub(sections[i][vertex], sections[i - 1][vertex])
        } else {
            scale(sub(sections[i + 1][vertex], sections[i - 1][vertex]), 0.5)
        }
    };
    let a = sections[band][vertex];
    let b = sections[band + 1][vertex];
    [
        a,
        add(a, scale(tangent(band), 1.0 / 3.0)),
        sub(b, scale(tangent(band + 1), 1.0 / 3.0)),
        b,
    ]
}

fn point(c: [[f64; 3]; 4], t: f64) -> [f64; 3] {
    let a = lerp(c[0], c[1], t);
    let b = lerp(c[1], c[2], t);
    let d = lerp(c[2], c[3], t);
    lerp(lerp(a, b, t), lerp(b, d, t), t)
}

pub(crate) fn sample(
    sections: &[Vec<[f64; 3]>],
    policy: LoftSamplingPolicy,
) -> Result<SampledLoft, LoftError> {
    policy.validate()?;
    if !(2..=65536).contains(&sections.len())
        || sections[0].is_empty()
        || sections.iter().any(|s| s.len() != sections[0].len())
    {
        return Err(LoftError::InvalidSections);
    }
    let count = u32::try_from(sections[0].len()).map_err(|_| LoftError::BudgetExceeded)?;
    if count > policy.max_vertices {
        return Err(LoftError::BudgetExceeded);
    }
    let mut rings = alloc::vec![sections[0].clone()];
    let mut spans = Vec::new();
    for band in 0..sections.len() - 1 {
        let mut curves = Vec::with_capacity(count as usize);
        let mut error = 0.0_f64;
        let mut magnitude = 0.0_f64;
        for vertex in 0..count as usize {
            let c = controls(sections, band, vertex);
            if c.iter().flatten().any(|x| !x.is_finite()) {
                return Err(LoftError::NumericLimit);
            }
            let secant = sub(c[3], c[0]);
            for pair in c.windows(2) {
                let advance = dot(sub(pair[1], pair[0]), secant);
                if !advance.is_finite() {
                    return Err(LoftError::NumericLimit);
                }
                if advance <= 0.0 {
                    return Err(LoftError::Foldover { band, vertex });
                }
            }
            for triple in c.windows(3) {
                // max |P''| / 8 bounds deviation from a uniform chord.
                let second = sub(sub(triple[2], triple[1]), sub(triple[1], triple[0]));
                error = error.max(0.75 * norm(second));
            }
            for x in c.iter().flatten() {
                magnitude = magnitude.max(x.abs());
            }
            curves.push(c);
        }
        let rounding = magnitude * (128.0 * f64::EPSILON);
        let available = policy.chord_tolerance - rounding;
        if !error.is_finite() || available <= 0.0 {
            return Err(LoftError::NumericLimit);
        }
        let required = libm::ceil(libm::sqrt(error / available)).max(1.0);
        if required > f64::from(policy.max_band_edges) {
            return Err(LoftError::BudgetExceeded);
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "positive count bounded by the u16 policy limit"
        )]
        let mut edges = required as u32;
        // Preserve the bound even if the square root rounded down at an integer.
        while error / (f64::from(edges) * f64::from(edges)) + rounding > policy.chord_tolerance {
            edges += 1;
            if edges > policy.max_band_edges {
                return Err(LoftError::BudgetExceeded);
            }
        }
        let vertices = (rings.len() as u64 + u64::from(edges)) * u64::from(count);
        if vertices > u64::from(policy.max_vertices) {
            return Err(LoftError::BudgetExceeded);
        }
        let bound = error / (f64::from(edges) * f64::from(edges)) + rounding;
        for step in 1..=edges {
            let t = f64::from(step) / f64::from(edges);
            let ring = if step == edges {
                sections[band + 1].clone()
            } else {
                curves.iter().map(|&c| point(c, t)).collect()
            };
            rings.push(ring);
            spans.push(LoftSpan {
                band: u16::try_from(band).expect("validated section count"),
                parameter: [f64::from(step - 1) / f64::from(edges), t],
                chord_bound: bound,
            });
        }
    }
    Ok(SampledLoft {
        rings,
        evidence: LoftSampling {
            policy,
            section_vertices: count,
            spans,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn trajectories_interpolate_sections_with_shared_derivatives_and_bounded_chords() {
        let sections = vec![
            vec![[0.0, 0.0, 0.0]],
            vec![[0.5, 0.2, 1.0]],
            vec![[-0.1, 0.4, 2.0]],
        ];
        let left = controls(&sections, 0, 0);
        let right = controls(&sections, 1, 0);
        assert_eq!(left[3], right[0]);
        assert!(
            norm(sub(
                scale(sub(left[3], left[2]), 3.0),
                scale(sub(right[1], right[0]), 3.0)
            )) < 1e-14
        );
        let policy = LoftSamplingPolicy {
            chord_tolerance: 0.001,
            ..LoftSamplingPolicy::default()
        };
        let sampled = sample(&sections, policy).unwrap();
        assert!(sampled.rings.contains(&sections[1]));
        assert_eq!(sampled.rings[0], sections[0]);
        assert_eq!(sampled.rings.last().unwrap(), &sections[2]);
        for (i, span) in sampled.evidence.spans.iter().enumerate() {
            let c = controls(&sections, usize::from(span.band), 0);
            for step in 0..=16 {
                let t = f64::from(step) / 16.0;
                let parameter = span.parameter[0] + t * (span.parameter[1] - span.parameter[0]);
                let chord = lerp(sampled.rings[i][0], sampled.rings[i + 1][0], t);
                assert!(norm(sub(point(c, parameter), chord)) <= span.chord_bound);
                assert!(span.chord_bound <= policy.chord_tolerance);
            }
        }
        let tighter = sample(
            &sections,
            LoftSamplingPolicy {
                chord_tolerance: 0.0001,
                ..policy
            },
        )
        .unwrap();
        assert!(tighter.rings.len() > sampled.rings.len());
    }

    #[test]
    fn budgets_numeric_limits_and_backward_trajectories_are_explicit() {
        let sections = vec![
            vec![[0.0, 0.0, 0.0]],
            vec![[0.5, 0.0, 1.0]],
            vec![[0.0, 0.0, 2.0]],
        ];
        for policy in [
            LoftSamplingPolicy {
                max_band_edges: 1,
                chord_tolerance: 1e-8,
                ..LoftSamplingPolicy::default()
            },
            LoftSamplingPolicy {
                max_vertices: 2,
                ..LoftSamplingPolicy::default()
            },
        ] {
            assert!(matches!(
                sample(&sections, policy),
                Err(LoftError::BudgetExceeded)
            ));
        }
        assert!(matches!(
            sample(
                &sections,
                LoftSamplingPolicy {
                    chord_tolerance: f64::NAN,
                    ..LoftSamplingPolicy::default()
                }
            ),
            Err(LoftError::InvalidPolicy)
        ));
        assert!(matches!(
            sample(
                &sections,
                LoftSamplingPolicy {
                    chord_tolerance: 1e-20,
                    ..LoftSamplingPolicy::default()
                }
            ),
            Err(LoftError::NumericLimit)
        ));
        let backward = vec![
            vec![[0.0, 0.0, 0.0]],
            vec![[0.0, 0.0, 1.0]],
            vec![[0.0, 0.0, 0.0]],
        ];
        assert!(matches!(
            sample(&backward, LoftSamplingPolicy::default()),
            Err(LoftError::Foldover { .. })
        ));
    }
}
