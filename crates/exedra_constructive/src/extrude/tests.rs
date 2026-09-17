// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::builders::rect;
use crate::profile::{Loop2, Seg2};
use crate::tessellate::{Feature, REGION_CAP_START};
use exedra_math::{cross, sub};
use exedra_mesh::FaceTriangulation;

fn volume(body: &TessellatedBody) -> f64 {
    let mut triangles = alloc::vec::Vec::new();
    let mut sum = 0.0;
    for face in body.mesh.faces() {
        assert!(
            !body
                .mesh
                .face_triangles_into(face, FaceTriangulation::Robust, &mut triangles)
        );
        for triangle in &triangles {
            let [a, b, c] = triangle.map(|corner| {
                body.mesh
                    .vertex_position(body.mesh.to_vertex(corner).unwrap())
                    .unwrap()
                    .map(f64::from)
            });
            sum += dot(a, cross(b, c)) / 6.0;
        }
    }
    sum
}

fn run(profile: &Profile2, placement: &Placement3, plane: Plane3) -> PlaneExtrusion {
    let result = extrude_to_plane(
        profile,
        placement,
        plane,
        &EvalPolicy::default(),
        &SectionPolicy::default(),
    )
    .unwrap();
    assert!(result.body.mesh.validate_deep().is_empty());
    assert!(result.body.mesh.boundary_loops().unwrap().is_empty());
    result.body.source_map.check(&result.body.mesh).unwrap();
    let regions = result
        .body
        .mesh
        .attrs()
        .dense(exedra_mesh::attr::FACE_REGION)
        .unwrap();
    let (normal, distance) = plane.normalized().unwrap();
    let mut start = 0;
    let mut end = 0;
    for face in result.body.mesh.faces() {
        match result.body.source_map.face_feature(face).unwrap() {
            Feature::CapStart => {
                assert_eq!(regions.get(face.into()), Some(&REGION_CAP_START));
                start += 1;
            }
            Feature::CapEnd => {
                end += 1;
                assert_eq!(regions.get(face.into()), Some(&REGION_CAP_END));
                for corner in result.body.mesh.face_loop(face) {
                    let p = result
                        .body
                        .mesh
                        .vertex_position(result.body.mesh.to_vertex(corner).unwrap())
                        .unwrap()
                        .map(f64::from);
                    assert!((dot(normal, p) - distance).abs() <= 1e-6);
                }
            }
            Feature::Wall { .. } => {}
            other => panic!("unexpected feature {other:?}"),
        }
    }
    assert!(start > 0 && end > 0);
    assert!(volume(&result.body) > 0.0);
    result
}

#[test]
fn oblique_rectangle_has_expected_volume_and_cap_orientation() {
    let result = run(
        &rect(2.0, 1.0).unwrap(),
        &Placement3::IDENTITY,
        Plane3 {
            normal: [-0.25, 0.0, 1.0],
            distance: 2.0,
        },
    );
    // rect spans x=0..2: mean height is 2.25.
    assert!((volume(&result.body) - 4.5).abs() < 1e-6);
    for face in result.body.mesh.faces() {
        if result.body.source_map.face_feature(face) != Some(Feature::CapEnd) {
            continue;
        }
        let p: alloc::vec::Vec<_> = result
            .body
            .mesh
            .face_loop(face)
            .map(|c| {
                result
                    .body
                    .mesh
                    .vertex_position(result.body.mesh.to_vertex(c).unwrap())
                    .unwrap()
                    .map(f64::from)
            })
            .collect();
        assert!(dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), [-0.25, 0.0, 1.0]) > 0.0);
    }
}

#[test]
fn holes_and_reflected_sheared_placements_remain_closed() {
    let outer = rect(4.0, 4.0).unwrap();
    let inner = Profile2::simple(
        Loop2::new(alloc::vec![
            Seg2::line((1.0, 1.0)),
            Seg2::line((3.0, 1.0)),
            Seg2::line((3.0, 3.0)),
            Seg2::line((1.0, 3.0))
        ])
        .unwrap(),
    )
    .unwrap();
    let profile =
        Profile2::new(outer.outer().clone(), alloc::vec![inner.outer().reversed()]).unwrap();
    let placement = Placement3::from_axes(
        [-1.0, 0.0, 0.0],
        [0.0, 2.0, 0.0],
        [0.2, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    );
    let result = run(
        &profile,
        &placement,
        Plane3 {
            normal: [0.0, 0.0, -2.0],
            distance: -6.0,
        },
    );
    assert_eq!(result.section.regions.len(), 1);
    assert_eq!(result.section.regions[0].holes.len(), 1);
    assert!((volume(&result.body) - 72.0).abs() < 1e-4);
}

#[test]
fn parallel_touching_crossing_and_backward_targets_are_refused() {
    let profile = rect(2.0, 2.0).unwrap();
    for plane in [
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 0.0,
        },
        Plane3 {
            normal: [1.0, 0.0, 1.0],
            distance: 0.0,
        },
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: -1.0,
        },
    ] {
        assert!(matches!(
            extrude_to_plane(
                &profile,
                &Placement3::IDENTITY,
                plane,
                &EvalPolicy::default(),
                &SectionPolicy::default()
            ),
            Err(ExtrudeToPlaneError::NotForward)
        ));
    }
    assert!(matches!(
        extrude_to_plane(
            &profile,
            &Placement3::IDENTITY,
            Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 2.0
            },
            &EvalPolicy::default(),
            &SectionPolicy::default()
        ),
        Err(ExtrudeToPlaneError::Parallel)
    ));
}

