// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One checked boundary description on the unchanged original support.
use super::{Curve, CurveSpan, ParameterDomain, ParameterRange, Ruled, SourceBudget};
use exedra_curves::enclosure::{
    EnclosureScratch, PointEnclosure, PreparationLimits, PreparedNurbsEnclosure, SourceRange,
    line_enclosure, line_restriction_error,
};

pub(crate) struct RuledBoundaries {
    curves: [CurveSpan; 4],
    errors: [f64; 4],
    full_domain: bool,
}
impl RuledBoundaries {
    pub(crate) fn alignment(
        &self,
        side: usize,
        actual: &CurveSpan,
        work: &mut SourceBudget,
    ) -> f64 {
        let ideal = &self.curves[side];
        let alignment = if !self.full_domain
            && matches!(
                (ideal.curve(), actual.curve()),
                (Curve::Line(_), Curve::Line(_))
            ) {
            if ideal == actual {
                0.0
            } else {
                // Rounded evaluations can cancel large original coefficients
                // to matching endpoints while the exact native lines differ.
                // Both exact-f64 coefficient paths need outward enclosure.
                let endpoint = |span: &CurveSpan, t| {
                    let Curve::Line(line) = span.curve() else {
                        unreachable!()
                    };
                    line_enclosure(
                        line.origin(),
                        line.direction(),
                        SourceRange {
                            parameters: span.range().bounds(),
                            reversed: span.reversed(),
                        },
                        [t, t],
                    )
                };
                let mut bound = 0.0_f64;
                for t in [0., 1.] {
                    // One fixed-size endpoint comparison includes both 3D
                    // affine boxes and their outward distance norm.
                    if !work.enclosure() {
                        return f64::INFINITY;
                    }
                    let Ok(a) = endpoint(ideal, t) else {
                        return f64::INFINITY;
                    };
                    let Ok(b) = endpoint(actual, t) else {
                        return f64::INFINITY;
                    };
                    let Ok(distance) = a.distance_bounds_to(b) else {
                        return f64::INFINITY;
                    };
                    bound = bound.max(distance[1]);
                }
                bound
            }
        } else {
            ideal.alignment(actual)
        };
        if self.errors[side] == 0.0 {
            alignment
        } else {
            (alignment + self.errors[side]).next_up()
        }
    }
}
struct Source<'a> {
    original: &'a CurveSpan,
    nurbs: Option<PreparedNurbsEnclosure<'a>>,
}
impl<'a> Source<'a> {
    fn prepare(original: &'a CurveSpan, work: &mut SourceBudget) -> Option<Self> {
        let preparation = work.source(original)?;
        let nurbs = match original.curve() {
            Curve::Line(_) => None,
            Curve::Nurbs(n) => {
                let prepared = PreparedNurbsEnclosure::prepare(
                    n.curve().view(),
                    SourceRange {
                        parameters: original.range().bounds(),
                        reversed: original.reversed(),
                    },
                    PreparationLimits {
                        max_controls: original_control_count(original),
                        max_work: preparation,
                    },
                )
                .ok()?;
                Some(prepared)
            }
            _ => return None,
        };
        Some(Self { original, nurbs })
    }
    fn point(
        &self,
        u: f64,
        scratch: &mut EnclosureScratch,
        work: &mut SourceBudget,
    ) -> Option<PointEnclosure> {
        match &self.nurbs {
            Some(n) => n.enclose([u, u], scratch, || work.enclosure()).ok(),
            None => {
                let Curve::Line(line) = self.original.curve() else {
                    unreachable!()
                };
                work.enclosure().then_some(())?;
                line_enclosure(
                    line.origin(),
                    line.direction(),
                    SourceRange {
                        parameters: self.original.range().bounds(),
                        reversed: self.original.reversed(),
                    },
                    [u, u],
                )
                .ok()
            }
        }
    }
    fn rim_error(
        &self,
        domain: [f64; 2],
        child: &CurveSpan,
        work: &mut SourceBudget,
    ) -> Option<f64> {
        match &self.nurbs {
            Some(n) => n
                .restriction_error(
                    domain,
                    SourceRange {
                        parameters: child.range().bounds(),
                        reversed: child.reversed(),
                    },
                    || work.enclosure(),
                )
                .ok(),
            None => {
                let Curve::Line(line) = self.original.curve() else {
                    unreachable!()
                };
                work.enclosure().then_some(())?;
                line_restriction_error(
                    line.direction(),
                    SourceRange {
                        parameters: self.original.range().bounds(),
                        reversed: self.original.reversed(),
                    },
                    domain,
                    SourceRange {
                        parameters: child.range().bounds(),
                        reversed: child.reversed(),
                    },
                    || work.enclosure(),
                )
                .ok()
            }
        }
    }
}
fn original_control_count(source: &CurveSpan) -> usize {
    match source.curve() {
        Curve::Nurbs(n) => n.curve().points().len(),
        _ => 2,
    }
}
impl Ruled {
    /// Prepare both source enclosures once and reuse them across all four sides.
    /// Refusal means the original-source agreement was not proved, never that a
    /// loose caller model gap can admit unbounded parameter arithmetic.
    #[cfg(test)]
    pub(crate) fn domain_boundaries(&self, domain: ParameterDomain) -> Option<RuledBoundaries> {
        self.domain_boundaries_with_budget(domain, &mut SourceBudget::default())
    }
    pub(crate) fn domain_boundaries_with_budget(
        &self,
        domain: ParameterDomain,
        work: &mut SourceBudget,
    ) -> Option<RuledBoundaries> {
        if domain.u == ParameterRange::UNIT && domain.v == ParameterRange::UNIT {
            return Some(RuledBoundaries {
                curves: [
                    self.boundary(0)?,
                    self.boundary(1)?,
                    self.boundary(2)?,
                    self.boundary(3)?,
                ],
                errors: [0.; 4],
                full_domain: true,
            });
        }
        if domain.v != ParameterRange::UNIT {
            return None;
        }
        let sources = [
            Source::prepare(&self.curves[0], work)?,
            Source::prepare(&self.curves[1], work)?,
        ];
        let order = sources
            .iter()
            .filter_map(|s| s.nurbs.as_ref().map(PreparedNurbsEnclosure::order))
            .max()
            .unwrap_or(0);
        let mut scratch = EnclosureScratch::new(order).ok()?;
        let curves = [
            self.boundary_in(domain, 0)?,
            self.boundary_in(domain, 1)?,
            self.boundary_in(domain, 2)?,
            self.boundary_in(domain, 3)?,
        ];
        let mut errors = [0.; 4];
        errors[0] = sources[0].rim_error(domain.u.bounds(), &curves[0], work)?;
        errors[2] = sources[1].rim_error(domain.u.bounds(), &curves[2], work)?;
        for (side, u) in [(3, domain.u.bounds()[0]), (1, domain.u.bounds()[1])] {
            let Curve::Line(line) = curves[side].curve() else {
                unreachable!()
            };
            for (source, v) in sources.iter().zip([0., 1.]) {
                let exact_source = source.point(u, &mut scratch, work)?;
                // One fixed-size generated-Line endpoint box + outward
                // box-distance comparison, separate from source point work.
                work.enclosure().then_some(())?;
                let exact_line =
                    line_enclosure(line.origin(), line.direction(), SourceRange::UNIT, [v, v])
                        .ok()?;
                errors[side] =
                    errors[side].max(exact_source.distance_bounds_to(exact_line).ok()?[1]);
            }
        }
        Some(RuledBoundaries {
            curves,
            errors,
            full_domain: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_alignment_encloses_both_original_coefficient_paths() {
        let make = |y| {
            CurveSpan::new(
                Curve::line([0., y, 0.], [1., 0., 0.]).unwrap(),
                ParameterRange::UNIT,
                false,
            )
            .unwrap()
        };
        let ruled = Ruled::new(make(0.), make(16.)).unwrap();
        let domain = ParameterDomain {
            u: ParameterRange::new(0.2, 0.8).unwrap(),
            v: ParameterRange::UNIT,
        };
        let boundaries = ruled.domain_boundaries(domain).unwrap();
        let direction = 2.0_f64.powi(60) + 256.;
        let t = 0.1_f64;
        let actual = CurveSpan::new(
            Curve::line([0.8, -(direction * t), 0.], [0., direction, 0.]).unwrap(),
            ParameterRange::new(t, t.next_up()).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(
            boundaries.curves[1].alignment(&actual),
            0.,
            "rounded endpoint comparison alone misses cancellation"
        );
        assert!(boundaries.alignment(1, &actual, &mut SourceBudget::default()) > 1.);
        let mut no_work = SourceBudget::new(usize::MAX, 0);
        assert!(boundaries.alignment(1, &actual, &mut no_work).is_infinite());
        assert!(no_work.refusal.is_some());
    }
}
