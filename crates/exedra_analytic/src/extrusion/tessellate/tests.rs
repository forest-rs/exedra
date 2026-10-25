// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::extrusion::{Boundary, EdgeRole, FaceRole, Level};
use alloc::vec;
use core::f64::consts::{PI, TAU};
use exedra_profile::LoopRef;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

const SEPARATION: ProfilePolicy = ProfilePolicy::new(1e-9);

fn circle(center: [f64; 2], radius: f64) -> Loop {
    let east = [center[0] + radius, center[1]];
    let west = [center[0] - radius, center[1]];
    Loop::new(vec![Segment::arc(west, 1.0), Segment::arc(east, 1.0)]).unwrap()
}

fn rect(min: [f64; 2], max: [f64; 2]) -> Loop {
    Loop::new(vec![
        Segment::line([max[0], min[1]]),
        Segment::line(max),
        Segment::line([min[0], max[1]]),
        Segment::line(min),
    ])
    .unwrap()
}

fn rounded(size: [f64; 2], radius: f64) -> Loop {
    let q = libm::tan(PI / 8.0);
    let [w, h] = size;
    Loop::new(vec![
        Segment::line([w - radius, 0.0]),
        Segment::arc([w, radius], q),
        Segment::line([w, h - radius]),
        Segment::arc([w - radius, h], q),
        Segment::line([radius, h]),
        Segment::arc([0.0, h - radius], q),
        Segment::line([0.0, radius]),
        Segment::arc([radius, 0.0], q),
    ])
    .unwrap()
}

fn panel() -> Profile {
    Profile::new(
        rect([0.0, 0.0], [10.0, 4.0]),
        vec![
            circle([2.0, 2.0], 1.0).reversed(),
            rect([4.0, 1.0], [6.0, 3.0]).reversed(),
            circle([8.0, 2.0], 0.5).reversed(),
        ],
        &SEPARATION,
    )
    .unwrap()
}

fn fixtures() -> Vec<(&'static str, Profile)> {
    let major = libm::tan(3.0 * PI / 8.0);
    let mixed = Loop::new(vec![
        Segment::line([2.0, 0.0]),
        Segment::arc([2.0, 2.0], major),
        Segment::line([0.0, 2.0]),
        Segment::line([0.0, 0.0]),
    ])
    .unwrap();
    vec![
        (
            "disk",
            Profile::new(circle([0.0, 0.0], 2.0), vec![], &SEPARATION).unwrap(),
        ),
        (
            "annulus",
            Profile::new(
                circle([0.0, 0.0], 2.0),
                vec![circle([0.0, 0.0], 1.0).reversed()],
                &SEPARATION,
            )
            .unwrap(),
        ),
        (
            "rounded",
            Profile::new(rounded([6.0, 4.0], 1.0), vec![], &SEPARATION).unwrap(),
        ),
        ("mixed", Profile::new(mixed, vec![], &SEPARATION).unwrap()),
        ("panel", panel()),
    ]
}

fn position(mesh: &Mesh, vertex: exedra_mesh::VertexId) -> [f64; 3] {
    exedra_math::promote(*mesh.vertex_position(vertex).unwrap())
}

/// Volume by the divergence theorem over fan-split faces.
fn volume(mesh: &Mesh) -> f64 {
    mesh.faces()
        .map(|face| {
            let corners: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .map(|half_edge| position(mesh, mesh.to_vertex(half_edge).unwrap()))
                .collect();
            (1..corners.len() - 1)
                .map(|i| {
                    exedra_math::dot(corners[0], exedra_math::cross(corners[i], corners[i + 1]))
                        / 6.0
                })
                .sum::<f64>()
        })
        .sum()
}

/// Exact area of the polygon through the samples the report counts.
fn sampled_area(profile: &Profile, report: &TessellationReport) -> f64 {
    let mut counts = report.segment_edges.iter();
    let mut area = 0.0;
    for loop_ in core::iter::once(profile.outer()).chain(profile.holes()) {
        let mut points = Vec::new();
        for (index, (from, _)) in loop_.iter_with_starts().enumerate() {
            let count = *counts.next().unwrap();
            match loop_.arc(index) {
                Some(arc) => {
                    points
                        .extend((0..count).map(|i| arc.point_at(f64::from(i) / f64::from(count))));
                }
                None => points.push(from),
            }
        }
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            area += 0.5 * (a[0] * b[1] - a[1] * b[0]);
        }
    }
    area
}

