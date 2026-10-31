// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rotation-minimizing frames along a sampled curve.

use alloc::vec::Vec;

use exedra_math::{add, cross, dot, normalize, scale, sub};

use crate::Workspace;

/// An orthonormal frame at one station of a sampled curve: `normal x
/// binormal = tangent` (right-handed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The station's point.
    pub point: [f64; 3],
    /// The unit tangent.
    pub tangent: [f64; 3],
    /// The unit normal, the first section axis.
    pub normal: [f64; 3],
    /// The unit binormal, `tangent x normal`.
    pub binormal: [f64; 3],
}

/// What [`rotation_minimizing_frames`] met along the way.
///
/// Non-exhaustive, so it can report more later without breaking a caller
/// that reads its fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FrameReport {
    /// Stations framed.
    pub stations: u32,
    /// Stations repeating the point before them: an estimated tangent is
    /// that of the nearest distinct neighbours, and the frame does not
    /// move.
    pub repeated: u32,
    /// Stations whose tangent was estimated where the path doubles back
    /// (the incoming and outgoing directions cancel): they take the
    /// incoming direction.
    pub cusps: u32,
    /// The first such station, if any, so a caller that refuses cusps can
    /// say where.
    pub first_cusp: Option<u32>,
    /// Stations whose supplied tangent was zero or not finite: their
    /// tangent was estimated from the chords instead. 0 without supplied
    /// tangents.
    pub estimated: u32,
    /// For a closed curve, the cosine of the seam's twist: the signed
    /// angle, right-handed about the first tangent, from the first normal
    /// to the normal transported once around the loop back to the first
    /// station. A closed sweep's last section joins its first across that
    /// angle. 1 when the loop closes without twist (any planar loop), and
    /// for an open curve.
    pub seam_cos: f64,
    /// The sine of the seam's twist ([`Self::seam_cos`]): its sign is the
    /// twist's direction, so a caller can distribute it along the loop
    /// without trigonometry here. 0 for an open curve.
    pub seam_sin: f64,
}

/// Supplied tangents are not one per point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TangentCount {
    /// The points.
    pub points: usize,
    /// The tangents given.
    pub tangents: usize,
}

impl core::fmt::Display for TangentCount {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} tangents for {} points", self.tangents, self.points)
    }
}

impl core::error::Error for TangentCount {}

/// Frames along `points`, transported without spin by
/// [`crate::path::double_reflection`] (Wang, Juttler, Zheng and Liu,
/// "Computation of Rotation Minimizing Frames", ACM Transactions on
/// Graphics 27(1), 2008, section 4).
///
/// - **Tangents.** A station's tangent is the bisector of the unit
///   directions to the previous and next distinct points; an open curve's
///   ends take their one direction, a closed curve's wrap around. Where
///   the two directions reverse to within `2^-26` radians (a cusp) the
///   incoming direction is used. A curve whose points all coincide is
///   given tangent +Z. The neighbours across runs of repeated points are
///   found in one pass each way, in linear time.
/// - **Seed.** The first normal is `seed` made perpendicular to the first
///   tangent (Gram-Schmidt) when given and more than `2^-26` radians from
///   parallel to it; otherwise the world axis least aligned with the
///   tangent (ties x, then y, then z), made perpendicular. No
///   trigonometry, so equal inputs give equal bits.
/// - **Closed curves** are not twisted to close: the report gives the
///   seam's twist ([`FrameReport::seam_cos`], [`FrameReport::seam_sin`]).
/// - **Non-finite input** is not an error. Framing is evaluation, run on
///   points a caller has already sampled, so it does not scan them: a
///   non-finite point gives its station a non-finite point, and the
///   tangents beside it are estimated across it as across a repeated
///   point. A zero or non-finite seed takes the fallback axis, and a zero
///   or non-finite supplied tangent is estimated. Every frame's axes stay
///   orthonormal.
///
/// Chord bisectors are only an estimate of a smooth curve's tangent, and
/// the double reflection method's fourth-order accuracy (Wang et al.)
/// assumes exact tangents. A caller that knows its curve's derivatives
/// supplies them with [`rotation_minimizing_frames_along`].
#[must_use]
pub fn rotation_minimizing_frames(
    points: &[[f64; 3]],
    closed: bool,
    seed: Option<[f64; 3]>,
) -> (Vec<Frame>, FrameReport) {
    let mut frames = Vec::new();
    let report = frame(
        points,
        None,
        closed,
        seed,
        [&mut Vec::new(), &mut Vec::new()],
        &mut frames,
    );
    (frames, report)
}

