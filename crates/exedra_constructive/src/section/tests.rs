// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::ir::LoftSection;
use crate::ir::PathClosure;
use alloc::collections::BTreeMap;
use exedra_math::{cross, dot, scale, sub};
use exedra_mesh::{FaceTriangulation, MeshBuilder};

fn signed_area(points: &[[f64; 2]]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
        .sum::<f64>()
        * 0.5
}
use super::*;
use crate::ir::{CapMode, LoftPolicy};
use crate::tessellate::{EvalPolicy, tessellate_extrude, tessellate_loft};

fn volume(body: &TessellatedBody) -> f64 {
    let mesh = &body.mesh;
    let mut triangles = Vec::new();
    let mut result = 0.0;
    for face in mesh.faces() {
        assert!(
            !mesh.face_triangles_into(face, FaceTriangulation::Robust, &mut triangles),
            "cannot triangulate {face:?}, {:?}: {:?}",
            body.source_map.face_feature(face),
            mesh.face_loop(face)
                .map(|corner| mesh
                    .vertex_position(mesh.to_vertex(corner).unwrap())
                    .unwrap())
                .collect::<Vec<_>>()
        );
        for triangle in &triangles {
            let p = triangle.map(|c| {
                mesh.vertex_position(mesh.to_vertex(c).unwrap())
                    .unwrap()
                    .map(f64::from)
            });
            result += dot(p[0], cross(p[1], p[2])) / 6.0;
        }
    }
    result
}
fn block() -> TessellatedBody {
    tessellate_extrude(
        &crate::builders::rect_from_corner(2.0, 3.0).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap()
}

#[test]
fn oblique_section_profiles_retain_geometry_frame_and_sources() {
    use crate::profile::SegKind;
    use crate::profile_section::profiles_from_mesh_section;
    let body = block();
    let plane = Plane3 {
        normal: [0.3, -0.2, 1.0],
        distance: 1.71,
    };
    let section = section_body(&body, plane, &SectionPolicy::default()).unwrap();
    let mesh_section =
        exedra_mesh_ops::section::section_mesh(&body.mesh, plane, &SectionPolicy::default())
            .unwrap();
    let converted = section.to_profiles().unwrap().remove(0);
    let direct = profiles_from_mesh_section(&mesh_section).unwrap().remove(0);
    assert_eq!(converted.profile, direct.profile);
    assert_eq!(converted.placement, section.frame);
    assert_eq!(direct.placement, section.frame);
    // Section samples include triangulation-diagonal crossings. Keep all of them.
    let boundary = &section.regions[0].outer;
    assert!(boundary.points.len() > 4);
    assert_eq!(
        converted.profile.outer().segs().len(),
        boundary.points.len()
    );
    for (i, (start, segment)) in converted.profile.outer().iter_with_starts().enumerate() {
        assert_eq!([start.x, start.y], boundary.points[i]);
        assert_eq!(
            [segment.to.x, segment.to.y],
            boundary.points[(i + 1) % boundary.points.len()]
        );
        assert_eq!(segment.kind, SegKind::Line);
        let tag = segment.tag.unwrap();
        assert_eq!(converted.source(tag), Some(&boundary.edge_features[i]));
        assert_eq!(
            converted.source(tag).copied(),
            body.source_map.face_feature(*direct.source(tag).unwrap())
        );
    }
    let expected_volume = section.measure().unwrap().area * 0.75;
    drop(body);
    drop(section);
    let extruded = tessellate_extrude(
        &converted.profile,
        &converted.placement,
        0.75,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert!(extruded.mesh.validate_deep().is_empty());
    assert!(extruded.mesh.boundary_loops().unwrap().is_empty());
    assert!((volume(&extruded) - expected_volume).abs() < 1e-5);
    let (normal, distance) = plane.normalized().unwrap();
    for vertex in extruded.mesh.vertices() {
        let point = extruded
            .mesh
            .vertex_position(vertex)
            .unwrap()
            .map(f64::from);
        let height = dot(normal, point) - distance;
        assert!(height.abs() < 1e-6 || (height - 0.75).abs() < 1e-6);
    }
}

#[test]
fn section_profiles_preserve_holes_disconnected_regions_and_nested_islands() {
    let ring = tessellate_extrude(
        &crate::builders::ring(2.0, 0.7).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let island = tessellate_extrude(
        &crate::builders::rect_from_corner(0.4, 0.3).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let source = combine(&[(&ring, 0.0), (&island, 0.0), (&block(), 6.0)]);
    for normal in [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
        let section = section_body(
            &source,
            Plane3 {
                normal,
                distance: normal[2] * 1.37,
            },
            &SectionPolicy::default(),
        )
        .unwrap();
        let profiles = section.to_profiles().unwrap();
        assert_eq!(profiles.len(), 3);
        assert_eq!(
            profiles
                .iter()
                .map(|p| p.profile.holes().len())
                .sum::<usize>(),
            1
        );
        let mut total_volume = 0.0;
        for (converted, region) in profiles.iter().zip(&section.regions) {
            assert_eq!(converted.placement, section.frame);
            assert_eq!(converted.profile.holes().len(), region.holes.len());
            let boundaries = core::iter::once(&region.outer).chain(&region.holes);
            let loops =
                core::iter::once(converted.profile.outer()).chain(converted.profile.holes());
            let mut tags = alloc::collections::BTreeSet::new();
            for (boundary, profile_loop) in boundaries.zip(loops) {
                assert_eq!(profile_loop.segs().len(), boundary.points.len());
                for ((start, segment), (point, source)) in profile_loop
                    .iter_with_starts()
                    .zip(boundary.points.iter().zip(&boundary.edge_features))
                {
                    assert_eq!([start.x, start.y], *point);
                    let tag = segment.tag.unwrap();
                    assert!(tags.insert(tag.0));
                    assert_eq!(converted.source(tag), Some(source));
                }
            }
            assert_eq!(tags.len(), converted.segment_sources.len());
            let body = tessellate_extrude(
                &converted.profile,
                &converted.placement,
                0.6,
                CapMode::Both,
                &EvalPolicy::default(),
            )
            .unwrap();
            assert!(body.mesh.validate_deep().is_empty());
            assert!(body.mesh.boundary_loops().unwrap().is_empty());
            body.source_map.check(&body.mesh).unwrap();
            assert!(
                (volume(&body) - crate::builders::profile_area(&converted.profile) * 0.6).abs()
                    < 1e-5
            );
            total_volume += volume(&body);
        }
        assert!((total_volume - section.measure().unwrap().area * 0.6).abs() < 1e-5);
    }
}

#[test]
fn section_profiles_sweep_with_connected_caps() {
    use crate::path::PathSegment3;
    use crate::tessellate::{tessellate_curved_sweep, tessellate_mitered_sweep, tessellate_sweep};

    let policy = EvalPolicy::default();
    let annulus = tessellate_extrude(
        &crate::builders::ring(2.0, 0.7).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &policy,
    )
    .unwrap();
    for source in [annulus, block()] {
        let section = section_body(
            &source,
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 1.371,
            },
            &SectionPolicy::default(),
        )
        .unwrap();
        let converted = section.to_profiles().unwrap().remove(0);
        let boundary_count = 1 + converted.profile.holes().len();
        for reflected in [false, true] {
            let mut placement = converted.placement;
            if reflected {
                for row in &mut placement.rows {
                    row[0] = -row[0];
                }
            }
            for (caps, open_ends) in [
                (CapMode::Both, 0),
                (CapMode::Start, 1),
                (CapMode::End, 1),
                (CapMode::None, 2),
            ] {
                for kind in ["polyline", "mitered", "curved"] {
                    let path = [[0.0; 3], [0.0, 0.0, 0.6]];
                    let body = match kind {
                        "polyline" => {
                            tessellate_sweep(&converted.profile, &placement, &path, caps, &policy)
                        }
                        "mitered" => tessellate_mitered_sweep(
                            &converted.profile,
                            &placement,
                            &path,
                            [1.0, 0.0, 0.0],
                            [0.0; 2],
                            PathClosure::Open,
                            4.0,
                            caps,
                            &policy,
                        ),
                        "curved" => tessellate_curved_sweep(
                            &converted.profile,
                            &placement,
                            [0.0; 3],
                            &[PathSegment3::Arc {
                                axis_origin: [10.0, 0.0, 0.0],
                                axis: [0.0, 1.0, 0.0],
                                sweep: core::f64::consts::FRAC_PI_6,
                            }],
                            [1.0, 0.0, 0.0],
                            caps,
                            &policy,
                        ),
                        _ => unreachable!(),
                    }
                    .unwrap();
                    assert!(body.mesh.validate_deep().is_empty());
                    body.source_map.check(&body.mesh).unwrap();
                    assert_eq!(
                        body.mesh.boundary_loops().unwrap().len(),
                        boundary_count * open_ends,
                        "{kind}, {caps:?}, reflected={reflected}",
                    );
                    if caps == CapMode::Both {
                        if kind == "curved" {
                            assert!(volume(&body) > 0.0);
                        } else {
                            let expected = section.measure().unwrap().area * 0.6;
                            assert!((volume(&body) - expected).abs() < 1e-5);
                        }
                    }
                    for face in body.mesh.faces() {
                        let tangent = match body.source_map.face_feature(face).unwrap() {
                            Feature::CapStart => [0.0, 0.0, -1.0],
                            Feature::CapEnd if kind == "curved" => {
                                [0.5, 0.0, libm::cos(core::f64::consts::FRAC_PI_6)]
                            }
                            Feature::CapEnd => [0.0, 0.0, 1.0],
                            _ => continue,
                        };
                        let outward = placement.rows.map(|r| dot([r[0], r[1], r[2]], tangent));
                        let mut triangles = Vec::new();
                        assert!(!body.mesh.face_triangles_into(
                            face,
                            FaceTriangulation::Robust,
                            &mut triangles,
                        ));
                        for triangle in triangles {
                            let p = triangle.map(|c| {
                                body.mesh
                                    .vertex_position(body.mesh.to_vertex(c).unwrap())
                                    .unwrap()
                                    .map(f64::from)
                            });
                            assert!(dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), outward) > 0.0);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn converted_profile_can_be_retained_extruded_and_lofted() {
    use crate::evaluate::evaluate;
    use crate::ir::{NodeKind, RecipeBuilder};
    let section = section_body(
        &block(),
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 1.37,
        },
        &SectionPolicy::default(),
    )
    .unwrap();
    let converted = section.to_profiles().unwrap().remove(0);
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(converted.profile);
    let extrusion = builder
        .add(NodeKind::Extrude {
            profile,
            placement: converted.placement,
            height: 2.5,
            caps: CapMode::Both,
        })
        .unwrap();
    // Reuse one profile at two stations: correspondence is explicitly identical.
    let mut end = converted.placement;
    end.rows[2][3] += 2.5;
    let loft = builder
        .add(NodeKind::Loft {
            sections: alloc::vec![
                LoftSection::new(converted.placement, profile),
                LoftSection::new(end, profile)
            ],
            policy: LoftPolicy::Ruled,
            caps: CapMode::Both,
        })
        .unwrap();
    let group = builder
        .add(NodeKind::Group {
            children: alloc::vec![extrusion, loft],
        })
        .unwrap();
    let recipe = builder.finish(group).unwrap();
    #[cfg(feature = "serde")]
    let recipe = {
        let json = serde_json::to_string(&crate::interchange::to_dto(&recipe)).unwrap();
        let restored = crate::interchange::from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(recipe.recipe_fingerprint(), restored.recipe_fingerprint());
        restored
    };
    let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    assert_eq!(result.bodies.len(), 2);
    for placed in &result.bodies {
        assert!(placed.body.mesh.validate_deep().is_empty());
        assert!(placed.body.mesh.boundary_loops().unwrap().is_empty());
        assert!((volume(&placed.body) - 15.0).abs() < 1e-5);
    }
}
fn cap() -> CutCap {
    CutCap {
        region: 1000,
        material: Some(SlotId(7)),
    }
}
fn check_split(source: &TessellatedBody, plane: Plane3) -> PlaneSplit {
    let split = split_body(source, plane, &SectionPolicy::default(), cap()).unwrap();
    let n = split.negative.as_ref().unwrap();
    let p = split.positive.as_ref().unwrap();
    for body in [n, p] {
        assert!(body.mesh.validate_deep().is_empty());
        assert!(body.mesh.boundary_loops().unwrap().is_empty());
        body.source_map.check(&body.mesh).unwrap();
        assert!(volume(body) > 0.0);
        assert!(body.loft_sampling.is_none());
    }
    assert!(
        (volume(n) + volume(p) - volume(source)).abs() < 1e-5,
        "volumes {}, {}, {}",
        volume(n),
        volume(p),
        volume(source)
    );
    let mut rims = Vec::new();
    for body in [n, p] {
        let mut positions: Vec<_> = body
            .mesh
            .vertices()
            .filter(|&v| body.source_map.vertex_feature(v) == Some(Feature::PlaneCutSeam))
            .map(|v| body.mesh.vertex_position(v).unwrap().map(f32::to_bits))
            .collect();
        positions.sort_unstable();
        rims.push(positions);
        let regions = body
            .mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .unwrap();
        for face in body.mesh.faces() {
            if body.source_map.face_feature(face) == Some(Feature::PlaneCutCap) {
                assert_eq!(regions.get(face.into()), Some(&cap().region));
                assert_eq!(body.face_materials.get(&face), cap().material.as_ref());
                let corners: Vec<_> = body.mesh.face_loop(face).collect();
                let points: Vec<_> = corners
                    .iter()
                    .map(|&c| {
                        body.mesh
                            .vertex_position(body.mesh.to_vertex(c).unwrap())
                            .unwrap()
                            .map(f64::from)
                    })
                    .collect();
                let actual = cross(sub(points[1], points[0]), sub(points[2], points[0]));
                let expected = if core::ptr::eq(body, n) {
                    plane.normal
                } else {
                    scale(plane.normal, -1.0)
                };
                assert!(dot(actual, expected) > 0.0);
            }
        }
    }
    assert_eq!(rims[0], rims[1]);
    split
}

#[test]
fn oblique_cut_is_closed_conserves_volume_and_attributes_caps() {
    let source = block();
    let plane = Plane3 {
        normal: [0.3, -0.2, 1.0],
        distance: 1.71,
    };
    let split = check_split(&source, plane);
    assert_eq!(split.section.regions.len(), 1);
    assert!(split.section.regions[0].holes.is_empty());
    assert_eq!(split.section.stats.input_triangles, 12);
    assert!(split.section.stats.cap_triangles > 0);
    let mut section = section_body(&source, plane, &SectionPolicy::default()).unwrap();
    section.stats.cap_triangles = split.section.stats.cap_triangles;
    assert_eq!(section, split.section);
    let again = split_body(&source, plane, &SectionPolicy::default(), cap()).unwrap();
    assert_eq!(again.section, split.section);
    assert_eq!(
        again
            .negative
            .unwrap()
            .mesh
            .to_trimesh(&exedra_mesh::ExtractParams::default())
            .0
            .positions,
        split
            .negative
            .unwrap()
            .mesh
            .to_trimesh(&exedra_mesh::ExtractParams::default())
            .0
            .positions
    );
}

#[test]
fn holed_section_retains_collinear_diagonal_crossings_and_opposite_cap_winding() {
    let source = tessellate_extrude(
        &crate::builders::ring(2.0, 0.7).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    for normal in [[0.0, 0.0, 1.0], [0.0, 0.0, -3.0]] {
        let split = check_split(
            &source,
            Plane3 {
                normal,
                distance: normal[2] * 1.371,
            },
        );
        assert_eq!(split.section.regions.len(), 1);
        assert_eq!(split.section.regions[0].holes.len(), 1);
        assert!(signed_area(&split.section.regions[0].outer.points) > 0.0);
        assert!(signed_area(&split.section.regions[0].holes[0].points) < 0.0);
        assert_eq!(split.section.stats.section_loops, 2);
    }
}

#[test]
fn asymmetric_smooth_loft_supports_oblique_cuts() {
    let profiles = [
        crate::builders::rect_from_corner(1.0, 0.7).unwrap(),
        crate::builders::rect_from_corner(1.5, 0.9).unwrap(),
        crate::builders::rect_from_corner(0.8, 1.1).unwrap(),
    ];
    let sections = [
        (Placement3::IDENTITY, &profiles[0]),
        (Placement3::translate(0.4, 0.1, 1.5), &profiles[1]),
        (Placement3::translate(-0.1, 0.2, 3.0), &profiles[2]),
    ];
    let source = tessellate_loft(
        &sections,
        LoftPolicy::Smooth,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert!(source.loft_sampling.is_some());
    check_split(
        &source,
        Plane3 {
            normal: [0.35, -0.17, 1.0],
            distance: 1.337,
        },
    );
}

#[test]
fn contacts_invalid_inputs_and_budgets_are_explicit() {
    let source = block();
    for distance in [0.0, 4.0, 1e-7] {
        assert!(matches!(
            section_body(
                &source,
                Plane3 {
                    normal: [0.0, 0.0, 1.0],
                    distance
                },
                &SectionPolicy::default()
            ),
            Err(SectionError::AmbiguousContact)
        ));
    }
    let plane = Plane3 {
        normal: [0.0, 0.0, 1.0],
        distance: 1.3,
    };
    for policy in [
        SectionPolicy {
            max_triangles: 1,
            ..Default::default()
        },
        SectionPolicy {
            max_section_vertices: 1,
            ..Default::default()
        },
        SectionPolicy {
            max_pair_checks: 1,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            split_body(&source, plane, &policy, cap()),
            Err(SectionError::BudgetExceeded)
        ));
    }
    assert!(matches!(
        section_body(
            &source,
            plane,
            &SectionPolicy {
                distance_tolerance: f64::NAN,
                ..Default::default()
            }
        ),
        Err(SectionError::InvalidPolicy)
    ));
    assert!(matches!(
        section_body(
            &source,
            Plane3 {
                normal: [0.0; 3],
                distance: 0.0
            },
            &SectionPolicy::default()
        ),
        Err(SectionError::InvalidPolicy)
    ));
    let open = tessellate_extrude(
        &crate::builders::rect_from_corner(2.0, 3.0).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert!(matches!(
        section_body(&open, plane, &SectionPolicy::default()),
        Err(SectionError::InvalidMesh)
    ));
}

#[test]
fn disjoint_plane_returns_one_empty_half() {
    let source = block();
    for distance in [-1.0, 5.0] {
        let split = split_body(
            &source,
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance,
            },
            &SectionPolicy::default(),
            cap(),
        )
        .unwrap();
        assert!(split.section.regions.is_empty());
        assert_eq!(split.negative.is_none(), distance < 0.0);
        assert_eq!(split.positive.is_none(), distance > 4.0);
        assert!(
            (volume(split.negative.as_ref().or(split.positive.as_ref()).unwrap()) - 24.0).abs()
                < 1e-6
        );
    }
}

fn combine(bodies: &[(&TessellatedBody, f32)]) -> TessellatedBody {
    let mut builder = MeshBuilder::new();
    for &(block, x) in bodies {
        let mut ids = BTreeMap::new();
        for v in block.mesh.vertices() {
            let p = block.mesh.vertex_position(v).unwrap();
            ids.insert(v, builder.push_vertex([p[0] + x, p[1], p[2]]));
        }
        for face in block.mesh.faces() {
            let corners: Vec<_> = block
                .mesh
                .face_loop(face)
                .map(|e| ids[&block.mesh.to_vertex(e).unwrap()])
                .collect();
            builder.add_face(&corners).unwrap();
        }
    }
    let built = builder.build().unwrap();
    let source_map = crate::source_map::SourceMap::new(
        &built.mesh,
        alloc::vec![Feature::Imported;built.mesh.faces().count()],
        alloc::vec![Feature::Imported;built.mesh.vertices().count()],
    );
    TessellatedBody {
        mesh: built.mesh,
        source_map,
        face_materials: BTreeMap::new(),
        sweep_checks: None,
        path_sampling: None,
        loft_sampling: None,
        refinement: None,
    }
}
#[test]
fn disconnected_sections_remain_separate_regions() {
    let split = check_split(
        &combine(&[(&block(), 0.0), (&block(), 5.0)]),
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 1.3,
        },
    );
    assert_eq!(split.section.regions.len(), 2);
    assert!(split.section.regions.iter().all(|r| r.holes.is_empty()));
}

#[test]
fn concave_profile_can_produce_disconnected_sections() {
    let outer = crate::profile::Loop2::new(
        [
            (0.0, 0.0),
            (3.0, 0.0),
            (3.0, 1.0),
            (1.0, 1.0),
            (1.0, 3.0),
            (0.0, 3.0),
        ]
        .into_iter()
        .map(crate::profile::Seg2::line)
        .collect(),
    )
    .unwrap();
    let source = tessellate_extrude(
        &crate::profile::Profile2::simple(outer).unwrap(),
        &Placement3::IDENTITY,
        2.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let split = check_split(
        &source,
        Plane3 {
            normal: [1.0, 1.0, 0.0],
            distance: 2.6,
        },
    );
    assert_eq!(split.section.regions.len(), 2);
}

#[test]
fn surface_attributes_survive_clipping_and_source_evidence_is_invalidated() {
    let mut source = block();
    let vertices: Vec<_> = source.mesh.vertices().collect();
    let corners: Vec<_> = source
        .mesh
        .faces()
        .flat_map(|face| source.mesh.face_loop(face))
        .collect();
    let uv: Vec<_> = corners
        .iter()
        .map(|&c| {
            let p = source
                .mesh
                .vertex_position(source.mesh.to_vertex(c).unwrap())
                .unwrap();
            [p[0], p[2]]
        })
        .collect();
    {
        let mut edit = source.mesh.edit();
        for &v in &vertices {
            exedra_mesh::op::set_vertex_sharpness(&mut edit, v, 0.25).unwrap();
        }
        for (&c, uv) in corners.iter().zip(uv) {
            exedra_mesh::op::set_corner_uv(&mut edit, c, uv).unwrap();
            exedra_mesh::op::set_corner_normal_override(&mut edit, c, Some([0.0, 0.0, 1.0]))
                .unwrap();
            exedra_mesh::op::set_edge_sharpness(&mut edit, c, 0.7).unwrap();
            exedra_mesh::op::set_edge_seam(&mut edit, c, true).unwrap();
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }
    source.source_map = source.source_map.repinned(&source.mesh);
    source.face_materials = source.mesh.faces().map(|f| (f, SlotId(3))).collect();
    let split = check_split(
        &source,
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 1.3,
        },
    );
    for body in [split.negative.unwrap(), split.positive.unwrap()] {
        let uvs = body
            .mesh
            .attrs()
            .sparse(exedra_mesh::attr::CORNER_UV)
            .unwrap();
        let normals = body
            .mesh
            .attrs()
            .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
            .unwrap();
        for face in body.mesh.faces() {
            if body.source_map.face_feature(face) == Some(Feature::PlaneCutCap) {
                continue;
            }
            assert_eq!(body.face_materials.get(&face), Some(&SlotId(3)));
            for edge in body.mesh.face_loop(face) {
                let vertex = body.mesh.to_vertex(edge).unwrap();
                let p = body.mesh.vertex_position(vertex).unwrap();
                let uv = uvs.get(edge.into()).unwrap();
                assert!((uv[0] - p[0]).abs() < 1e-6 && (uv[1] - p[2]).abs() < 1e-6);
                assert_eq!(normals.get(edge.into()), Some(&[0.0, 0.0, 1.0]));
                if body.source_map.vertex_feature(vertex) != Some(Feature::PlaneCutSeam) {
                    assert_eq!(body.mesh.vertex_sharpness(vertex), Some(0.25));
                }
            }
        }
        assert!(
            body.mesh
                .faces()
                .flat_map(|f| body.mesh.face_loop(f))
                .any(|e| body.mesh.edge_seam(e) == Some(true))
        );
    }
}

#[test]
fn unattainable_plane_accuracy_and_stale_sources_are_refused() {
    let mut source = block();
    assert!(matches!(
        split_body(
            &source,
            Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 1.0 / 3.0
            },
            &SectionPolicy {
                distance_tolerance: 1e-12,
                ..Default::default()
            },
            cap()
        ),
        Err(SectionError::NumericLimit)
    ));
    let vertex = source.mesh.vertices().next().unwrap();
    {
        let mut edit = source.mesh.edit();
        exedra_mesh::op::set_vertex_position(&mut edit, vertex, [0.01, 0.0, 0.0]).unwrap();
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }
    assert!(matches!(
        section_body(
            &source,
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 1.3
            },
            &SectionPolicy::default()
        ),
        Err(SectionError::StaleSourceMap)
    ));
}

#[test]
fn nested_island_is_separate_from_the_surrounding_holed_region() {
    let tube = tessellate_extrude(
        &crate::builders::ring(2.0, 0.7).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let island = tessellate_extrude(
        &crate::builders::circle(0.3).unwrap(),
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let split = check_split(
        &combine(&[(&tube, 0.0), (&island, 0.0)]),
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 1.3,
        },
    );
    assert_eq!(split.section.regions.len(), 2);
    assert_eq!(
        split
            .section
            .regions
            .iter()
            .map(|r| r.holes.len())
            .sum::<usize>(),
        1
    );
}

#[test]
fn intersecting_section_loops_are_refused_even_when_topology_is_closed() {
    let source = combine(&[(&block(), 0.0), (&block(), 1.0)]);
    assert!(source.mesh.validate_deep().is_empty());
    assert!(matches!(
        split_body(
            &source,
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 1.3
            },
            &SectionPolicy::default(),
            cap()
        ),
        Err(SectionError::InvalidSection)
    ));
}

#[test]
fn section_frame_and_edge_provenance_reconstruct_the_world_boundary() {
    let source = block();
    let plane = Plane3 {
        normal: [0.3, -0.2, 1.0],
        distance: 1.71,
    };
    let split = check_split(&source, plane);
    let body = split.negative.as_ref().unwrap();
    let frame = split.section.frame.rows;
    let n = plane.normalized().unwrap().0;
    let x = frame.map(|r| r[0]);
    let y = frame.map(|r| r[1]);
    assert!(dot(cross(x, y), n) > 1.0 - 1e-12);
    for region in &split.section.regions {
        for boundary in core::iter::once(&region.outer).chain(&region.holes) {
            assert_eq!(boundary.points.len(), boundary.edge_features.len());
            for (p, feature) in boundary.points.iter().zip(&boundary.edge_features) {
                assert!(!source.source_map.faces_for(*feature).is_empty());
                let world = frame.map(|r| r[0] * p[0] + r[1] * p[1] + r[3]);
                assert!((dot(plane.normal, world) - plane.distance).abs() < 1e-12);
                assert!(body.mesh.vertices().any(|v| {
                    let q = body.mesh.vertex_position(v).unwrap().map(f64::from);
                    exedra_math::norm(sub(world, q)) < 1e-6
                }));
            }
        }
    }
}

#[test]
fn oblique_cuts_preserve_closed_filleted_and_chamfered_bodies() {
    use crate::edge_finish::{EdgeSelection, RoundPolicy, finish_edges};
    for policy in [RoundPolicy::fillet(0.15), RoundPolicy::chamfer(0.15)] {
        let (finished, _) = finish_edges(&block(), &EdgeSelection::SharpEdges, &policy).unwrap();
        let split = check_split(
            &finished,
            Plane3 {
                normal: [0.3, -0.2, 1.0],
                distance: 1.71,
            },
        );
        assert_eq!(split.section.regions.len(), 1);
        assert!(split.section.stats.section_vertices > 4);
        if matches!(policy.kind, exedra_mesh_ops::RoundKind::Fillet { .. }) {
            for body in [split.negative.unwrap(), split.positive.unwrap()] {
                let normals = body
                    .mesh
                    .attrs()
                    .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                    .unwrap();
                assert!(
                    body.mesh
                        .faces()
                        .filter(|&f| body.source_map.face_feature(f) != Some(Feature::PlaneCutCap))
                        .flat_map(|f| body.mesh.face_loop(f))
                        .any(|c| normals.get(c.into()).is_some())
                );
            }
        }
    }
}

fn collinear_prism(bent: bool) -> TessellatedBody {
    let ring = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    let mut builder = MeshBuilder::new();
    for z in [0.0, 2.0] {
        for [x, y] in ring {
            builder.push_vertex([
                x,
                y,
                if bent && x == 1.0 && z == 2.0 {
                    2.125
                } else {
                    z
                },
            ]);
        }
    }
    builder.add_face(&[4, 3, 2, 1, 0]).unwrap();
    builder.add_face(&[5, 6, 7, 8, 9]).unwrap();
    for i in 0..5 {
        let j = (i + 1) % 5;
        builder.add_face(&[i, j, j + 5, i + 5]).unwrap();
    }
    let built = builder.build().unwrap();
    let source_map = crate::source_map::SourceMap::new(
        &built.mesh,
        alloc::vec![Feature::Imported;built.mesh.faces().count()],
        alloc::vec![Feature::Imported;built.mesh.vertices().count()],
    );
    TessellatedBody {
        mesh: built.mesh,
        source_map,
        face_materials: BTreeMap::new(),
        sweep_checks: None,
        path_sampling: None,
        loft_sampling: None,
        refinement: None,
    }
}

#[test]
fn collinear_face_boundaries_remain_closed_for_crossing_and_disjoint_planes() {
    let body = collinear_prism(false);
    for plane in [
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 0.7,
        },
        Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 0.7,
        },
    ] {
        check_split(&body, plane);
    }
    let split = split_body(
        &body,
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 3.0,
        },
        &SectionPolicy::default(),
        cap(),
    )
    .unwrap();
    assert!(split.positive.is_none());
    let negative = split.negative.unwrap();
    assert!(negative.mesh.boundary_loops().unwrap().is_empty());
    assert!((volume(&negative) - 8.0).abs() < 1e-6);
}

#[test]
fn rounding_cannot_merge_separate_section_boundaries() {
    let mut builder = MeshBuilder::new();
    for (index, (base_x, base_z)) in [(999999.0_f32, 0.0_f32), (1000000.0_f32, -0.125_f32)]
        .into_iter()
        .enumerate()
    {
        let start = u32::try_from(index * 8).unwrap();
        for z in [0.0, 1.0] {
            for [x, y] in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
                builder.push_vertex([base_x + x + 0.0625 * z, y, base_z + z]);
            }
        }
        for ring in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            builder.add_face(&ring.map(|i| i + start)).unwrap();
        }
    }
    let built = builder.build().unwrap();
    let source_map = crate::source_map::SourceMap::new(
        &built.mesh,
        alloc::vec![Feature::Imported;built.mesh.faces().count()],
        alloc::vec![Feature::Imported;built.mesh.vertices().count()],
    );
    let body = TessellatedBody {
        mesh: built.mesh,
        source_map,
        face_materials: BTreeMap::new(),
        sweep_checks: None,
        path_sampling: None,
        loft_sampling: None,
        refinement: None,
    };
    assert!(body.mesh.validate_deep().is_empty());
    // The exact sections have a 1/128 gap. At this coordinate magnitude,
    // both inner boundaries round to x=1_000_000 despite zero plane error.
    assert!(matches!(
        split_body(
            &body,
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 0.25
            },
            &SectionPolicy::default(),
            cap()
        ),
        Err(SectionError::NumericLimit)
    ));
}