#[test]
fn required_fixtures_realize_closed_valid_meshes_with_independent_volumes() {
    for (name, profile) in fixtures() {
        let solid = Extrusion::new(profile.clone(), Placement3::IDENTITY, 3.0).unwrap();
        let realized = solid.tessellate(&TessellationPolicy::default()).unwrap();
        let mesh = &realized.mesh;
        assert!(mesh.validate_deep().is_empty(), "{name}");
        assert!(
            mesh.boundary_loops().unwrap().is_empty(),
            "{name} is closed"
        );
        assert!(
            realized.report.within_tolerance(),
            "{name}: {:?}",
            realized.report
        );
        // The sampled polygon's exact area times the length is an oracle
        // independent of the cap triangulation and wall assembly.
        let expected = 3.0 * sampled_area(&profile, &realized.report);
        assert!(
            (volume(mesh) - expected).abs() <= 1e-5 * expected,
            "{name}: {} != {expected}",
            volume(mesh)
        );
    }
}

#[test]
fn rigid_frames_keep_volume_and_repeat_bit_for_bit() {
    let frame = Placement3::rotate_x_then_translate(0.7, 2.0, -3.0, 5.0)
        * Placement3::rotate_z_then_translate(-1.1, 0.0, 0.0, 0.0);
    for (name, profile) in fixtures() {
        let placed = Extrusion::new(profile.clone(), frame, 3.0).unwrap();
        let first = placed.tessellate(&TessellationPolicy::default()).unwrap();
        let again = placed.tessellate(&TessellationPolicy::default()).unwrap();
        let positions = |mesh: &Mesh| -> Vec<[u32; 3]> {
            mesh.vertices()
                .map(|v| mesh.vertex_position(v).unwrap().map(f32::to_bits))
                .collect()
        };
        assert_eq!(positions(&first.mesh), positions(&again.mesh), "{name}");
        assert!(first.mesh.boundary_loops().unwrap().is_empty(), "{name}");
        let expected = 3.0 * sampled_area(&profile, &first.report);
        assert!(
            (volume(&first.mesh) - expected).abs() <= 1e-5 * expected,
            "{name}"
        );
    }
}

#[test]
fn caps_and_walls_share_each_rim_sample() {
    let solid = Extrusion::new(panel(), Placement3::IDENTITY, 2.0).unwrap();
    let realized = solid.tessellate(&TessellationPolicy::default()).unwrap();
    let native = |mesh_face| {
        realized
            .mesh_faces
            .iter()
            .find(|(face, _)| *face == mesh_face)
            .map(|(_, native)| *native)
            .unwrap()
    };
    let mesh = &realized.mesh;
    for (edge, chain) in realized.edge_chains.iter().enumerate() {
        let EdgeRole::Boundary { segment, level } = solid.edges()[edge].role else {
            continue;
        };
        let cap = FaceId(u32::from(level == Level::End));
        let wall = solid
            .faces()
            .iter()
            .position(|face| matches!(face.role, FaceRole::Wall { segment: s, .. } if s == segment))
            .map(|index| FaceId(crate::usize_to_u32(index)))
            .unwrap();
        for pair in chain.windows(2) {
            // The rim between consecutive samples borders exactly the cap and
            // the wall of the same segment.
            let half_edge = mesh
                .half_edges()
                .find(|&h| {
                    mesh.to_vertex(h) == Some(pair[1])
                        && mesh.twin(h).and_then(|t| mesh.to_vertex(t)) == Some(pair[0])
                })
                .expect("rim edge exists");
            let faces = [
                native(mesh.face(half_edge).unwrap()),
                native(mesh.face(mesh.twin(half_edge).unwrap()).unwrap()),
            ];
            assert!(faces.contains(&cap) && faces.contains(&wall), "{faces:?}");
        }
    }
    let natives = realized
        .mesh_vertices
        .iter()
        .filter(|(_, source)| matches!(source, VertexSource::Vertex(_)))
        .count();
    assert_eq!(natives, solid.vertices().len());
}