/// [`rotation_minimizing_frames`] with each station's tangent supplied, as
/// a curve's derivative ([`crate::sample_derivative`],
/// [`crate::ValidNurbs::evaluation`]) or an authored tangent: each is
/// normalized and used as given, so the transport is as accurate as the
/// tangents. A zero or non-finite tangent is estimated from the chords as
/// without supplied tangents, and counted ([`FrameReport::estimated`]).
///
/// # Errors
///
/// [`TangentCount`] unless there is one tangent per point.
pub fn rotation_minimizing_frames_along(
    points: &[[f64; 3]],
    tangents: &[[f64; 3]],
    closed: bool,
    seed: Option<[f64; 3]>,
) -> Result<(Vec<Frame>, FrameReport), TangentCount> {
    let mut frames = Vec::new();
    let report = rotation_minimizing_frames_into(
        points,
        Some(tangents),
        closed,
        seed,
        &mut Workspace::new(),
        &mut frames,
    )?;
    Ok((frames, report))
}

/// [`rotation_minimizing_frames`], or with `tangents`
/// [`rotation_minimizing_frames_along`], into a caller's buffer, which is
/// cleared first, with `workspace` as scratch.
///
/// # Errors
///
/// [`TangentCount`] unless `tangents`, when given, has one per point;
/// `out` is left empty.
pub fn rotation_minimizing_frames_into(
    points: &[[f64; 3]],
    tangents: Option<&[[f64; 3]]>,
    closed: bool,
    seed: Option<[f64; 3]>,
    workspace: &mut Workspace,
    out: &mut Vec<Frame>,
) -> Result<FrameReport, TangentCount> {
    out.clear();
    if let Some(tangents) = tangents
        && tangents.len() != points.len()
    {
        return Err(TangentCount {
            points: points.len(),
            tangents: tangents.len(),
        });
    }
    Ok(frame(
        points,
        tangents,
        closed,
        seed,
        [&mut workspace.directions, &mut workspace.outgoing],
        out,
    ))
}

