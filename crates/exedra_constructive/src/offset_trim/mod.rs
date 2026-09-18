// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Local intersections and retained parameter cuts on fitted offset runs.

mod interval;
mod solve;

use super::{Corner, OffsetContext, OffsetTrim, Piece, PieceKind, wrap};
use crate::profile::{ProfileError, Seg2, SegKind};
use alloc::vec::Vec;
use kurbo::{CubicBez, ParamCurve, Point};
use solve::{Curve, System};

#[derive(Clone, Copy)]
pub(super) struct Cut {
    piece: usize,
    t: f64,
    bounds: [f64; 2],
    evidence: usize,
    tolerance: f64,
}

fn unwrapped(kind: &SegKind) -> &SegKind {
    match kind {
        SegKind::PolicyTo { realized, .. } => realized,
        _ => kind,
    }
}

fn curves(piece: &Piece) -> Vec<Curve> {
    match &piece.kind {
        PieceKind::Line => alloc::vec![Curve::line(piece.start, piece.end)],
        PieceKind::Arc { .. } => Vec::new(),
        PieceKind::Fitted(segments) => {
            let mut from = piece.start;
            segments
                .iter()
                .map(|s| {
                    let curve = match unwrapped(&s.kind) {
                        SegKind::Cubic { c1, c2 } => {
                            Curve::cubic(CubicBez::new(from, *c1, *c2, s.to))
                        }
                        SegKind::Line => Curve::line(from, s.to),
                        _ => unreachable!("fitter emits only polynomial segments"),
                    };
                    from = s.to;
                    curve
                })
                .collect()
        }
    }
}

// An isolated circle root may lie outside the finite authored arc. Reject roots
// too close to either endpoint to decide that membership at the proven resolution.
fn arc_parameter(
    piece: &Piece,
    point: Point,
    bound: f64,
    hole: Option<usize>,
    seg: usize,
) -> Result<Option<(f64, [f64; 2])>, ProfileError> {
    let PieceKind::Arc {
        center,
        radius,
        sweep,
        ..
    } = piece.kind
    else {
        unreachable!()
    };
    let margin = bound + 128.0 * f64::EPSILON * (radius + center.to_vec2().hypot());
    if point.distance(piece.start) <= margin || point.distance(piece.end) <= margin {
        return Err(ProfileError::OffsetTrimUnresolved { hole, seg });
    }
    let a = piece.start - center;
    let b = point - center;
    let mut angle = libm::atan2(a.cross(b), a.dot(b));
    if sweep > 0.0 && angle < 0.0 {
        angle += core::f64::consts::TAU;
    }
    if sweep < 0.0 && angle > 0.0 {
        angle -= core::f64::consts::TAU;
    }
    let t = angle / sweep;
    if !(t > 0.0 && t < 1.0) {
        return Ok(None);
    }
    let ratio = bound / b.hypot();
    if !ratio.is_finite() || ratio >= 0.5 {
        return Err(ProfileError::OffsetTrimUnresolved { hole, seg });
    }
    // Twice the angular radius of the positional enclosure, plus rounding
    // headroom for atan2 and division. This also guards finite-arc membership.
    let angular = 2.0 * libm::asin(ratio) + 128.0 * f64::EPSILON * (1.0 + angle.abs());
    let error = angular / sweep.abs();
    let bounds = [(t - error).next_down(), (t + error).next_up()];
    if bounds[0] <= 0.0 || bounds[1] >= 1.0 {
        return Err(ProfileError::OffsetTrimUnresolved { hole, seg });
    }
    Ok(Some((t, bounds)))
}

pub(super) fn corner(
    before: &Piece,
    after: &Piece,
    tolerance: f64,
    hole: Option<usize>,
    seg: usize,
    context: &mut OffsetContext,
) -> Result<Corner, ProfileError> {
    let first = curves(before);
    let second = curves(after);
    let mut candidate = None;
    let mut accept = |pieces: [usize; 2],
                      parameters: [f64; 2],
                      parameter_bounds: [[f64; 2]; 2],
                      point: Point,
                      bound: f64,
                      adjustment: f64|
     -> Result<(), ProfileError> {
        if candidate.is_some() {
            return Err(ProfileError::OffsetTrimAmbiguous { hole, seg });
        }
        if !adjustment.is_finite() || adjustment > tolerance {
            return Err(ProfileError::OffsetTrimUnresolved { hole, seg });
        }
        candidate = Some((
            pieces,
            parameters,
            parameter_bounds,
            point,
            bound,
            adjustment,
        ));
        Ok(())
    };
    if let PieceKind::Arc { center, radius, .. } = before.kind {
        for (j, &curve) in second.iter().enumerate() {
            for root in solve::roots(
                System::Circle(curve, center, radius),
                tolerance,
                context,
                hole,
                seg,
            )? {
                if let Some((t, bounds)) = arc_parameter(before, root.point, root.bound, hole, seg)?
                {
                    accept(
                        [0, j],
                        [t, root.parameters[0]],
                        [bounds, root.parameter_bounds()[0]],
                        root.point,
                        root.bound,
                        curve.point(root.parameters[0]).distance(root.point),
                    )?;
                }
            }
        }
    } else if let PieceKind::Arc { center, radius, .. } = after.kind {
        for (i, &curve) in first.iter().enumerate() {
            for root in solve::roots(
                System::Circle(curve, center, radius),
                tolerance,
                context,
                hole,
                seg,
            )? {
                if let Some((t, bounds)) = arc_parameter(after, root.point, root.bound, hole, seg)?
                {
                    accept(
                        [i, 0],
                        [root.parameters[0], t],
                        [root.parameter_bounds()[0], bounds],
                        root.point,
                        root.bound,
                        curve.point(root.parameters[0]).distance(root.point),
                    )?;
                }
            }
        }
    } else {
        for (i, &a) in first.iter().enumerate() {
            for (j, &b) in second.iter().enumerate() {
                for root in solve::roots(System::Curves(a, b), tolerance, context, hole, seg)? {
                    let adjustment = a
                        .point(root.parameters[0])
                        .distance(root.point)
                        .max(b.point(root.parameters[1]).distance(root.point));
                    accept(
                        [i, j],
                        root.parameters,
                        root.parameter_bounds(),
                        root.point,
                        root.bound,
                        adjustment,
                    )?;
                }
            }
        }
    }
    let (pieces, parameters, parameter_bounds, point, bound, adjustment) =
        candidate.ok_or(ProfileError::OffsetLoopDegenerate { hole })?;
    let evidence = context.trims.len();
    context.trims.push(OffsetTrim {
        hole,
        corner: crate::len_u32(seg),
        pieces: pieces.map(crate::len_u32),
        parameters,
        parameter_bounds,
        point: [point.x, point.y],
        position_bound: bound,
        endpoint_adjustment: adjustment,
    });
    Ok(Corner {
        end: point,
        start: point,
        trimmed: true,
        inserts: Vec::new(),
        cuts: Some(core::array::from_fn(|i| Cut {
            piece: pieces[i],
            t: parameters[i],
            bounds: parameter_bounds[i],
            evidence,
            tolerance,
        })),
    })
}

