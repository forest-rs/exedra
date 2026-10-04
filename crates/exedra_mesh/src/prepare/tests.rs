// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;

const SQUARE: [[f32; 3]; 5] = [
    [0.0, 0.0, 0.0],
    [2.0, 0.0, 0.0],
    [2.0, 2.0, 0.0],
    [0.0, 2.0, 0.0],
    [2.0, 0.0, 0.0],
];

fn area2(positions: &[[f32; 3]], indices: &[u32], triangle: &PreparedTriangle) -> f64 {
    let [a, b, c] = triangle
        .corners
        .map(|i| positions[indices[i] as usize].map(f64::from));
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

#[test]
fn warped_concave_face_requires_projection_and_keeps_winding_and_corners() {
    // A fan from corner zero would put triangle [0, 2, 3] outside this face.
    let points = [
        [0.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [3.0, 3.0, 0.0],
        [2.0, 1.0, 0.25],
        [0.0, 3.0, 0.0],
    ];
    let ccw = [0, 1, 2, 3, 4];
    let cw = [4, 3, 2, 1, 0];
    assert_eq!(
        prepare_polygons(&points, &[&ccw], &PrepareParams::default())
            .unwrap_err()
            .kind,
        PrepareErrorKind::WarpedFace
    );
    let params = PrepareParams {
        warped_faces: WarpedFacePolicy::Project,
        ..PrepareParams::default()
    };
    let result = prepare_polygons(&points, &[&ccw, &cw], &params).unwrap();
    assert_eq!(
        result,
        prepare_polygons(&points, &[&ccw, &cw], &params).unwrap()
    );
    assert_eq!(result.projected_faces, [0, 1]);
    assert_eq!(result.triangles.len(), 6);
    for (face, indices) in [&ccw[..], &cw[..]].into_iter().enumerate() {
        let areas: Vec<_> = result
            .triangles
            .iter()
            .filter(|t| t.face == face)
            .map(|t| area2(&points, indices, t))
            .collect();
        assert!(
            areas
                .iter()
                .all(|&a| if face == 0 { a > 0.0 } else { a < 0.0 }),
            "each triangle keeps source winding"
        );
        assert_eq!(
            areas.iter().sum::<f64>(),
            if face == 0 { 12.0 } else { -12.0 }
        );
        let used: Vec<_> = result
            .triangles
            .iter()
            .filter(|t| t.face == face)
            .flat_map(|t| t.corners)
            .collect();
        assert!(
            (0..5).all(|c| used.contains(&c)),
            "all distinct corners keep attribute provenance"
        );
    }
}

#[test]
fn repeated_indices_and_distinct_coincident_vertices_require_explicit_cleanup() {
    let repeated = [0, 1, 1, 2, 3, 0];
    let coincident = [0, 1, 4, 2, 3, 0];
    for indices in [repeated, coincident] {
        assert!(
            crate::Mesh::from_polygons(&SQUARE, &[&indices]).is_err(),
            "raw topology construction cannot accept this loop"
        );
        assert_eq!(
            prepare_polygons(&SQUARE, &[&indices], &PrepareParams::default())
                .unwrap_err()
                .kind,
            PrepareErrorKind::CoincidentCorners {
                first: 1,
                second: 2
            }
        );
        let params = PrepareParams {
            coincident_corners: CoincidentCornerPolicy::DropConsecutive,
            ..PrepareParams::default()
        };
        let result = prepare_polygons(&SQUARE, &[&indices], &params).unwrap();
        assert_eq!(
            result.discarded_corners,
            [
                SourceCorner { face: 0, corner: 2 },
                SourceCorner { face: 0, corner: 5 }
            ]
        );
        assert_eq!(result.triangles.len(), 2);
        for t in &result.triangles {
            assert!(
                t.corners.iter().all(|c| [0, 1, 3, 4].contains(c)),
                "indices address original corner offsets"
            );
            assert!(
                area2(&SQUARE, &indices, t) > 0.0,
                "cleanup emits no zero-area triangles"
            );
        }
    }
}

#[test]
fn distinct_collinear_corners_and_face_provenance_survive() {
    let points = [
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    // Corner zero lies on a chain crossing the cyclic seam and can be pruned by triangulate().
    let face = [0, 1, 2, 3, 4];
    let other = [4, 3, 2, 1, 0];
    let result = prepare_polygons(&points, &[&face, &other], &PrepareParams::default()).unwrap();
    assert_eq!(result.triangles.len(), 6);
    assert!(
        result.discarded_corners.is_empty(),
        "distinct collinear samples are retained"
    );
    for (source_face, indices) in [&face[..], &other[..]].into_iter().enumerate() {
        let triangles: Vec<_> = result
            .triangles
            .iter()
            .filter(|t| t.face == source_face)
            .collect();
        assert_eq!(
            triangles
                .iter()
                .map(|t| area2(&points, indices, t))
                .sum::<f64>(),
            if source_face == 0 { 8.0 } else { -8.0 }
        );
        for corner in 0..5 {
            let next = (corner + 1) % 5;
            let incidence = triangles
                .iter()
                .flat_map(|t| (0..3).map(|i| (t.corners[i], t.corners[(i + 1) % 3])))
                .filter(|&(a, b)| a == corner && b == next)
                .count();
            assert_eq!(
                incidence, 1,
                "every authored boundary edge keeps its source corners"
            );
        }
    }
}

#[test]
fn degenerate_faces_drop_only_when_requested_and_keep_later_face_mapping() {
    let points = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let faces: [&[u32]; 4] = [&[0, 1, 2], &[], &[0, 1], &[0, 1, 3]];
    assert_eq!(
        prepare_polygons(&points, &faces, &PrepareParams::default()).unwrap_err(),
        PrepareError {
            face: 0,
            kind: PrepareErrorKind::DegenerateFace
        }
    );
    let params = PrepareParams {
        degenerate_faces: DegenerateFacePolicy::Drop,
        ..PrepareParams::default()
    };
    let result = prepare_polygons(&points, &faces, &params).unwrap();
    assert_eq!(result.dropped_faces, [0, 1, 2]);
    assert_eq!(
        result.triangles,
        [PreparedTriangle {
            face: 3,
            corners: [2, 0, 1]
        }]
    );
    assert!(
        area2(&points, faces[3], &result.triangles[0]) > 0.0,
        "surviving triangle has positive area"
    );
}

#[test]
fn wholly_collinear_faces_apply_degenerate_policy_before_simplicity_checks() {
    let points = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    // Nonconsecutive coincidence and backtracking do not override exact degeneracy.
    let faces: [&[u32]; 1] = [&[0, 1, 0, 2]];
    assert_eq!(
        prepare_polygons(&points, &faces, &PrepareParams::default()).unwrap_err(),
        PrepareError {
            face: 0,
            kind: PrepareErrorKind::DegenerateFace,
        }
    );
    let params = PrepareParams {
        degenerate_faces: DegenerateFacePolicy::Drop,
        ..PrepareParams::default()
    };
    assert_eq!(
        prepare_polygons(&points, &faces, &params).unwrap(),
        PreparedPolygons {
            dropped_faces: vec![0],
            ..PreparedPolygons::default()
        }
    );
}

#[test]
fn finite_triangles_with_extreme_dynamic_range_keep_exact_winding() {
    let points = [[1e20_f32, 1e20_f32, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
    for winding in [[0, 1, 2], [0, 2, 1]] {
        for rotation in 0..3 {
            let indices = core::array::from_fn::<_, 3, _>(|i| winding[(i + rotation) % 3]);
            let [a, b, c] = indices.map(|i| points[i as usize].map(f64::from));
            let expected_2d = orient2d([a[0], a[1]], [b[0], b[1]], [c[0], c[1]]);
            let expected_3d = orient3d(a, b, c, [0.0, 0.0, 1.0]);
            assert_ne!(expected_2d, Orientation::Collinear, "exact nonzero area");
            assert_ne!(expected_3d, Orientation3d::Coplanar, "exact nonzero area");
            let result = prepare_polygons(&points, &[&indices], &PrepareParams::default()).unwrap();
            assert_eq!(
                result,
                PreparedPolygons {
                    triangles: vec![PreparedTriangle {
                        face: 0,
                        corners: [2, 0, 1],
                    }],
                    ..PreparedPolygons::default()
                }
            );
            let [a, b, c] = result.triangles[0]
                .corners
                .map(|i| points[indices[i] as usize].map(f64::from));
            assert_eq!(
                orient2d([a[0], a[1]], [b[0], b[1]], [c[0], c[1]]),
                expected_2d,
                "source winding survives every cyclic rotation"
            );
            assert_eq!(
                orient3d(a, b, c, [0.0, 0.0, 1.0]),
                expected_3d,
                "3D winding survives every cyclic rotation"
            );
        }
    }
}

#[test]
fn noncollinear_self_crossing_touching_and_backtracking_faces_never_fall_back_to_fans() {
    let params = PrepareParams {
        degenerate_faces: DegenerateFacePolicy::Drop,
        coincident_corners: CoincidentCornerPolicy::DropConsecutive,
        ..PrepareParams::default()
    };
    let fixtures = [
        vec![
            [0.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
        ],
        vec![
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 3.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
        vec![
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [3.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
        vec![
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 3.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
    ];
    for points in fixtures {
        let indices: Vec<_> = (0..u32::try_from(points.len()).unwrap()).collect();
        let error = prepare_polygons(&points, &[&indices], &params).unwrap_err();
        assert!(
            matches!(
                error.kind,
                PrepareErrorKind::ZeroProjectedArea | PrepareErrorKind::NonSimpleProjection
            ),
            "invalid boundary stays an error: {error:?}"
        );
    }
}

#[test]
fn coincident_projection_does_not_discard_distinct_3d_corners() {
    let points = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 0.0, 0.125],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
    ];
    let params = PrepareParams {
        warped_faces: WarpedFacePolicy::Project,
        coincident_corners: CoincidentCornerPolicy::DropConsecutive,
        ..PrepareParams::default()
    };
    assert_eq!(
        prepare_polygons(&points, &[&[0, 1, 2, 3, 4]], &params)
            .unwrap_err()
            .kind,
        PrepareErrorKind::NonSimpleProjection
    );
}

#[test]
fn budgets_and_referenced_position_errors_are_checked_before_geometry() {
    let params = PrepareParams {
        max_corners_per_face: 2,
        ..PrepareParams::default()
    };
    assert_eq!(
        prepare_polygons(&[], &[&[0, 1, 2]], &params)
            .unwrap_err()
            .kind,
        PrepareErrorKind::CornerLimit
    );
    assert_eq!(
        prepare_polygons(&SQUARE, &[&[0, 99, 1]], &PrepareParams::default())
            .unwrap_err()
            .kind,
        PrepareErrorKind::MissingPosition {
            corner: 1,
            index: 99
        }
    );
    let mut points = SQUARE;
    points[4][0] = f32::NAN;
    assert!(
        prepare_polygons(&points, &[&[0, 1, 2, 3]], &PrepareParams::default()).is_ok(),
        "unused positions need no geometry validation"
    );
    assert_eq!(
        prepare_polygons(&points, &[&[0, 4, 2]], &PrepareParams::default())
            .unwrap_err()
            .kind,
        PrepareErrorKind::NonFinitePosition { corner: 1 }
    );
    assert_eq!(
        prepare_polygons(&[], &[], &PrepareParams::default()).unwrap(),
        PreparedPolygons::default()
    );
}
