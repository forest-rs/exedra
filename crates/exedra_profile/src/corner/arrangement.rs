// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::arc::{Piece, cross, length, sub};
use crate::{Arc, Loop, LoopRef, ProfilePolicy, Segment};
use alloc::{vec, vec::Vec};

#[derive(Copy, Clone)]
struct Cut {
    incoming: [f64; 2],
    outgoing: [f64; 2],
    distance: f64,
    curve: Curve,
    tag: Option<SegTag>,
}

fn loops(profile: &Profile) -> impl Iterator<Item = (LoopRef, &Loop)> {
    core::iter::once((LoopRef::Outer, profile.outer())).chain(
        profile
            .holes()
            .iter()
            .enumerate()
            .map(|(i, l)| (LoopRef::Hole(i), l)),
    )
}
pub(super) fn piece(loop_: &Loop, index: usize) -> Piece {
    let segments = loop_.segments();
    let segment = segments[index];
    let from = segments[(index + segments.len() - 1) % segments.len()].to;
    match segment.curve {
        Curve::Line => Piece::Line(from, segment.to),
        curve => Piece::Arc(
            Arc::resolve(from, segment.to, curve)
                .expect("validated arc")
                .expect("non-line curve"),
        ),
    }
}
fn corner(loop_: &Loop, reference: SegmentRef) -> Corner {
    let incoming = piece(loop_, reference.segment).tangent(true);
    let outgoing = piece(loop_, (reference.segment + 1) % loop_.segments().len()).tangent(false);
    let sine = cross(incoming, outgoing);
    let cosine = incoming[0] * outgoing[0] + incoming[1] * outgoing[1];
    let turn = libm::atan2(sine, cosine);
    let kind = if sine.abs() <= 1e-12 && cosine > 0.0 {
        CornerKind::Smooth
    } else if turn > 0.0 {
        CornerKind::Convex
    } else {
        CornerKind::Concave
    };
    Corner {
        segment: reference,
        point: loop_.segments()[reference.segment].to,
        kind,
        turn,
    }
}

impl Profile {
    /// Identifies exactly one boundary vertex within `max_distance` of `point`.
    ///
    /// The distance is inclusive, in profile units; zero requires an exact
    /// coordinate match. Outer and hole boundaries participate equally. This
    /// is an intentional selection, not a nearest-point or snapping operation.
    /// Use the returned [`Corner::segment`] with [`Self::with_corners`], and
    /// select again after editing the profile. Smooth and arc junctions remain
    /// inspectable even when a particular treatment cannot handle them.
    ///
    /// # Errors
    ///
    /// Returns invalid-input, missing or ambiguous selection evidence. A large
    /// search distance never silently chooses one of several matching vertices.
    pub fn corner_at(
        &self,
        point: [f64; 2],
        max_distance: f64,
    ) -> Result<Corner, CornerSelectionError> {
        if !point.iter().all(|v| v.is_finite()) || !max_distance.is_finite() || max_distance < 0.0 {
            return Err(CornerSelectionError::Input {
                point,
                max_distance,
            });
        }
        let mut found: Option<Corner> = None;
        for (loop_ref, boundary) in loops(self) {
            for (segment, candidate) in boundary.segments().iter().enumerate() {
                let distance = libm::hypot(candidate.to[0] - point[0], candidate.to[1] - point[1]);
                if distance <= max_distance {
                    let reference = SegmentRef { loop_ref, segment };
                    if let Some(first) = found {
                        return Err(CornerSelectionError::Ambiguous {
                            point,
                            max_distance,
                            matches: [first.segment, reference],
                        });
                    }
                    // Tangents/classification are needed only for the matching
                    // vertex, not for every vertex rejected by position.
                    found = Some(corner(boundary, reference));
                }
            }
        }
        found.ok_or(CornerSelectionError::Missing {
            point,
            max_distance,
        })
    }

