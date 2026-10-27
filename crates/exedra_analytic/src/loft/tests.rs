// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{
    body::{Body, ValidationPolicy},
    geometry::{ParameterRange, Surface},
    realize::VertexSource,
};
use alloc::vec;
use exedra_math::{cross, dot};
use exedra_profile::{Loop, Segment};

fn rectangle(h: f64) -> Profile {
    let ring = Loop::new(vec![
        Segment::line([h, -h]),
        Segment::line([h, h]),
        Segment::line([-h, h]),
        Segment::line([-h, -h]),
    ])
    .unwrap();
    Profile::new(ring, vec![], &ProfilePolicy::new(1e-9)).unwrap()
}
fn circle(r: f64, hole: Option<f64>) -> Profile {
    let ring = |r: f64, sign: f64| {
        Loop::new(vec![
            Segment::arc([r, 0.0], sign * libm::tan(core::f64::consts::FRAC_PI_8)),
            Segment::arc(
                [0.0, sign * r],
                sign * libm::tan(core::f64::consts::FRAC_PI_8),
            ),
            Segment::arc([-r, 0.0], sign * libm::tan(core::f64::consts::FRAC_PI_8)),
            Segment::arc(
                [0.0, -sign * r],
                sign * libm::tan(core::f64::consts::FRAC_PI_8),
            ),
        ])
        .unwrap()
    };
    Profile::new(
        ring(r, 1.0),
        hole.map(|h| vec![ring(h, -1.0)]).unwrap_or_default(),
        &ProfilePolicy::new(1e-9),
    )
    .unwrap()
}
fn policy() -> LoftPolicy {
    LoftPolicy::new(ProfilePolicy::new(1e-9))
}
#[test]
fn both_source_labels_survive_and_large_occurrence_quantization_is_reported() {
    let tagged = |tag| {
        let p = rectangle(2.0);
        Profile::new(
            Loop::new(
                p.outer()
                    .segments()
                    .iter()
                    .copied()
                    .map(|s| s.tagged(tag))
                    .collect(),
            )
            .unwrap(),
            vec![],
            &ProfilePolicy::new(1e-9),
        )
        .unwrap()
    };
    let loft = Loft::from_profiles(
        [
            (Placement3::IDENTITY, tagged(SegTag(7))),
            (Placement3::translate(0.0, 0.0, 10.0), tagged(SegTag(9))),
        ],
        Placement3::translate(1_000_000.03, 0.0, 0.0),
        &policy(),
    )
    .unwrap();
    for tag in [SegTag(7), SegTag(9)] {
        assert_eq!(loft.faces_tagged(tag).count(), 4);
    }
    assert!(loft.body().faces()[..4].iter().all(|f| matches!(
        f.role,
        FaceRole::Wall {
            tags: [Some(SegTag(7)), Some(SegTag(9))],
            ..
        }
    )));
    let result = loft
        .realize(&RealizationPolicy {
            tolerance: 0.01,
            ..RealizationPolicy::default()
        })
        .unwrap();
    assert!(result.report.max_vertex_displacement > 0.029);
    assert!(!result.report.within_tolerance());
    assert!(result.mesh.validate_deep().is_empty());
}
fn transition() -> Loft {
    Loft::from_profiles(
        [
            (Placement3::IDENTITY, circle(2.0, None)),
            (Placement3::translate(0.0, 0.0, 10.0), rectangle(3.0)),
        ],
        Placement3::IDENTITY,
        &policy(),
    )
    .unwrap()
}
fn volume(mesh: &exedra_mesh::Mesh) -> f64 {
    mesh.faces()
        .map(|f| {
            let mut corners = mesh.face_loop(f);
            let p = core::array::from_fn::<_, 3, _>(|_| {
                mesh.vertex_position(mesh.to_vertex(corners.next().unwrap()).unwrap())
                    .unwrap()
                    .map(f64::from)
            });
            dot(p[0], cross(p[1], p[2])) / 6.0
        })
        .sum()
}
#[test]
fn planar_taper_is_exact_and_curved_walls_remesh_without_reconstructing_sources() {
    let taper = Loft::from_profiles(
        [
            (Placement3::IDENTITY, rectangle(2.0)),
            (Placement3::translate(0.0, 0.0, 10.0), rectangle(3.0)),
        ],
        Placement3::IDENTITY,
        &policy(),
    )
    .unwrap();
    let p = RealizationPolicy {
        tolerance: 0.1,
        ..RealizationPolicy::default()
    };
    for (end, expected) in [(End::Start, -1.0), (End::End, 1.0)] {
        let face = taper.cap(end).unwrap();
        let plane = taper.body().planar_frame(face, [0.0; 2]).unwrap();
        assert_eq!(plane.rows[2][2], expected);
    }
    let mesh = taper.realize(&p).unwrap();
    assert_eq!(mesh.mesh.vertices().count(), 8);
    assert!((volume(&mesh.mesh) - 10.0 * (16.0 + 24.0 + 36.0) / 3.0).abs() < 1e-9);
    let loft = transition();
    assert!(loft.body().is_closed());
    assert_eq!(loft.body().faces().len(), 6);
    assert!(
        loft.body().faces()[..4]
            .iter()
            .all(|f| matches!(f.surface, Surface::Ruled(_)))
    );
    let coarse = loft.realize(&p).unwrap();
    let fine = loft
        .realize(&RealizationPolicy {
            tolerance: 0.005,
            ..p
        })
        .unwrap();
    assert!(fine.mesh.vertices().count() > coarse.mesh.vertices().count());
    for result in [&coarse, &fine] {
        assert!(result.report.within_tolerance(), "{:?}", result.report);
        assert!(result.mesh.validate_deep().is_empty());
        assert!(result.mesh.boundary_loops().unwrap().is_empty());
        assert!(volume(&result.mesh) > 0.0);
        assert!(
            result
                .mesh_vertices
                .iter()
                .any(|(_, s)| matches!(s, VertexSource::Face { .. }))
        );
    }
    let detached = Body::try_from_parts(
        loft.body().clone().into_parts(),
        &ValidationPolicy { max_gap: 1e-9 },
    )
    .unwrap();
    let again = detached.realize(Placement3::IDENTITY, &p).unwrap();
    assert_eq!(again.report, coarse.report);
    assert_eq!(again.mesh.faces().count(), coarse.mesh.faces().count());
    assert_eq!(
        again
            .mesh
            .vertices()
            .map(|v| *again.mesh.vertex_position(v).unwrap())
            .collect::<Vec<_>>(),
        coarse
            .mesh
            .vertices()
            .map(|v| *coarse.mesh.vertex_position(v).unwrap())
            .collect::<Vec<_>>()
    );
}
#[test]
fn hollow_variable_sections_share_intermediate_edges_and_keep_oriented_incidence() {
    let loft = Loft::from_profiles(
        [
            (Placement3::IDENTITY, circle(2.0, Some(1.0))),
            (
                Placement3::rotate_z_then_translate(0.2, 0.5, 0.0, 5.0),
                circle(3.0, Some(2.0)),
            ),
            (
                Placement3::translate(0.0, 0.0, 10.0),
                circle(2.0, Some(1.0)),
            ),
        ],
        Placement3::IDENTITY,
        &policy(),
    )
    .unwrap();
    assert_eq!(loft.body().vertices().len(), 24);
    assert_eq!(loft.body().edges().len(), 40);
    assert_eq!(loft.body().faces().len(), 18);
    assert!(loft.body().is_closed());
    let result = loft
        .realize(&RealizationPolicy {
            tolerance: 0.02,
            ..RealizationPolicy::default()
        })
        .unwrap();
    assert!(result.mesh.validate_deep().is_empty());
    assert!(result.mesh.boundary_loops().unwrap().is_empty());
    assert!(result.report.within_tolerance());
}
#[test]
fn source_refusals_preserve_section_band_and_work_limits() {
    assert!(matches!(
        Loft::from_profiles(
            [(Placement3::IDENTITY, rectangle(2.0))],
            Placement3::IDENTITY,
            &policy()
        ),
        Err(LoftError::Sections)
    ));
    assert!(matches!(
        Loft::from_profiles(
            [
                (Placement3::IDENTITY, rectangle(2.0)),
                (Placement3::IDENTITY, rectangle(3.0))
            ],
            Placement3::IDENTITY,
            &policy()
        ),
        Err(LoftError::Direction { band: 0, .. })
    ));
    let mut p = policy();
    p.max_native_elements = 10;
    assert!(matches!(
        Loft::from_profiles(
            [
                (Placement3::IDENTITY, rectangle(2.0)),
                (Placement3::translate(0.0, 0.0, 1.0), rectangle(3.0))
            ],
            Placement3::IDENTITY,
            &p
        ),
        Err(LoftError::Budget { .. })
    ));
    let loft = transition();
    let mut parts = loft.body().clone().into_parts();
    parts.faces[0].domain.u = ParameterRange::new(-0.1, 1.0).unwrap();
    assert!(Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).is_err());
    assert!(matches!(
        loft.realize(&RealizationPolicy {
            max_vertices: 1,
            ..RealizationPolicy::default()
        }),
        Err(RealizationError::Budget { .. })
    ));
}