fn frame(
    points: &[[f64; 3]],
    supplied: Option<&[[f64; 3]]>,
    closed: bool,
    seed: Option<[f64; 3]>,
    [directions, outgoing]: [&mut Vec<Option<[f64; 3]>>; 2],
    frames: &mut Vec<Frame>,
) -> FrameReport {
    let n = points.len();
    let mut report = FrameReport {
        stations: u32::try_from(n).unwrap_or(u32::MAX),
        seam_cos: 1.0,
        ..FrameReport::default()
    };
    frames.clear();
    if n == 0 {
        return report;
    }
    // The unit direction from each point to the next (wrapping when
    // closed), or `None` where they coincide.
    let segments = if closed { n } else { n - 1 };
    directions.clear();
    directions.extend((0..segments).map(|i| direction(sub(points[(i + 1) % n], points[i]))));
    // The first direction leaving each point, skipping repeated points:
    // one backward pass, wrapping once around a closed curve.
    outgoing.clear();
    outgoing.resize(n, None);
    let mut ahead = if closed {
        directions.iter().find_map(|d| *d)
    } else {
        None
    };
    for i in (0..segments).rev() {
        ahead = directions[i].or(ahead);
        outgoing[i] = ahead;
    }
    // The last direction arriving at each point, carried forward as the
    // frames are built, likewise wrapping once.
    let mut incoming = if closed {
        directions.iter().rev().find_map(|d| *d)
    } else {
        None
    };
    let directions = &*directions;
    let outgoing = &*outgoing;
    let mut tangent_at = |i: usize, report: &mut FrameReport| {
        if i > 0 {
            incoming = directions[i - 1].or(incoming);
            if points[i] == points[i - 1] {
                report.repeated += 1;
            }
        }
        if let Some(tangents) = supplied {
            match direction(tangents[i]) {
                Some(tangent) => return tangent,
                None => report.estimated += 1,
            }
        }
        match (incoming, outgoing[i]) {
            (Some(a), Some(b)) => bisector(a, b).unwrap_or_else(|| {
                report.cusps += 1;
                report
                    .first_cusp
                    .get_or_insert(u32::try_from(i).unwrap_or(u32::MAX));
                a
            }),
            (Some(d), None) | (None, Some(d)) => d,
            (None, None) => [0.0, 0.0, 1.0],
        }
    };
    let t0 = tangent_at(0, &mut report);
    let seeded = seed.and_then(|s| perpendicular(s, t0));
    let normal = seeded.unwrap_or_else(|| least_aligned_perpendicular(t0));
    frames.reserve(n);
    frames.push(Frame {
        point: points[0],
        tangent: t0,
        normal,
        binormal: cross(t0, normal),
    });
    for i in 1..n {
        let tangent = tangent_at(i, &mut report);
        frames.push(transported(&frames[i - 1], points[i], tangent));
    }
    if closed && n > 1 {
        [report.seam_cos, report.seam_sin] = seam(&frames[0], &frames[n - 1]);
    }
    report
}

/// The frame at `point` with unit `tangent`, carried from `previous` by
/// the double reflection: one step of [`rotation_minimizing_frames`].
pub(crate) fn transported(previous: &Frame, point: [f64; 3], tangent: [f64; 3]) -> Frame {
    let moved = crate::path::double_reflection(
        previous.point,
        previous.normal,
        previous.tangent,
        point,
        tangent,
    );
    // Re-orthonormalize against rounding; a repeated point carries the
    // previous normal, which the new tangent may not be perpendicular to.
    let normal =
        perpendicular(moved, tangent).unwrap_or_else(|| least_aligned_perpendicular(tangent));
    Frame {
        point,
        tangent,
        normal,
        binormal: cross(tangent, normal),
    }
}

/// A closed loop's seam twist `[cos, sin]`: the `last` frame carried on
/// to the `first` station, measured against the first normal
/// ([`FrameReport::seam_cos`]).
pub(crate) fn seam(first: &Frame, last: &Frame) -> [f64; 2] {
    let (normal, t0) = (first.normal, first.tangent);
    let around =
        crate::path::double_reflection(last.point, last.normal, last.tangent, first.point, t0);
    let around = perpendicular(around, t0).unwrap_or(normal);
    // Both are unit and perpendicular to `t0`, so their cross product is
    // `sin(angle) t0`.
    [dot(normal, around), dot(cross(normal, around), t0)]
}

/// The unit direction of `v`'s part perpendicular to the unit `t`, or
/// `None` when `v` is zero, not finite, or within `2^-26` radians of
/// parallel to `t` (its perpendicular part at or below `sqrt(EPSILON) |v|`).
///
/// Removing `v`'s part along `t` leaves a residual of rounding size,
/// around `EPSILON |v|`, when the two are parallel; normalizing that
/// residual would magnify noise into a direction along `t`. Past the
/// threshold the perpendicular part is trusted, and is projected once more
/// ("twice is enough": Parlett, *The Symmetric Eigenvalue Problem*, section
/// 6-9) so its rounding along `t` is that of one projection of an already
/// perpendicular vector.
pub(crate) fn perpendicular(v: [f64; 3], t: [f64; 3]) -> Option<[f64; 3]> {
    let r = sub(v, scale(t, dot(v, t)));
    // Squared lengths, so no square root: |r|^2 > EPSILON |v|^2. Any NaN or
    // overflow compares false.
    if dot(r, r) > f64::EPSILON * dot(v, v) {
        normalize(sub(r, scale(t, dot(r, t))))
    } else {
        None
    }
}