#[test]
fn tightening_the_tolerance_converges_on_the_exact_volume() {
    let profile = Profile::new(circle([0.0, 0.0], 1.0), vec![], &SEPARATION).unwrap();
    let solid = Extrusion::new(profile, Placement3::IDENTITY, 1.0).unwrap();
    let mut previous = f64::INFINITY;
    for tolerance in [1e-1, 1e-2, 1e-3, 1e-4] {
        let policy = TessellationPolicy {
            chord_tolerance: tolerance,
            ..TessellationPolicy::default()
        };
        let realized = solid.tessellate(&policy).unwrap();
        let error = PI - volume(&realized.mesh);
        assert!(error > 0.0 && error < previous, "{tolerance}: {error}");
        // An inscribed polygon with sagitta s loses at most 2 pi r s of area.
        assert!(error <= TAU * tolerance, "{tolerance}: {error}");
        assert!(realized.report.max_deviation <= tolerance);
        previous = error;
    }
}

#[test]
fn far_placements_report_an_unmet_tolerance() {
    let profile = Profile::new(rounded([0.06, 0.04], 0.01), vec![], &SEPARATION).unwrap();
    let policy = TessellationPolicy {
        chord_tolerance: 1e-4,
        ..TessellationPolicy::default()
    };
    let near = Extrusion::new(profile.clone(), Placement3::IDENTITY, 0.01)
        .unwrap()
        .tessellate(&policy)
        .unwrap();
    assert!(near.report.within_tolerance());
    let far = Extrusion::new(profile, Placement3::translate(1.0e6, 0.0, 0.0), 0.01)
        .unwrap()
        .tessellate(&policy);
    // Narrowing at 1e6 moves vertices by up to half an f32 ulp there,
    // far beyond the requested tolerance; the mesh is still valid.
    let far = far.unwrap();
    assert!(far.mesh.validate_deep().is_empty());
    assert!(!far.report.within_tolerance());
    assert!(far.report.max_vertex_displacement > 1e-3);
}

