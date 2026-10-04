// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Tiny source-derived regressions from Intel New Sponza,
// main_sponza/NewSponza_Main_USD_Yup_003.usda (local asset).
// Values retain exact authored f32 bits; no USD runtime is needed for tests.

use super::*;

// /root/_st_floor/arch_stones_01, face 0; original point indices [2, 1031, 1032, 1035].
const WARPED: [[f32; 3]; 4] = [
    [
        f32::from_bits(0xc3ac2113),
        f32::from_bits(0x41ea21f6),
        f32::from_bits(0x42594f1f),
    ],
    [
        f32::from_bits(0xc3ab4b10),
        f32::from_bits(0x41ea21f6),
        f32::from_bits(0x425fa564),
    ],
    [
        f32::from_bits(0xc3ab3ce2),
        f32::from_bits(0x41fd879c),
        f32::from_bits(0x42600675),
    ],
    [
        f32::from_bits(0xc3ac09c5),
        f32::from_bits(0x42018e2a),
        f32::from_bits(0x4259f7ac),
    ],
];

// /root/_st_floor/wooden_parts, face 18404; original point indices [21381, 22855, 19299, 23011].
const COINCIDENT: [[f32; 3]; 4] = [
    [
        f32::from_bits(0xc3f5f780),
        f32::from_bits(0x43b9739d),
        f32::from_bits(0x42f2d450),
    ],
    [
        f32::from_bits(0xc3f5f780),
        f32::from_bits(0x43b9739d),
        f32::from_bits(0x42f2ede8),
    ],
    [
        f32::from_bits(0xc3f5f780),
        f32::from_bits(0x43bc36c1),
        f32::from_bits(0x42f2d450),
    ],
    [
        f32::from_bits(0xc3f5f780),
        f32::from_bits(0x43bc36c1),
        f32::from_bits(0x42f2d450),
    ],
];

// /root/_st_floor/arch_stones_04, face 5605; original point indices [5817, 5547, 5816].
const ZERO_AREA: [[f32; 3]; 3] = [
    [
        f32::from_bits(0xc4a493e0),
        f32::from_bits(0x43586e08),
        f32::from_bits(0x42475694),
    ],
    [
        f32::from_bits(0xc4a493e0),
        f32::from_bits(0x43586e08),
        f32::from_bits(0x42475694),
    ],
    [
        f32::from_bits(0xc4a493e0),
        f32::from_bits(0x43586e08),
        f32::from_bits(0x42475694),
    ],
];

fn import_policy() -> PrepareParams {
    PrepareParams {
        warped_faces: WarpedFacePolicy::Project,
        coincident_corners: CoincidentCornerPolicy::DropConsecutive,
        degenerate_faces: DegenerateFacePolicy::Drop,
        ..PrepareParams::default()
    }
}

fn has_area(points: &[[f32; 3]], triangle: &PreparedTriangle) -> bool {
    let [a, b, c] = triangle.corners.map(|i| points[i].map(f64::from));
    !collinear(a, b, c)
}

#[test]
fn sponza_warped_arch_stone_quad_projects_without_zero_area_results() {
    assert_eq!(
        prepare_polygons(&WARPED, &[&[0, 1, 2, 3]], &PrepareParams::default())
            .unwrap_err()
            .kind,
        PrepareErrorKind::WarpedFace
    );
    let result = prepare_polygons(&WARPED, &[&[0, 1, 2, 3]], &import_policy()).unwrap();
    assert_eq!(result.projected_faces, [0]);
    assert_eq!(result.triangles.len(), 2);
    assert!(
        result.triangles.iter().all(|t| has_area(&WARPED, t)),
        "source-derived warped quad emits nonzero 3D triangles"
    );
    let used: Vec<_> = result.triangles.iter().flat_map(|t| t.corners).collect();
    assert!(
        (0..4).all(|i| used.contains(&i)),
        "each original source corner remains addressable"
    );
}

#[test]
fn sponza_coincident_wooden_parts_corners_need_explicit_attribute_choice() {
    assert_eq!(COINCIDENT[2], COINCIDENT[3]);
    assert_eq!(
        prepare_polygons(&COINCIDENT, &[&[0, 1, 2, 3]], &PrepareParams::default())
            .unwrap_err()
            .kind,
        PrepareErrorKind::CoincidentCorners {
            first: 2,
            second: 3
        }
    );
    let result = prepare_polygons(&COINCIDENT, &[&[0, 1, 2, 3]], &import_policy()).unwrap();
    assert_eq!(
        result.discarded_corners,
        [SourceCorner { face: 0, corner: 3 }]
    );
    assert!(result.projected_faces.is_empty());
    assert_eq!(result.triangles.len(), 1);
    assert!(
        has_area(&COINCIDENT, &result.triangles[0]),
        "explicit corner cleanup leaves a nonzero source triangle"
    );
    let mut retained = result.triangles[0].corners;
    retained.sort_unstable();
    assert_eq!(
        retained,
        [0, 1, 2],
        "the first coincident corner supplies attribute provenance"
    );
}

#[test]
fn sponza_collapsed_arch_stone_triangle_is_reported_and_dropped() {
    assert!(
        ZERO_AREA.iter().all(|&p| p == ZERO_AREA[0]),
        "authored positions really coincide"
    );
    let result = prepare_polygons(&ZERO_AREA, &[&[0, 1, 2]], &import_policy()).unwrap();
    assert_eq!(result.dropped_faces, [0]);
    assert_eq!(
        result.discarded_corners,
        [
            SourceCorner { face: 0, corner: 1 },
            SourceCorner { face: 0, corner: 2 }
        ]
    );
    assert!(
        result.triangles.is_empty(),
        "zero-area source faces never produce render triangles"
    );
}
