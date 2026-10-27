// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Side-surface refinement after shared point-trajectory sampling.

use super::{LoftError, LoftSampling, LoftSamplingPolicy, LoftSpan, SampledLoft, sample};
use alloc::vec::Vec;
use exedra_math::{lerp, narrow, norm, sub, surface::bilinear_triangle_bounds};

struct Plan {
    source: LoftSpan,
    edges: u32,
    planar: f64,
    mixed: f64,
    rounding: f64,
}
impl Plan {
    fn bound(&self, step: f64) -> f64 {
        self.source.chord_bound + self.planar.max(self.mixed * step) + self.rounding
    }
}

/// Refines ruled walls and the chord surface of a sampled smooth loft.
/// Smooth point chords spend at most half the tolerance; linear subdivisions
/// retain that bound while reducing the remaining bilinear triangulation error.
pub(crate) fn sample_surface(
    sections: Vec<Vec<[f64; 3]>>,
    ring_sizes: &[usize],
    smooth: bool,
    policy: LoftSamplingPolicy,
) -> Result<SampledLoft, LoftError> {
    policy.validate()?;
    if !(2..=65536).contains(&sections.len())
        || sections[0].is_empty()
        || sections.iter().any(|s| s.len() != sections[0].len())
        || ring_sizes.iter().any(|&n| n < 2)
        || ring_sizes.iter().sum::<usize>() != sections[0].len()
    {
        return Err(LoftError::InvalidSections);
    }
    let mut coarse = if smooth {
        let chord_tolerance = policy.chord_tolerance * 0.5;
        if chord_tolerance == 0.0 {
            return Err(LoftError::NumericLimit);
        }
        sample(
            &sections,
            LoftSamplingPolicy {
                chord_tolerance,
                ..policy
            },
        )?
    } else {
        let count = u32::try_from(sections[0].len()).map_err(|_| LoftError::BudgetExceeded)?;
        if (sections.len() as u64) * u64::from(count) > u64::from(policy.max_vertices) {
            return Err(LoftError::BudgetExceeded);
        }
        let spans = (0..sections.len() - 1)
            .map(|i| LoftSpan {
                band: u16::try_from(i).expect("validated band count"),
                parameter: [0.0, 1.0],
                chord_bound: 0.0,
                surface_bound: 0.0,
            })
            .collect();
        SampledLoft {
            rings: sections,
            evidence: LoftSampling {
                policy,
                section_vertices: count,
                spans,
                max_vertex_displacement: 0.0,
            },
        }
    };
    let mut plans = Vec::with_capacity(coarse.evidence.spans.len());
    let mut rows = 1_u64;
    let mut band = None;
    let mut band_edges = 0_u32;
    for (i, source) in coarse.evidence.spans.iter().copied().enumerate() {
        let (a, b) = (&coarse.rings[i], &coarse.rings[i + 1]);
        let magnitude = a
            .iter()
            .chain(b)
            .flatten()
            .map(|v| v.abs())
            .fold(0.0_f64, f64::max);
        let rounding = magnitude * (128.0 * f64::EPSILON);
        let available = policy.chord_tolerance - source.chord_bound - rounding;
        if !available.is_finite() || available <= 0.0 {
            return Err(LoftError::NumericLimit);
        }
        let mut planar = 0.0_f64;
        let mut mixed = 0.0_f64;
        let mut start = 0;
        for &n in ring_sizes {
            for u in 0..n {
                let (j, k) = (start + u, start + (u + 1) % n);
                let bound = bilinear_triangle_bounds([a[j], a[k], b[k], b[j]])
                    .ok_or(LoftError::NumericLimit)?;
                if bound.planar_deviation <= available {
                    planar = planar.max(bound.planar_deviation);
                } else {
                    mixed = mixed.max(bound.mixed_deviation);
                }
            }
            start += n;
        }
        let required = libm::ceil(mixed / available).max(1.0);
        if !required.is_finite() || required > f64::from(policy.max_band_edges) {
            return Err(LoftError::BudgetExceeded);
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "positive count is bounded by the validated u16 policy limit"
        )]
        let mut edges = required as u32;
        // Two divisions can make neighboring normalized parameters one ulp
        // wider. Preserve the bound before allocating any output rings.
        while mixed * (1.0 / f64::from(edges) + 2.0 * f64::EPSILON) > available {
            edges += 1;
            if edges > policy.max_band_edges {
                return Err(LoftError::BudgetExceeded);
            }
        }
        if band != Some(source.band) {
            band = Some(source.band);
            band_edges = 0;
        }
        band_edges = band_edges
            .checked_add(edges)
            .ok_or(LoftError::BudgetExceeded)?;
        if band_edges > policy.max_band_edges {
            return Err(LoftError::BudgetExceeded);
        }
        rows = rows
            .checked_add(u64::from(edges))
            .ok_or(LoftError::BudgetExceeded)?;
        if rows * u64::from(coarse.evidence.section_vertices) > u64::from(policy.max_vertices) {
            return Err(LoftError::BudgetExceeded);
        }
        plans.push(Plan {
            source,
            edges,
            planar,
            mixed,
            rounding,
        });
    }
    if plans.iter().all(|p| p.edges == 1) {
        for (span, plan) in coarse.evidence.spans.iter_mut().zip(plans) {
            span.surface_bound = plan.bound(1.0);
        }
        coarse.evidence.policy = policy;
        coarse.evidence.max_vertex_displacement = emission_displacement(&coarse.rings)?;
        return Ok(coarse);
    }
    let count = coarse.evidence.section_vertices;
    let capacity = usize::try_from(rows).map_err(|_| LoftError::BudgetExceeded)?;
    let mut output = Vec::with_capacity(capacity);
    let mut spans = Vec::with_capacity(capacity - 1);
    let mut incoming = coarse.rings.into_iter();
    output.push(incoming.next().expect("nonempty checked sections"));
    for (next, plan) in incoming.zip(plans) {
        let first = output.len() - 1;
        let mut previous = 0.0;
        for step in 1..=plan.edges {
            let t = f64::from(step) / f64::from(plan.edges);
            if step < plan.edges {
                let row = output[first]
                    .iter()
                    .zip(&next)
                    .map(|(&a, &b)| lerp(a, b, t))
                    .collect();
                output.push(row);
            }
            let parameter = if step == plan.edges {
                plan.source.parameter[1]
            } else {
                plan.source.parameter[0] + (plan.source.parameter[1] - plan.source.parameter[0]) * t
            };
            let from = spans
                .last()
                .filter(|s: &&LoftSpan| s.band == plan.source.band)
                .map_or(plan.source.parameter[0], |s| s.parameter[1]);
            spans.push(LoftSpan {
                band: plan.source.band,
                parameter: [from, parameter],
                chord_bound: plan.source.chord_bound,
                surface_bound: plan.bound(t - previous),
            });
            previous = t;
        }
        output.push(next);
    }
    let displacement = emission_displacement(&output)?;
    Ok(SampledLoft {
        rings: output,
        evidence: LoftSampling {
            policy,
            section_vertices: count,
            spans,
            max_vertex_displacement: displacement,
        },
    })
}