#[test]
fn boundaries_that_meet_in_f32_are_refused() {
    // Valid in f64, but the two holes become coincident in f32.
    let profile = Profile::new(
        rect([0.0, 0.0], [1000.0, 10.0]),
        vec![
            rect([400.0, 2.0], [500.0, 8.0]).reversed(),
            rect([500.0 + 1e-9, 2.0], [600.0, 8.0]).reversed(),
        ],
        &ProfilePolicy::new(1e-10),
    )
    .unwrap();
    let result = Extrusion::new(profile, Placement3::IDENTITY, 1.0)
        .unwrap()
        .tessellate(&TessellationPolicy::default());
    assert!(
        matches!(
            result,
            Err(TessellateExtrusionError::Arrangement {
                error: TriError::BoundaryContact { .. },
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn a_hole_beside_an_unsampled_extremum_refines_the_arc() {
    // A shallow outer arc whose few chords would cut across a small hole
    // close to its extremum.
    let outer = Loop::new(vec![
        Segment::line([10.0, 0.0]),
        Segment::arc([0.0, 0.0], 0.25),
    ])
    .unwrap();
    let arc = outer.arc(1).unwrap();
    let extremum_y = arc.center[1] + arc.radius;
    let hole = circle([5.0, extremum_y - 0.03], 0.01).reversed();
    let profile = Profile::new(outer, vec![hole], &SEPARATION).unwrap();
    let policy = TessellationPolicy {
        chord_tolerance: 0.5,
        // Three chords leave the extremum unsampled and pass below the hole.
        min_arc_edges: 3,
        ..TessellationPolicy::default()
    };
    let realized = Extrusion::new(profile, Placement3::IDENTITY, 1.0)
        .unwrap()
        .tessellate(&policy)
        .unwrap();
    assert_eq!(realized.report.refinements, 1, "{:?}", realized.report);
    assert!(realized.mesh.boundary_loops().unwrap().is_empty());
}

#[test]
fn invalid_policies_and_budgets_are_refused() {
    let solid = Extrusion::new(fixtures().remove(0).1, Placement3::IDENTITY, 1.0).unwrap();
    for policy in [
        TessellationPolicy {
            chord_tolerance: 0.0,
            ..TessellationPolicy::default()
        },
        TessellationPolicy {
            min_arc_edges: 0,
            ..TessellationPolicy::default()
        },
        TessellationPolicy {
            min_arc_edges: 9,
            max_segment_edges: 8,
            ..TessellationPolicy::default()
        },
    ] {
        assert_eq!(
            solid.tessellate(&policy).unwrap_err(),
            TessellateExtrusionError::InvalidPolicy
        );
    }
    let tight = TessellationPolicy {
        chord_tolerance: 1e-12,
        max_segment_edges: 64,
        ..TessellationPolicy::default()
    };
    assert!(matches!(
        solid.tessellate(&tight),
        Err(TessellateExtrusionError::Sampling { .. })
    ));
    let few = TessellationPolicy {
        max_vertices: 8,
        ..TessellationPolicy::default()
    };
    assert!(matches!(
        solid.tessellate(&few),
        Err(TessellateExtrusionError::VertexBudget { .. })
    ));
}

#[test]
fn caller_regions_group_faces_without_losing_correspondence() {
    let solid = Extrusion::new(panel(), Placement3::IDENTITY, 2.0).unwrap();
    // Holes' walls share region 9; everything else keeps its face index.
    let region = |face: FaceId, native: &Face| match native.role {
        FaceRole::Wall { segment, .. } if segment.boundary != Boundary::Profile(LoopRef::Outer) => {
            9
        }
        _ => face.0,
    };
    let realized = solid
        .tessellate_with_regions(&TessellationPolicy::default(), region)
        .unwrap();
    let regions = realized
        .mesh
        .attrs()
        .dense(exedra_mesh::attr::FACE_REGION)
        .unwrap();
    for (mesh_face, native) in &realized.mesh_faces {
        let expected = region(*native, &solid.faces()[native.index()]);
        assert_eq!(regions.get(mesh_face.as_id()), Some(&expected));
    }
    let hole_walls = realized
        .mesh_faces
        .iter()
        .filter(|(mesh_face, _)| regions.get(mesh_face.as_id()) == Some(&9))
        .count();
    assert!(hole_walls > 3, "every hole's walls share one region");
    // The geometry is the same as with native regions.
    let plain = solid.tessellate(&TessellationPolicy::default()).unwrap();
    assert_eq!(plain.report, realized.report);
}

#[test]
fn axial_narrowing_error_counts_against_the_tolerance() {
    // A radius-one hole far along the extrusion axis: f32 moves every
    // vertex about 0.03 in z, on top of the in-plane chord error.
    let profile = Profile::new(
        rect([-3.0, -3.0], [3.0, 3.0]),
        vec![circle([0.0, 0.0], 1.0).reversed()],
        &SEPARATION,
    )
    .unwrap();
    let far = Placement3::translate(0.0, 0.0, 1_000_000.03);
    let solid = Extrusion::new(profile, far, 1.0).unwrap();
    let policy = TessellationPolicy {
        chord_tolerance: 0.031,
        ..TessellationPolicy::default()
    };
    let realized = solid.tessellate(&policy).unwrap();
    assert!(!realized.report.within_tolerance(), "{:?}", realized.report);
    // The bound covers the farthest emitted chord midpoint from the hole's
    // rim, measured independently in 3D.
    let mesh = &realized.mesh;
    let mut worst = 0.0_f64;
    for (edge, chain) in realized.edge_chains.iter().enumerate() {
        let EdgeRole::Boundary { segment, level } = solid.edges()[edge].role else {
            continue;
        };
        if segment.boundary != Boundary::Profile(LoopRef::Hole(0)) {
            continue;
        }
        let height = if level == Level::Start { 0.0 } else { 1.0 };
        for pair in chain.windows(2) {
            let [a, b] = [pair[0], pair[1]].map(|v| {
                let p = exedra_math::promote(*mesh.vertex_position(v).unwrap());
                [p[0], p[1], p[2] - 1_000_000.03]
            });
            let mid = [
                0.5 * (a[0] + b[0]),
                0.5 * (a[1] + b[1]),
                0.5 * (a[2] + b[2]),
            ];
            let radial = libm::hypot(mid[0], mid[1]) - 1.0;
            worst = worst.max(libm::hypot(radial, mid[2] - height));
        }
    }
    assert!(worst > 0.031, "{worst}");
    assert!(
        realized.report.max_deviation >= worst,
        "{:?} < {worst}",
        realized.report
    );
}
