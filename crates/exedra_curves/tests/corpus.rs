// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A regression corpus of difficult curves, checked through the public API
//! for the invariants every result must keep: orthonormal frames, surface
//! normals, both sides of a jump, and `_into` forms equal to the
//! allocating ones.

use exedra_curves::{
    CubicBasis, Frame, Nurbs, NurbsSamples, SweptMesh, Tolerance, Workspace, adaptive_stations,
    ribbon, ribbon_into, rotation_minimizing_frames, rotation_minimizing_frames_along,
    rotation_minimizing_frames_into, sample, sample_derivative, stations, tube, tube_into,
};
use exedra_math::{Real, cross, dot, norm, sub};

/// Named point sequences that have broken frame construction somewhere.
fn paths() -> Vec<(&'static str, Vec<[f64; 3]>, bool)> {
    let trefoil = (0..48)
        .map(|k| {
            let at = |m: f64| Real::sin_cos(m * core::f64::consts::TAU * f64::from(k) / 48.0);
            let ((s1, c1), (s2, c2), (s3, _)) = (at(1.0), at(2.0), at(3.0));
            [s1 + 2.0 * s2, c1 - 2.0 * c2, -s3]
        })
        .collect();
    vec![
        ("diagonal", vec![[0.0; 3], [1.0; 3], [2.0; 3]], false),
        (
            "exact reversal",
            vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0; 3]],
            false,
        ),
        ("near reversal", vec![[0.0; 3], [1.0; 3], [0.1; 3]], false),
        ("all coincident", vec![[2.0; 3]; 6], false),
        ("all coincident, closed", vec![[2.0; 3]; 6], true),
        (
            "repeated runs",
            vec![
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            false,
        ),
        (
            "repeated across the seam",
            vec![
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0; 3],
            ],
            true,
        ),
        ("trefoil", trefoil, true),
        ("single point", vec![[1.0, 2.0, 3.0]], false),
    ]
}

fn orthonormal(frame: &Frame) -> bool {
    let unit = |v: [f64; 3]| (dot(v, v) - 1.0).abs() < 1e-13;
    unit(frame.tangent)
        && unit(frame.normal)
        && unit(frame.binormal)
        && dot(frame.tangent, frame.normal).abs() < 1e-13
        && dot(frame.tangent, frame.binormal).abs() < 1e-13
        && dot(frame.normal, frame.binormal).abs() < 1e-13
}

#[test]
fn frames_stay_orthonormal_for_every_seed() {
    let seeds = [
        None,
        Some([1.0; 3]),
        Some([-1.0; 3]),
        Some([1.0, 1.0, 1.0 + 1e-12]),
        Some([0.0, 0.0, 1.0]),
        Some([0.0; 3]),
        Some([f64::NAN; 3]),
    ];
    let mut workspace = Workspace::new();
    let mut out = Vec::new();
    for (name, points, closed) in paths() {
        for seed in seeds {
            let (frames, report) = rotation_minimizing_frames(&points, closed, seed);
            assert_eq!(frames.len(), points.len(), "{name}");
            assert!(
                frames.iter().all(orthonormal),
                "{name} {seed:?}: {frames:?}"
            );
            let into = rotation_minimizing_frames_into(
                &points,
                None,
                closed,
                seed,
                &mut workspace,
                &mut out,
            )
            .unwrap();
            assert_eq!(
                (out.as_slice(), into),
                (frames.as_slice(), report),
                "{name}"
            );
            if closed {
                let (c, s) = (report.seam_cos, report.seam_sin);
                assert!((c * c + s * s - 1.0).abs() < 1e-12, "{name}: {report:?}");
            }
        }
    }
}