    /// Boundary vertices classified relative to the filled material.
    /// References are local to this immutable profile; select again after an edit.
    pub fn corners(&self) -> impl Iterator<Item = Corner> + '_ {
        loops(self).flat_map(|(loop_ref, l)| {
            (0..l.segments().len()).map(move |segment| corner(l, SegmentRef { loop_ref, segment }))
        })
    }

    /// Treats selected line-line corners with tangent fillets or equal-distance
    /// chamfers. Select by convexity, coordinates or existing segment identity.
    /// Retained segments keep their tags; generated transitions use `treatment`'s
    /// tag, or the common adjacent tag. Unselected arcs are preserved exactly.
    /// The input, trims and final arrangement are validated under `policy`.
    ///
    /// # Errors
    ///
    /// Returns typed dimension, reference, unsupported-junction, trim or final
    /// clearance refusals. A selected junction involving an arc is refused
    /// rather than approximated.
    pub fn with_corners(
        &self,
        selected: impl IntoIterator<Item = SegmentRef>,
        treatment: CornerTreatment,
        policy: &ProfilePolicy,
    ) -> Result<CornerResult, CornerError> {
        self.validate(policy).map_err(CornerError::Input)?;
        let size = match treatment.shape {
            CornerShape::Fillet { radius } => radius,
            CornerShape::Chamfer { distance } => distance,
        };
        if !(size.is_finite() && size > 0.0) {
            return Err(CornerError::InvalidSize {
                shape: treatment.shape,
            });
        }
        let loops: Vec<_> = loops(self).collect();
        let mut cuts: Vec<Vec<Option<Cut>>> = loops
            .iter()
            .map(|(_, l)| vec![None; l.segments().len()])
            .collect();
        let mut changed = 0;
        for reference in selected {
            let index = match reference.loop_ref {
                LoopRef::Outer => 0,
                LoopRef::Hole(i) => i
                    .checked_add(1)
                    .ok_or(CornerError::UnknownCorner { corner: reference })?,
            };
            let Some(&(_, loop_)) = loops.get(index) else {
                return Err(CornerError::UnknownCorner { corner: reference });
            };
            let segments = loop_.segments();
            if reference.segment >= segments.len() {
                return Err(CornerError::UnknownCorner { corner: reference });
            }
            if cuts[index][reference.segment].is_some() {
                return Err(CornerError::RepeatedCorner { corner: reference });
            }
            let next = (reference.segment + 1) % segments.len();
            let descriptor = corner(loop_, reference);
            if descriptor.kind == CornerKind::Smooth {
                return Err(CornerError::SmoothCorner { corner: reference });
            }
            if segments[reference.segment].curve != Curve::Line
                || segments[next].curve != Curve::Line
            {
                return Err(CornerError::UnsupportedCorner {
                    corner: reference,
                    incoming: segments[reference.segment].curve,
                    outgoing: segments[next].curve,
                });
            }
            let incoming = piece(loop_, reference.segment).tangent(true);
            let outgoing = piece(loop_, next).tangent(false);
            let (distance, curve) = match treatment.shape {
                CornerShape::Fillet { radius } => (
                    radius * libm::tan(0.5 * descriptor.turn.abs()),
                    Curve::Arc {
                        bulge: libm::tan(0.25 * descriptor.turn),
                    },
                ),
                CornerShape::Chamfer { distance } => (distance, Curve::Line),
            };
            let point = descriptor.point;
            cuts[index][reference.segment] = Some(Cut {
                incoming: [
                    point[0] - incoming[0] * distance,
                    point[1] - incoming[1] * distance,
                ],
                outgoing: [
                    point[0] + outgoing[0] * distance,
                    point[1] + outgoing[1] * distance,
                ],
                distance,
                curve,
                tag: treatment.tag.or_else(|| {
                    (segments[reference.segment].tag == segments[next].tag)
                        .then_some(segments[next].tag)
                        .flatten()
                }),
            });
            changed += 1;
        }
        let mut output = Vec::with_capacity(loops.len());
        let mut correspondence = Vec::new();
        for (index, &(loop_ref, loop_)) in loops.iter().enumerate() {
            let segments = loop_.segments();
            let mut result = Vec::with_capacity(
                segments.len() + cuts[index].iter().filter(|c| c.is_some()).count(),
            );
            for (i, &segment) in segments.iter().enumerate() {
                let previous = (i + segments.len() - 1) % segments.len();
                let start = cuts[index][previous];
                let end = cuts[index][i];
                let span = length(sub(segment.to, segments[previous].to));
                let (head, tail) = (
                    start.map_or(0.0, |c| c.distance),
                    end.map_or(0.0, |c| c.distance),
                );
                if head + tail > 0.0
                    && (!(head + tail).is_finite() || span - head - tail < policy.min_separation)
                {
                    return Err(CornerError::OverlappingTrims {
                        segment: SegmentRef {
                            loop_ref,
                            segment: i,
                        },
                        length: span,
                        requested: head + tail,
                        min_separation: policy.min_separation,
                    });
                }
                let to = end.map_or(segment.to, |c| c.incoming);
                correspondence.push(CornerSegment {
                    segment: SegmentRef {
                        loop_ref,
                        segment: result.len(),
                    },
                    source: CornerSource::Segment {
                        segment: SegmentRef {
                            loop_ref,
                            segment: i,
                        },
                        parameters: [head / span, 1.0 - tail / span],
                    },
                });
                result.push(Segment { to, ..segment });
                if let Some(cut) = end {
                    correspondence.push(CornerSegment {
                        segment: SegmentRef {
                            loop_ref,
                            segment: result.len(),
                        },
                        source: CornerSource::Transition {
                            corner: SegmentRef {
                                loop_ref,
                                segment: i,
                            },
                        },
                    });
                    result.push(Segment {
                        to: cut.outgoing,
                        curve: cut.curve,
                        tag: cut.tag,
                    });
                }
            }
            output.push(Loop::new(result).map_err(CornerError::Loop)?);
        }
        let mut iter = output.into_iter();
        let outer = iter.next().expect("profile has outer loop");
        let profile = Self::new(outer, iter.collect(), policy).map_err(CornerError::Output)?;
        Ok(CornerResult {
            profile,
            segments: correspondence,
            changed_corners: changed,
        })
    }
}
