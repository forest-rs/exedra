// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    PlanarExtrusionError as Error, PlanarExtrusionPolicy, PlanarExtrusionReport, PlanarSegment,
};
use crate::geometry::{Curve, ParameterRange};
use alloc::vec::Vec;
use exedra_curves::{
    intersect::{BezierEnclosure, HullSeparation},
    profile::{Loop, LoopRef, Profile, ProfilePolicy, Segment},
};

pub(super) fn loop_ref(index: usize) -> LoopRef {
    if index == 0 {
        LoopRef::Outer
    } else {
        LoopRef::Hole(index - 1)
    }
}
#[derive(Clone)]
struct Piece {
    enclosure: BezierEnclosure,
    loop_index: usize,
    segment: usize,
}
impl Piece {
    fn error(&self, other: Option<&Self>) -> Error {
        Error::Unresolved {
            loop_ref: loop_ref(self.loop_index),
            segment: self.segment,
            other: other.map(|p| (loop_ref(p.loop_index), p.segment)),
        }
    }
}
fn source(segment: &PlanarSegment, li: usize, si: usize) -> Result<BezierEnclosure, Error> {
    let unsupported = || Error::UnsupportedSource {
        loop_ref: loop_ref(li),
        segment: si,
    };
    let span = &segment.span;
    let (mut points, mut weights) = match span.curve() {
        Curve::Line(line) => {
            if span.range() != ParameterRange::UNIT {
                return Err(unsupported());
            }
            let a = line.origin();
            let direction = line.direction();
            let b = core::array::from_fn(|k| a[k] + direction[k]);
            // The endpoint defining the proof must be the exact real sum of the
            // f64 line coefficients, rather than an accidentally rounded join.
            if (0..3).any(|k| sum_error(a[k], direction[k], b[k]) != 0.0) {
                return Err(unsupported());
            }
            (alloc::vec![a, b], None)
        }
        Curve::Nurbs(n) => {
            let c = n.curve();
            let order = c.order() as usize;
            if order > exedra_curves::intersect::MAX_ORDER {
                return Err(Error::UnsupportedOrder {
                    loop_ref: loop_ref(li),
                    segment: si,
                    order: c.order(),
                    maximum: u32::try_from(exedra_curves::intersect::MAX_ORDER)
                        .expect("small constant"),
                });
            }
            let [a, b] = c.domain();
            if c.points().len() != order
                || span.range().bounds() != [a, b]
                || c.knots()[..order].iter().any(|&t| t != a)
                || c.knots()[order..].iter().any(|&t| t != b)
            {
                return Err(unsupported());
            }
            (c.points().to_vec(), c.weights().map(<[f64]>::to_vec))
        }
        Curve::Circle(_) | Curve::Ellipse(_) => return Err(unsupported()),
    };
    let distance = points.iter().map(|p| p[2].abs()).fold(0.0, f64::max);
    if distance != 0.0 {
        return Err(Error::NonPlanar {
            loop_ref: loop_ref(li),
            segment: si,
            distance,
            maximum: 0.0,
        });
    }
    if span.reversed() {
        points.reverse();
        if let Some(weights) = &mut weights {
            weights.reverse();
        }
    }
    let points: Vec<_> = points.iter().map(|p| [p[0], p[1]]).collect();
    BezierEnclosure::new(&points, weights.as_deref()).map_err(|_| Error::Unresolved {
        loop_ref: loop_ref(li),
        segment: si,
        other: None,
    })
}
pub(super) fn sum_error(a: f64, b: f64, sum: f64) -> f64 {
    let partial = sum - a;
    (a - (sum - partial)) + (b - partial)
}