/// `v`'s direction, scaled by its largest coordinate before normalizing,
/// so tiny and huge vectors keep theirs; `None` for a zero or non-finite
/// vector.
pub(crate) fn direction(v: [f64; 3]) -> Option<[f64; 3]> {
    let largest = v.iter().fold(0.0_f64, |m, c| m.max(c.abs()));
    if largest == 0.0 || !largest.is_finite() {
        return None;
    }
    normalize(v.map(|c| c / largest))
}

/// The unit bisector of the unit directions `a` and `b`, or `None` at a
/// cusp: where they reverse to within `2^-26` radians, their sum is
/// rounding, not a direction.
fn bisector(a: [f64; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let sum = add(a, b);
    // Directions an angle `delta` short of a full reversal have
    // `|a + b| = 2 sin(delta / 2)`, about `delta`; `delta > 2^-26` is
    // `|a + b|^2 > EPSILON`, the threshold `perpendicular` uses too.
    if dot(sum, sum) > f64::EPSILON {
        normalize(sum)
    } else {
        None
    }
}

/// The world axis least aligned with `t` (ties x, then y, then z), made
/// perpendicular to it.
fn least_aligned_perpendicular(t: [f64; 3]) -> [f64; 3] {
    let a = [t[0].abs(), t[1].abs(), t[2].abs()];
    let index = if a[0] <= a[1] && a[0] <= a[2] {
        0
    } else if a[1] <= a[2] {
        1
    } else {
        2
    };
    let mut axis = [0.0; 3];
    axis[index] = 1.0;
    normalize(sub(axis, scale(t, dot(axis, t)))).unwrap_or(axis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-12)
    }

    #[test]
    fn a_straight_line_keeps_its_seeded_frame() {
        let points: Vec<[f64; 3]> = (0..4).map(|i| [0.0, 0.0, f64::from(i)]).collect();
        let (frames, report) = rotation_minimizing_frames(&points, false, Some([1.0, 0.0, 0.0]));
        assert_eq!(report.stations, 4);
        for frame in &frames {
            assert_eq!(frame.tangent, [0.0, 0.0, 1.0]);
            assert_eq!(frame.normal, [1.0, 0.0, 0.0]);
            assert_eq!(frame.binormal, [0.0, 1.0, 0.0]);
        }
    }

    #[test]
    fn a_planar_loop_closes_without_twist() {
        let points = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
        ];
        let (frames, report) = rotation_minimizing_frames(&points, true, Some([0.0, 0.0, 1.0]));
        for frame in &frames {
            assert!(close(frame.normal, [0.0, 0.0, 1.0]), "{frame:?}");
            assert!(dot(frame.tangent, frame.normal).abs() < 1e-15);
        }
        assert!((report.seam_cos - 1.0).abs() < 1e-12);
        assert!(report.seam_sin.abs() < 1e-12);
        assert_eq!(report.cusps, 0);
    }

    #[test]
    fn repeated_points_and_cusps_are_reported() {
        let points = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        ];
        let (frames, report) = rotation_minimizing_frames(&points, false, None);
        assert_eq!(report.repeated, 1);
        assert_eq!(report.cusps, 1);
        assert_eq!(report.first_cusp, Some(2));
        assert_eq!(frames[0].tangent, [1.0, 0.0, 0.0]);
        assert_eq!(frames[2].tangent, [1.0, 0.0, 0.0]);
        for frame in &frames {
            assert!(dot(frame.tangent, frame.normal).abs() < 1e-15);
        }
    }

    fn orthonormal(frame: &Frame) -> bool {
        let unit = |v: [f64; 3]| (dot(v, v) - 1.0).abs() < 1e-14;
        unit(frame.tangent)
            && unit(frame.normal)
            && unit(frame.binormal)
            && dot(frame.tangent, frame.normal).abs() < 1e-14
            && dot(frame.tangent, frame.binormal).abs() < 1e-14
            && dot(frame.normal, frame.binormal).abs() < 1e-14
    }

    #[test]
    fn a_seed_along_the_tangent_falls_back_to_an_axis() {
        // Diagonal, antiparallel and nearly parallel seeds leave a rounding
        // residual, not a direction: each takes the least-aligned axis.
        let points = [[0.0; 3], [1.0; 3]];
        let (fallback, _) = rotation_minimizing_frames(&points, false, None);
        for seed in [[1.0; 3], [-2.0; 3], [1.0, 1.0, 1.0 + 1e-12]] {
            let (frames, _) = rotation_minimizing_frames(&points, false, Some(seed));
            assert!(frames.iter().all(orthonormal), "{seed:?}: {frames:?}");
            assert_eq!(frames, fallback, "{seed:?}");
        }
        // A seed a little off the tangent is still the caller's.
        let (frames, _) = rotation_minimizing_frames(&points, false, Some([1.0, 1.0, 1.001]));
        assert!(frames.iter().all(orthonormal));
        assert!(frames[0].normal[2] > 0.8, "{:?}", frames[0]);
    }

    #[test]
    fn a_twisted_loop_reports_its_signed_seam() {
        // A trefoil knot, which is chiral, and its mirror image twist by
        // opposite angles.
        let loop_points: Vec<[f64; 3]> = (0..48)
            .map(|k| {
                let at = |m: f64| {
                    exedra_math::Real::sin_cos(m * core::f64::consts::TAU * f64::from(k) / 48.0)
                };
                let ((s1, c1), (s2, c2), (s3, _)) = (at(1.0), at(2.0), at(3.0));
                [s1 + 2.0 * s2, c1 - 2.0 * c2, -s3]
            })
            .collect();
        let mirrored: Vec<[f64; 3]> = loop_points.iter().map(|p| [p[0], p[1], -p[2]]).collect();
        let (frames, report) =
            rotation_minimizing_frames(&loop_points, true, Some([0.0, 0.0, 1.0]));
        let (_, mirror) = rotation_minimizing_frames(&mirrored, true, Some([0.0, 0.0, -1.0]));
        assert!(report.seam_sin.abs() > 1e-3, "{report:?}");
        assert!((report.seam_cos.powi(2) + report.seam_sin.powi(2) - 1.0).abs() < 1e-12);
        assert!((mirror.seam_cos - report.seam_cos).abs() < 1e-12);
        assert!((mirror.seam_sin + report.seam_sin).abs() < 1e-12);
        // The angle turns the first normal onto the transported one.
        let last = frames[47];
        let first = frames[0];
        let around = crate::path::double_reflection(
            last.point,
            last.normal,
            last.tangent,
            first.point,
            first.tangent,
        );
        let turned = add(
            scale(first.normal, report.seam_cos),
            scale(first.binormal, report.seam_sin),
        );
        assert!(
            close(turned, normalize(around).unwrap()),
            "{turned:?} {around:?}"
        );
    }

    #[test]
    fn repeated_points_wrap_around_a_closed_seam() {
        // A square whose first corner is repeated at both ends of the list:
        // the runs on either side of the seam are skipped as one.
        let points = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        ];
        let (frames, report) = rotation_minimizing_frames(&points, true, None);
        assert_eq!(report.repeated, 2);
        assert_eq!(report.cusps, 0);
        let corner = [
            core::f64::consts::FRAC_1_SQRT_2,
            -core::f64::consts::FRAC_1_SQRT_2,
            0.0,
        ];
        for i in [0, 1, 5, 6] {
            assert!(close(frames[i].tangent, corner), "{i}: {:?}", frames[i]);
        }
        // An all-coincident chain takes +Z everywhere.
        let (frames, report) = rotation_minimizing_frames(&[[2.0; 3]; 5], true, None);
        assert_eq!(report.repeated, 4);
        assert!(frames.iter().all(|f| f.tangent == [0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_near_reversal_is_a_cusp() {
        // The two unit directions at the middle station cancel up to
        // rounding; their sum is noise, not a bisector.
        let points = [[0.0; 3], [1.0; 3], [0.1; 3]];
        let (frames, report) = rotation_minimizing_frames(&points, false, None);
        assert_eq!(report.cusps, 1);
        assert!(frames.iter().all(orthonormal), "{frames:?}");
        assert_eq!(frames[1].tangent, frames[0].tangent);
    }

    #[test]
    fn supplied_tangents_are_used_as_given() {
        // A circle with its exact tangents: a planar loop, so the normal
        // stays on the axis and the seam does not twist.
        let n = 16;
        let (points, tangents): (Vec<[f64; 3]>, Vec<[f64; 3]>) = (0..n)
            .map(|k| {
                let (sin, cos) = exedra_math::Real::sin_cos(
                    core::f64::consts::TAU * f64::from(k) / f64::from(n),
                );
                ([cos, sin, 0.0], [-2.0 * sin, 2.0 * cos, 0.0])
            })
            .unzip();
        let (frames, report) =
            rotation_minimizing_frames_along(&points, &tangents, true, Some([0.0, 0.0, 1.0]))
                .unwrap();
        for (frame, tangent) in frames.iter().zip(&tangents) {
            assert!(close(frame.tangent, scale(*tangent, 0.5)));
            assert!(close(frame.normal, [0.0, 0.0, 1.0]));
            assert!(orthonormal(frame));
        }
        assert!((report.seam_cos - 1.0).abs() < 1e-12 && report.seam_sin.abs() < 1e-12);
        assert_eq!(report.estimated, 0);
        // A zero tangent is estimated from the chords, as without any.
        let mut some = tangents.clone();
        some[3] = [0.0; 3];
        let mut workspace = Workspace::new();
        let mut out = Vec::new();
        let report = rotation_minimizing_frames_into(
            &points,
            Some(&some),
            true,
            None,
            &mut workspace,
            &mut out,
        )
        .unwrap();
        assert_eq!(report.estimated, 1);
        let (chords, _) = rotation_minimizing_frames(&points, true, None);
        assert_eq!(out[3].tangent, chords[3].tangent);
        assert_eq!(
            rotation_minimizing_frames_along(&points, &tangents[1..], true, None),
            Err(TangentCount {
                points: 16,
                tangents: 15
            })
        );
        // The buffers are reused.
        let capacity = out.capacity();
        rotation_minimizing_frames_into(&points, None, true, None, &mut workspace, &mut out)
            .unwrap();
        assert_eq!(out.capacity(), capacity);
        assert_eq!(out, chords);
    }

    #[test]
    fn a_reversal_is_a_cusp_within_two_to_the_minus_26_radians() {
        // Reversals short of a full turn by `delta`: a cusp at or below
        // 2^-26 (about 1.49e-8) radians, a bisector above.
        let turn = |delta: f64| {
            let (sin, cos) = exedra_math::Real::sin_cos(delta);
            let points = [[0.0; 3], [1.0, 0.0, 0.0], [1.0 - cos, sin, 0.0]];
            rotation_minimizing_frames(&points, false, None).1.cusps
        };
        assert_eq!(turn(1.0e-8), 1);
        assert_eq!(turn(2.5e-8), 0);
        assert_eq!(turn(1.0e-6), 0);
    }
}