#[test]
fn construction_charts_preserve_geometry_exact_distances_and_both_seam_coordinates() {
    let loft = transition();
    let p = RealizationPolicy {
        tolerance: 0.05,
        ..RealizationPolicy::default()
    };
    let plain = loft.realize(&p).unwrap();
    let mut calls = 0;
    let mut min = f64::INFINITY;
    let mut max = 0.0_f64;
    let charted = loft
        .realize_with_uvs(
            &p,
            0,
            &[0.0, 10.0],
            |_, _| 7,
            |role, points| -> Result<_, usize> {
                calls += 1;
                if matches!(role, FaceRole::Wall { .. }) {
                    for point in points {
                        min = min.min(point[0]);
                        max = max.max(point[0]);
                        assert!((0.0..=10.0).contains(&point[1]));
                    }
                }
                Ok(points.map(|p| {
                    let p = exedra_math::narrow([p[0], p[1], 0.0]);
                    [p[0], p[1]]
                }))
            },
        )
        .unwrap();
    assert_eq!(calls, charted.mesh_faces.len());
    assert_eq!(min, 0.0);
    assert!((max - 4.0 * core::f64::consts::PI).abs() < 1e-12);
    assert_eq!(
        plain
            .mesh
            .vertices()
            .map(|v| *plain.mesh.vertex_position(v).unwrap())
            .collect::<Vec<_>>(),
        charted
            .mesh
            .vertices()
            .map(|v| *charted.mesh.vertex_position(v).unwrap())
            .collect::<Vec<_>>()
    );
    assert!(matches!(
        loft.realize_with_uvs(
            &p,
            0,
            &[0.0, 0.0],
            |_, _| 0,
            |_, _| -> Result<_, usize> { panic!("invalid metric must fail before mapping") }
        ),
        Err(LoftUvError::Metric)
    ));
    assert!(matches!(
        loft.realize_with_uvs(
            &p,
            0,
            &[0.0, 10.0],
            |_, _| 0,
            |_, _| -> Result<_, usize> { Err(17) }
        ),
        Err(LoftUvError::Mapping { error: 17, .. })
    ));
    assert!(matches!(
        loft.realize_with_uvs(
            &p,
            0,
            &[0.0, 10.0],
            |_, _| 0,
            |_, _| -> Result<_, usize> { Ok([[f32::NAN; 2]; 3]) }
        ),
        Err(LoftUvError::NonFiniteUv { .. })
    ));
    assert!(matches!(
        loft.realize_with_uvs(
            &p,
            99,
            &[0.0, 10.0],
            |_, _| 0,
            |_, _| -> Result<_, usize> { Ok([[0.0; 2]; 3]) }
        ),
        Err(LoftUvError::Metric)
    ));
}