fn last_piece(piece: &Piece) -> usize {
    match &piece.kind {
        PieceKind::Fitted(fitted) => fitted.len() - 1,
        _ => 0,
    }
}
pub(super) fn check_cuts(
    piece: &Piece,
    start: Option<Cut>,
    end: Option<Cut>,
    hole: Option<usize>,
    seg: usize,
) -> Result<(), ProfileError> {
    let a = start.map_or((0, [0.0; 2]), |c| (c.piece, c.bounds));
    let b = end.map_or((last_piece(piece), [1.0; 2]), |c| (c.piece, c.bounds));
    if a.0 > b.0 || (a.0 == b.0 && a.1[0] >= b.1[1]) {
        return Err(ProfileError::OffsetLoopDegenerate { hole });
    }
    if a.0 == b.0 && a.1[1] >= b.1[0] {
        return Err(ProfileError::OffsetTrimUnresolved { hole, seg });
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "private run geometry, cuts and evidence context"
)]
pub(super) fn emit(
    piece: &Piece,
    segments: &[Seg2],
    start: Option<Cut>,
    end: Option<Cut>,
    new_start: Point,
    new_end: Point,
    hole: Option<usize>,
    context: &mut OffsetContext,
) -> Result<Vec<Seg2>, ProfileError> {
    if start.is_none() && end.is_none() {
        return Ok(segments.to_vec());
    }
    let first = start.map_or(0, |c| c.piece);
    let last = end.map_or(segments.len() - 1, |c| c.piece);
    let mut result = Vec::with_capacity(last - first + 1);
    for i in first..=last {
        let from = if i == 0 {
            piece.start
        } else {
            segments[i - 1].to
        };
        let segment = &segments[i];
        let t0 = if i == first {
            start.map_or(0.0, |c| c.t)
        } else {
            0.0
        };
        let t1 = if i == last {
            end.map_or(1.0, |c| c.t)
        } else {
            1.0
        };
        let (a, b, controls) = match unwrapped(&segment.kind) {
            SegKind::Line => (from.lerp(segment.to, t0), from.lerp(segment.to, t1), None),
            SegKind::Cubic { c1, c2 } => {
                let c = CubicBez::new(from, *c1, *c2, segment.to).subsegment(t0..t1);
                (c.p0, c.p3, Some((c.p1, c.p2)))
            }
            _ => unreachable!("fitted polynomial"),
        };
        let da = if i == first {
            new_start - a
        } else {
            kurbo::Vec2::ZERO
        };
        let db = if i == last {
            new_end - b
        } else {
            kurbo::Vec2::ZERO
        };
        let shifted = controls.map(|(c1, c2)| (c1 + da, c2 + db));
        let adjustments =
            controls
                .zip(shifted)
                .map_or([da.hypot(), db.hypot()], |((c1, c2), (a, b))| {
                    [
                        da.hypot().max(c1.distance(a)),
                        db.hypot().max(c2.distance(b)),
                    ]
                });
        for (cut, adjustment) in [
            (start.filter(|_| i == first), adjustments[0]),
            (end.filter(|_| i == last), adjustments[1]),
        ] {
            if let Some(cut) = cut {
                let evidence = &mut context.trims[cut.evidence];
                if !adjustment.is_finite() || adjustment > cut.tolerance {
                    return Err(ProfileError::OffsetTrimUnresolved {
                        hole,
                        seg: evidence.corner as usize,
                    });
                }
                evidence.endpoint_adjustment = evidence.endpoint_adjustment.max(adjustment);
            }
        }
        let kind = shifted.map_or(SegKind::Line, |(c1, c2)| SegKind::Cubic { c1, c2 });
        result.push(Seg2 {
            to: if i == last { new_end } else { b },
            kind: wrap(kind, piece.policy),
            tag: piece.tag,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