#[test]
fn projected_collinearity_cannot_change_the_nonplanar_surface() {
    let body = collinear_prism(true);
    assert!(body.mesh.validate_deep().is_empty());
    assert!(matches!(
        split_body(
            &body,
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 0.7
            },
            &SectionPolicy::default(),
            cap()
        ),
        Err(SectionError::Triangulation)
    ));
}

#[test]
fn original_and_realized_boundaries_share_the_pair_budget() {
    let source = block();
    let plane = Plane3 {
        normal: [0.0, 0.0, 1.0],
        distance: 1.3,
    };
    let mut policy = SectionPolicy {
        max_pair_checks: 64,
        ..Default::default()
    };
    assert!(matches!(
        section_body(&source, plane, &policy),
        Err(SectionError::BudgetExceeded)
    ));
    policy.max_pair_checks = 128;
    let section = section_body(&source, plane, &policy).unwrap();
    assert_eq!(section.stats.section_vertices, 8);
}

#[test]
fn direct_cut_correspondence_drives_constructive_bindings() {
    use exedra_mesh_ops::section::{CutFaceSource, CutVertexSource, split_mesh};
    let mut source = block();
    source.face_materials = source.mesh.faces().map(|face| (face, SlotId(3))).collect();
    let plane = Plane3 {
        normal: [0.3, -0.2, 1.0],
        distance: 1.71,
    };
    let direct = split_mesh(&source.mesh, plane, &SectionPolicy::default(), cap().region).unwrap();
    let wrapped = split_body(&source, plane, &SectionPolicy::default(), cap()).unwrap();
    assert_eq!(direct.section.frame, wrapped.section.frame);
    assert_eq!(direct.section.stats, wrapped.section.stats);
    assert_eq!(
        direct.section.measure().unwrap(),
        wrapped.section.measure().unwrap()
    );
    for (direct, wrapped) in [
        (direct.negative.unwrap(), wrapped.negative.unwrap()),
        (direct.positive.unwrap(), wrapped.positive.unwrap()),
    ] {
        assert_eq!(
            direct
                .mesh
                .to_trimesh(&exedra_mesh::ExtractParams::default())
                .0,
            wrapped
                .mesh
                .to_trimesh(&exedra_mesh::ExtractParams::default())
                .0
        );
        for (face, origin) in direct.face_sources {
            let (feature, material) = match origin {
                CutFaceSource::Original(input) => (
                    source.source_map.face_feature(input).unwrap(),
                    Some(SlotId(3)),
                ),
                CutFaceSource::Cap => (Feature::PlaneCutCap, cap().material),
            };
            assert_eq!(wrapped.source_map.face_feature(face), Some(feature));
            assert_eq!(wrapped.face_materials.get(&face).copied(), material);
        }
        let mut saw_diagonal = false;
        for (vertex, origin) in direct.vertex_sources {
            match origin {
                CutVertexSource::Original(input) => {
                    assert_eq!(
                        direct.mesh.vertex_position(vertex),
                        source.mesh.vertex_position(input)
                    );
                    assert_eq!(
                        wrapped.source_map.vertex_feature(vertex),
                        source.source_map.vertex_feature(input)
                    );
                }
                CutVertexSource::Intersection {
                    vertices: [a, b],
                    parameter,
                } => {
                    assert!(parameter > 0.0 && parameter < 1.0);
                    let a_position = source.mesh.vertex_position(a).unwrap().map(f64::from);
                    let b_position = source.mesh.vertex_position(b).unwrap().map(f64::from);
                    let expected = exedra_math::lerp(a_position, b_position, parameter);
                    let actual = direct.mesh.vertex_position(vertex).unwrap().map(f64::from);
                    assert!(exedra_math::norm(sub(actual, expected)) < 1e-6);
                    assert_eq!(
                        wrapped.source_map.vertex_feature(vertex),
                        Some(Feature::PlaneCutSeam)
                    );
                    saw_diagonal |= !source.mesh.half_edges().any(|edge| {
                        (source.mesh.from_vertex(edge) == Some(a)
                            && source.mesh.to_vertex(edge) == Some(b))
                            || (source.mesh.from_vertex(edge) == Some(b)
                                && source.mesh.to_vertex(edge) == Some(a))
                    });
                }
            }
        }
        assert!(
            saw_diagonal,
            "input quad triangulation creates diagonal intersections too"
        );
    }
}