#[test]
fn curves_and_work_budgets_are_respected() {
    let profile = Profile2::simple(
        Loop2::new(alloc::vec![
            Seg2::arc((1.0, 0.0), 1.0),
            Seg2::arc((-1.0, 0.0), 1.0)
        ])
        .unwrap(),
    )
    .unwrap();
    let plane = Plane3 {
        normal: [0.3, 0.0, 1.0],
        distance: 2.0,
    };
    run(&profile, &Placement3::IDENTITY, plane);
    let budget = SectionPolicy {
        max_triangles: 1,
        ..Default::default()
    };
    assert!(matches!(
        extrude_to_plane(
            &profile,
            &Placement3::IDENTITY,
            plane,
            &EvalPolicy::default(),
            &budget
        ),
        Err(ExtrudeToPlaneError::Section(SectionError::BudgetExceeded))
    ));
}

#[test]
fn invalid_inputs_and_unrepresentable_start_caps_are_refused() {
    let profile = rect(2.0, 2.0).unwrap();
    let plane = Plane3 {
        normal: [0.0, 0.0, 1.0],
        distance: 2.0,
    };
    for placement in [
        Placement3::from_axes([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]),
        Placement3::translate(f64::NAN, 0.0, 0.0),
    ] {
        assert!(matches!(
            extrude_to_plane(
                &profile,
                &placement,
                plane,
                &EvalPolicy::default(),
                &SectionPolicy::default()
            ),
            Err(ExtrudeToPlaneError::InvalidInput)
        ));
    }
    assert!(matches!(
        extrude_to_plane(
            &profile,
            &Placement3::IDENTITY,
            Plane3 {
                normal: [0.0; 3],
                distance: 2.0
            },
            &EvalPolicy::default(),
            &SectionPolicy::default()
        ),
        Err(ExtrudeToPlaneError::InvalidInput)
    ));
    // At this translation f32 rounds the start above its f64 target plane.
    assert!(matches!(
        extrude_to_plane(
            &profile,
            &Placement3::translate(0.0, 0.0, 100_000_005.0),
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 100_000_006.0
            },
            &EvalPolicy::default(),
            &SectionPolicy::default()
        ),
        Err(ExtrudeToPlaneError::NumericLimit) | Err(ExtrudeToPlaneError::Section(_))
    ));
}

#[test]
fn concave_profile_and_refined_caps_are_supported() {
    let profile = Profile2::simple(
        Loop2::new(alloc::vec![
            Seg2::line((0.0, 0.0)),
            Seg2::line((3.0, 0.0)),
            Seg2::line((3.0, 1.0)),
            Seg2::line((1.0, 1.0)),
            Seg2::line((1.0, 3.0)),
            Seg2::line((0.0, 3.0))
        ])
        .unwrap(),
    )
    .unwrap();
    let plane = Plane3 {
        normal: [0.0, 0.0, 1.0],
        distance: 2.0,
    };
    let result = run(&profile, &Placement3::IDENTITY, plane);
    assert!((volume(&result.body) - 10.0).abs() < 1e-5);
    let policy =
        EvalPolicy::default().with_cap_refinement(exedra_triangulate::RefineParams::default());
    let refined = extrude_to_plane(
        &profile,
        &Placement3::IDENTITY,
        plane,
        &policy,
        &SectionPolicy::default(),
    )
    .unwrap();
    assert!(refined.body.mesh.boundary_loops().unwrap().is_empty());
    assert!((volume(&refined.body) - 10.0).abs() < 1e-5);
}

#[test]
fn policy_wrapped_lines_have_the_same_forward_clearance() {
    let plain = rect(2.0, 1.0).unwrap();
    let tagged = Profile2::simple(
        Loop2::new(
            plain
                .outer()
                .segs()
                .iter()
                .map(|segment| Seg2::policy(segment.to, crate::ir::PolicyId(0), SegKind::Line))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let plane = Plane3 {
        normal: [1.0, 0.0, 1.0],
        distance: 2.001,
    };
    let a = run(&plain, &Placement3::IDENTITY, plane);
    let b = run(&tagged, &Placement3::IDENTITY, plane);
    assert_eq!(
        a.body
            .mesh
            .to_trimesh(&exedra_mesh::ExtractParams::default())
            .0,
        b.body
            .mesh
            .to_trimesh(&exedra_mesh::ExtractParams::default())
            .0
    );
}

#[test]
fn cubic_interior_crossing_is_not_hidden_by_safe_endpoints() {
    let profile = Profile2::simple(
        Loop2::new(alloc::vec![
            Seg2::line((0.0, 0.0)),
            Seg2::line((2.0, 0.0)),
            Seg2::cubic((2.0, 2.0), (4.0, 0.0), (4.0, 2.0)),
            Seg2::line((0.0, 2.0)),
        ])
        .unwrap(),
    )
    .unwrap();
    run(
        &profile,
        &Placement3::IDENTITY,
        Plane3 {
            normal: [1.0, 0.0, 1.0],
            distance: 5.0,
        },
    );
    // Every authored endpoint has x <= 2, but the cubic reaches x = 3.5.
    assert!(matches!(
        extrude_to_plane(
            &profile,
            &Placement3::IDENTITY,
            Plane3 {
                normal: [1.0, 0.0, 1.0],
                distance: 2.5
            },
            &EvalPolicy::default(),
            &SectionPolicy::default()
        ),
        Err(ExtrudeToPlaneError::NotForward)
    ));
}