fn emission_displacement(rings: &[Vec<[f64; 3]>]) -> Result<f64, LoftError> {
    let mut maximum = 0.0_f64;
    for &point in rings.iter().flatten() {
        let emitted = narrow(point);
        if emitted.iter().any(|v| !v.is_finite()) {
            return Err(LoftError::NumericLimit);
        }
        maximum = maximum.max(norm(sub(point, emitted.map(f64::from))));
    }
    Ok(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_math::{add, cross, dot, scale};

    fn triangle_distance(p: [f64; 3], q: [[f64; 3]; 3]) -> f64 {
        let (ab, ac, ap) = (sub(q[1], q[0]), sub(q[2], q[0]), sub(p, q[0]));
        let n = cross(ab, ac);
        let nn = dot(n, n);
        let projection = sub(ap, scale(n, dot(ap, n) / nn));
        let (aa, bb, cc) = (dot(ab, ab), dot(ab, ac), dot(ac, ac));
        let determinant = aa * cc - bb * bb;
        let u = (cc * dot(projection, ab) - bb * dot(projection, ac)) / determinant;
        let v = (aa * dot(projection, ac) - bb * dot(projection, ab)) / determinant;
        if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
            return dot(ap, n).abs() / libm::sqrt(nn);
        }
        [[0, 1], [1, 2], [2, 0]]
            .into_iter()
            .map(|[a, b]| {
                let edge = sub(q[b], q[a]);
                let t = (dot(sub(p, q[a]), edge) / dot(edge, edge)).clamp(0.0, 1.0);
                norm(sub(p, add(q[a], scale(edge, t))))
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn emitted_triangle_distance_covers_ruled_and_smooth_surface_twist() {
        let circle: Vec<_> = (0..8)
            .map(|i| {
                let t = f64::from(i) * core::f64::consts::FRAC_PI_4;
                [libm::cos(t), libm::sin(t), 0.0]
            })
            .collect();
        let square = alloc::vec![
            [1.0, 0.0, 2.0],
            [1.0, 1.0, 2.0],
            [0.0, 1.0, 2.0],
            [-1.0, 1.0, 2.0],
            [-1.0, 0.0, 2.0],
            [-1.0, -1.0, 2.0],
            [0.0, -1.0, 2.0],
            [1.0, -1.0, 2.0]
        ];
        for smooth in [false, true] {
            let sections = if smooth {
                let last = circle
                    .iter()
                    .map(|p| [0.8 * p[0], 0.8 * p[1], 4.0])
                    .collect();
                alloc::vec![circle.clone(), square.clone(), last]
            } else {
                alloc::vec![circle.clone(), square.clone()]
            };
            let sampled = sample_surface(
                sections.clone(),
                &[8],
                smooth,
                LoftSamplingPolicy::default(),
            )
            .unwrap();
            assert!(
                sampled.rings.len() > sections.len(),
                "twisted walls need longitudinal samples even for linear trajectories"
            );
            assert!(sampled.evidence.within_surface_tolerance());
            for (row, span) in sampled.evidence.spans.iter().enumerate() {
                let q = [
                    sampled.rings[row][0],
                    sampled.rings[row][1],
                    sampled.rings[row + 1][1],
                    sampled.rings[row + 1][0],
                ]
                .map(|p| narrow(p).map(f64::from));
                for u in [0.25, 0.5, 0.75] {
                    for t in [0.25, 0.5, 0.75] {
                        let v = span.parameter[0] + t * (span.parameter[1] - span.parameter[0]);
                        let band = usize::from(span.band);
                        let p = if smooth {
                            lerp(
                                super::super::point(super::super::controls(&sections, band, 0), v),
                                super::super::point(super::super::controls(&sections, band, 1), v),
                                u,
                            )
                        } else {
                            lerp(
                                lerp(sections[band][0], sections[band][1], u),
                                lerp(sections[band + 1][0], sections[band + 1][1], u),
                                v,
                            )
                        };
                        let error = triangle_distance(p, [q[0], q[1], q[2]])
                            .min(triangle_distance(p, [q[0], q[2], q[3]]));
                        assert!(
                            error <= span.surface_bound + sampled.evidence.max_vertex_displacement,
                            "surface distance {error} exceeds {span:?}"
                        );
                    }
                }
            }
        }
        assert!(matches!(
            sample_surface(
                alloc::vec![circle, square],
                &[8],
                false,
                LoftSamplingPolicy {
                    max_band_edges: 1,
                    ..LoftSamplingPolicy::default()
                }
            ),
            Err(LoftError::BudgetExceeded)
        ));
    }

    #[test]
    fn planar_tapers_keep_one_band_without_parameterization_overrefinement() {
        let a = alloc::vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0]
        ];
        let b = a.iter().map(|p| [2.0 * p[0], 2.0 * p[1], 2.0]).collect();
        let result = sample_surface(
            alloc::vec![a, b],
            &[4],
            false,
            LoftSamplingPolicy {
                max_band_edges: 1,
                ..LoftSamplingPolicy::default()
            },
        )
        .unwrap();
        assert_eq!(result.rings.len(), 2);
        assert!(result.evidence.within_surface_tolerance());
    }
}