#[test]
fn supplied_tangents_on_a_sampled_chain() {
    // A Hermite chain's derivative frames it as well as its chords.
    let controls = [
        [0.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [2.0, 1.0, 0.0],
        [0.0, 3.0, 1.0],
        [4.0, 2.0, 2.0],
        [3.0, 0.0, 0.0],
    ];
    let spans = [[0, 1, 2, 3], [2, 3, 4, 5]];
    let at = stations(2, 16, false);
    let points = sample(CubicBasis::Hermite, &controls, &spans, &at).unwrap();
    let tangents = sample_derivative(CubicBasis::Hermite, &controls, &spans, &at).unwrap();
    let (frames, report) =
        rotation_minimizing_frames_along(&points, &tangents, false, None).unwrap();
    assert_eq!(report.estimated, 0);
    assert!(frames.iter().all(orthonormal));
    let (chords, _) = rotation_minimizing_frames(&points, false, None);
    for (a, b) in frames.iter().zip(&chords) {
        // Fine sampling: the chord bisector is close to the derivative.
        assert!(dot(a.tangent, b.tangent) > 0.99);
    }
}

/// Each normal is perpendicular to the mesh's longitudinal edges at its
/// vertex, for a straight centerline.
fn assert_surface_normals(mesh: &SweptMesh) {
    let ring = mesh.ring as usize;
    let stations = mesh.positions.len() / ring;
    for i in 0..stations {
        for k in 0..ring {
            let v = i * ring + k;
            for j in [i.wrapping_sub(1), i + 1] {
                if j >= stations {
                    continue;
                }
                let edge = sub(mesh.positions[j * ring + k], mesh.positions[v]);
                if norm(edge) == 0.0 {
                    continue;
                }
                let cosine = dot(mesh.normals[v], edge) / norm(edge);
                assert!(cosine.abs() < 1e-12, "station {i} vertex {k}: {cosine}");
            }
        }
    }
}

#[test]
fn tapered_tubes_have_surface_normals() {
    let points: Vec<[f64; 3]> = (0..6).map(|i| [0.0, 0.0, f64::from(i) * 0.5]).collect();
    let (frames, _) = rotation_minimizing_frames(&points, false, Some([1.0, 0.0, 0.0]));
    let mut workspace = Workspace::new();
    let mut out = SweptMesh::default();
    for radii in [
        [1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
        [3.0, 2.4, 1.8, 1.2, 0.6, 0.0],
        [0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        [1.0; 6],
    ] {
        let mesh = tube(&frames, &radii, 7, false).unwrap();
        assert_surface_normals(&mesh);
        tube_into(&frames, &radii, 7, false, &mut workspace, &mut out).unwrap();
        assert_eq!(out, mesh);
    }
    // Ribbons keep their width whatever their authored normals.
    let normals = [
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
        [0.0; 3],
        [f64::NAN; 3],
        [0.0, 1.0, 0.0],
    ];
    let mesh = ribbon(&frames, Some(&normals), &[2.0; 6], false).unwrap();
    assert_eq!(mesh.report.unoriented, 3);
    for pair in mesh.positions.chunks(2) {
        assert!((norm(sub(pair[1], pair[0])) - 2.0).abs() < 1e-14);
    }
    for (normal, frame) in mesh.normals.chunks(2).zip(&frames) {
        assert!(dot(normal[0], frame.tangent).abs() < 1e-14);
        assert_eq!(normal[0], normal[1]);
    }
    ribbon_into(&frames, Some(&normals), &[2.0; 6], false, &mut out).unwrap();
    assert_eq!(out, mesh);
}

#[test]
fn nurbs_jumps_keep_both_sides_everywhere() {
    // Order 3 with a knot of multiplicity 3 at 1: two parabolic pieces.
    let points = [
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [2.0, 0.0, 0.0],
        [5.0, 0.0, 0.0],
        [6.0, 1.0, 0.0],
        [7.0, 0.0, 0.0],
    ];
    let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0];
    let curve = Nurbs {
        order: 3,
        knots: &knots,
        points: &points,
        weights: Some(&[1.0, 2.0, 1.0, 1.0, 0.5, 1.0]),
    }
    .validate()
    .unwrap();
    let check = |samples: &NurbsSamples| {
        assert_eq!(samples.breaks.len(), 1);
        let b = samples.breaks[0];
        assert_eq!(samples.params[b - 1], samples.params[b]);
        assert_eq!(samples.points[b - 1], [2.0, 0.0, 0.0]);
        assert_eq!(samples.points[b], [5.0, 0.0, 0.0]);
    };
    check(&curve.sample(None, 4).unwrap());
    let prepared = curve.prepare();
    assert_eq!(prepared.breaks(), [1]);
    let tolerance = Tolerance {
        chord: 1e-3,
        angle: 0.2,
    };
    check(&prepared.adaptive_sample(tolerance, 1000).0);
    let left = prepared.evaluation_in(0, 1.0).unwrap();
    let right = prepared.evaluation_in(1, 1.0).unwrap();
    assert_eq!(left.point, [2.0, 0.0, 0.0]);
    assert!((right.point[0] - 5.0).abs() < 1e-15);
    // One-sided derivatives point along each piece's end controls.
    assert!(
        cross(left.first, [1.0, -1.0, 0.0])
            .iter()
            .all(|c| c.abs() < 1e-12)
    );
    assert!(
        cross(right.first, [1.0, 1.0, 0.0])
            .iter()
            .all(|c| c.abs() < 1e-12)
    );
}

#[test]
fn adaptive_sampling_is_deterministic_and_capped() {
    let controls: Vec<[f64; 3]> = (0..12)
        .map(|k| {
            let (s, c) = Real::sin_cos(f64::from(k));
            [c * f64::from(k), s * f64::from(k), 0.1 * f64::from(k)]
        })
        .collect();
    let spans: Vec<[u32; 4]> = (0..9).map(|s| [s, s + 1, s + 2, s + 3]).collect();
    let tolerance = Tolerance {
        chord: 1e-4,
        angle: 0.05,
    };
    let first = adaptive_stations(
        CubicBasis::BSpline,
        &controls,
        &spans,
        false,
        tolerance,
        100_000,
    )
    .unwrap();
    let again = adaptive_stations(
        CubicBasis::BSpline,
        &controls,
        &spans,
        false,
        tolerance,
        100_000,
    )
    .unwrap();
    assert_eq!(first, again);
    assert!(!first.1.capped);
    for max in [0, 10, 50, 200] {
        let (at, report) = adaptive_stations(
            CubicBasis::BSpline,
            &controls,
            &spans,
            false,
            tolerance,
            max,
        )
        .unwrap();
        assert!(report.capped);
        assert!(
            at.len() <= (max as usize).max(spans.len() + 1),
            "{max}: {}",
            at.len()
        );
        // Stations stay ordered: span, then parameter.
        assert!(
            at.windows(2)
                .all(|w| (w[0].span, w[0].u) < (w[1].span, w[1].u))
        );
    }
}

/// The distance from `p` to the segment from `a` to `b`.
fn to_segment(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let along = sub(b, a);
    let offset = sub(p, a);
    let length2 = dot(along, along);
    let s = if length2 > 0.0 {
        (dot(offset, along) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    norm(sub(offset, [along[0] * s, along[1] * s, along[2] * s]))
}

#[test]
fn adaptive_sampling_sees_an_excursion_its_samples_miss() {
    // A Bezier span whose ends, middle and end tangents all agree, while
    // the curve swings out to about 0.2887 between them.
    let controls = [[0.0; 3], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0; 3]];
    let tolerance = Tolerance {
        chord: 1e-3,
        angle: 0.1,
    };
    let (at, report) = adaptive_stations(
        CubicBasis::Bezier,
        &controls,
        &[[0, 1, 2, 3]],
        false,
        tolerance,
        1000,
    )
    .unwrap();
    assert!(!report.capped && at.len() > 2);
    for pair in at.windows(2) {
        let [a, b] = [pair[0].u, pair[1].u];
        let ends = [a, b].map(|u| CubicBasis::Bezier.evaluate(controls, u));
        for k in 1..64 {
            let p = CubicBasis::Bezier.evaluate(controls, a + (b - a) * f64::from(k) / 64.0);
            assert!(to_segment(p, ends[0], ends[1]) <= 1e-3);
        }
    }
    // The same curve as a NURBS.
    let curve = Nurbs {
        order: 4,
        knots: &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        points: &controls,
        weights: Some(&[1.0, 2.0, 0.5, 1.0]),
    }
    .validate()
    .unwrap();
    let (samples, report) = curve.prepare().adaptive_sample(tolerance, 1000);
    assert!(!report.capped && samples.points.len() > 2);
    for (pair, points) in samples.params.windows(2).zip(samples.points.windows(2)) {
        for k in 1..64 {
            let p = curve.evaluate(pair[0] + (pair[1] - pair[0]) * f64::from(k) / 64.0);
            assert!(to_segment(p, points[0], points[1]) <= 1e-3);
        }
    }
}