pub(super) fn check(
    loops: &[Vec<PlanarSegment>],
    policy: &PlanarExtrusionPolicy,
) -> Result<PlanarExtrusionReport, Error> {
    if !(policy.max_gap.is_finite()
        && policy.max_gap > 0.0
        && policy.chord_separation.is_finite()
        && policy.chord_separation > 0.0)
        || policy.max_pieces == 0
        || policy.max_pieces > (u32::MAX / 8) as usize
        || policy.max_pair_tests == 0
        || policy.max_subdivisions > 16
    {
        return Err(Error::InvalidPolicy);
    }
    let mut report = PlanarExtrusionReport::default();
    let mut pieces = Vec::new();
    for (li, loop_) in loops.iter().enumerate() {
        if loop_.len() < 2 {
            return Err(Error::EmptyLoop {
                loop_ref: loop_ref(li),
            });
        }
        report.source_segments = report.source_segments.saturating_add(loop_.len());
        if report.source_segments > policy.max_pieces {
            return Err(Error::Budget {
                resource: "source segments",
                maximum: policy.max_pieces,
            });
        }
        let base = pieces.len();
        for (si, segment) in loop_.iter().enumerate() {
            pieces.push(Piece {
                enclosure: source(segment, li, si)?,
                loop_index: li,
                segment: si,
            });
        }
        for si in 0..loop_.len() {
            let start = pieces[base + si].enclosure.endpoints()[0];
            let end = pieces[base + (si + loop_.len() - 1) % loop_.len()]
                .enclosure
                .endpoints()[1];
            if start != end {
                return Err(Error::OpenLoop {
                    loop_ref: loop_ref(li),
                    segment: si,
                    gap: libm::hypot(start[0] - end[0], start[1] - end[1]),
                });
            }
        }
    }
    for depth in 0..=policy.max_subdivisions {
        let mut unresolved = pieces
            .iter()
            .find(|p| !p.enclosure.is_monotone())
            .map(|p| p.error(None));
        if unresolved.is_none() {
            'pairs: for (i, a) in pieces.iter().enumerate() {
                for (offset, b) in pieces[i + 1..].iter().enumerate() {
                    if report.pair_tests == policy.max_pair_tests {
                        return Err(Error::Budget {
                            resource: "control-hull pairs",
                            maximum: policy.max_pair_tests,
                        });
                    }
                    report.pair_tests += 1;
                    let j = i + 1 + offset;
                    let same_loop = a.loop_index == b.loop_index;
                    let adjacent = same_loop
                        && (j == i + 1
                            || (pieces[..i].iter().all(|p| p.loop_index != a.loop_index)
                                && pieces[j + 1..].iter().all(|p| p.loop_index != a.loop_index)));
                    match a.enclosure.separation(&b.enclosure) {
                        Some(HullSeparation::Disjoint) => {}
                        Some(HullSeparation::SharedEndpoint) if adjacent => {}
                        _ => {
                            unresolved = Some(a.error(Some(b)));
                            break 'pairs;
                        }
                    }
                }
            }
        }
        if let Some(error) = unresolved {
            if depth == policy.max_subdivisions {
                return Err(error);
            }
            if pieces.len() > policy.max_pieces / 2 {
                return Err(Error::Budget {
                    resource: "outline pieces",
                    maximum: policy.max_pieces,
                });
            }
            let mut refined = Vec::with_capacity(pieces.len() * 2);
            for piece in pieces {
                let (left, right) = piece.enclosure.bisect().map_err(|_| piece.error(None))?;
                for enclosure in [left, right] {
                    refined.push(Piece {
                        enclosure,
                        loop_index: piece.loop_index,
                        segment: piece.segment,
                    });
                }
            }
            pieces = refined;
            continue;
        }
        let mut checked = Vec::new();
        for li in 0..loops.len() {
            let segments = pieces
                .iter()
                .filter(|p| p.loop_index == li)
                .map(|p| Segment::line(p.enclosure.endpoints()[1]))
                .collect();
            checked.push(Loop::new(segments).map_err(|_| Error::Unresolved {
                loop_ref: loop_ref(li),
                segment: 0,
                other: None,
            })?);
        }
        let outer = checked.remove(0);
        let mut profile_policy = ProfilePolicy::new(policy.chord_separation);
        profile_policy.max_segments = u32::try_from(policy.max_pieces).expect("policy checked");
        Profile::new(outer, checked, &profile_policy).map_err(Error::Profile)?;
        report.checked_pieces = pieces.len();
        report.subdivisions = depth;
        return Ok(report);
    }
    unreachable!("finite subdivision loop returns")
}
