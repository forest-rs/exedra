// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::builders::rect;
use crate::ir::{CapMode, Placement3};
use crate::tessellate::{EvalPolicy, REGION_CAP_END, tessellate_extrude};
use exedra_math::{cross, dot};

fn block() -> TessellatedBody {
    tessellate_extrude(
        &rect(4.0, 3.0).unwrap(),
        &Placement3::IDENTITY,
        2.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap()
}
fn workplane(body: &TessellatedBody) -> Workplane {
    face_workplane(
        body,
        WorkplaneSelection::Region(REGION_CAP_END),
        [0.0, 0.0, 2.0],
        [1.0, 0.0, 0.0],
        &WorkplanePolicy::default(),
    )
    .unwrap()
}
#[test]
fn local_coordinates_round_trip_and_offset_placement() {
    let body = block();
    let p = workplane(&body);
    assert_eq!(p.to_body([1.0, 2.0, 3.0]), [1.0, 2.0, 5.0]);
    assert_eq!(p.to_local([1.0, 2.0, 5.0]), [1.0, 2.0, 3.0]);
    assert_eq!(
        p.placement_at([1.0, 2.0, 3.0]).rows.map(|r| r[3]),
        [1.0, 2.0, 5.0]
    );
    assert_eq!(p.max_plane_deviation(), 0.0);
    p.check(&body.mesh).unwrap();
    let face = p.faces()[0];
    let selected = face_workplane(
        &body,
        WorkplaneSelection::Face(face),
        [0.0, 0.0, 2.0],
        [1.0, 0.0, 0.0],
        &WorkplanePolicy::default(),
    )
    .unwrap();
    assert_eq!(p.frame(), selected.frame());
}
#[test]
fn no_implicit_origin_or_axis_fallback() {
    let body = block();
    let select = WorkplaneSelection::Region(REGION_CAP_END);
    assert_eq!(
        face_workplane(
            &body,
            select.clone(),
            [0.0; 3],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default()
        )
        .unwrap_err()
        .kind,
        WorkplaneError::OriginOffPlane
    );
    assert_eq!(
        face_workplane(
            &body,
            select.clone(),
            [0.0, 0.0, 2.0],
            [0.0, 0.0, 1.0],
            &WorkplanePolicy::default()
        )
        .unwrap_err()
        .kind,
        WorkplaneError::ParallelAxis
    );
    let p = face_workplane(
        &body,
        select,
        [0.0, 0.0, 2.0 + 1e-7],
        [1.0, 0.0, 1.0],
        &WorkplanePolicy::default(),
    )
    .unwrap();
    assert_eq!(p.frame(), workplane(&body).frame());
}

fn patch(points: &[[f32; 3]], faces: &[&[u32]]) -> TessellatedBody {
    let mut builder = exedra_mesh::MeshBuilder::new();
    for &p in points {
        builder.push_vertex(p);
    }
    for face in faces {
        builder.add_face(face).unwrap();
    }
    let mesh = builder.build().unwrap().mesh;
    let source_map = crate::source_map::SourceMap::new(
        &mesh,
        mesh.faces().map(|_| Feature::Imported).collect(),
        mesh.vertices().map(|_| Feature::Imported).collect(),
    );
    let mut body = block();
    body.mesh = mesh;
    body.source_map = source_map;
    body
}

#[test]
fn triangulation_does_not_choose_origin_or_roll() {
    let points = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 1.0],
        [2.0, 2.0, 1.0],
        [0.0, 2.0, 0.0],
    ];
    let quad = patch(&points, &[&[0, 1, 2, 3]]);
    let triangles = patch(&points, &[&[0, 1, 2], &[0, 2, 3]]);
    let make = |body| {
        face_workplane(
            body,
            WorkplaneSelection::Region(0),
            [1.0, 1.0, 0.5],
            [1.0, 0.0, 0.5],
            &WorkplanePolicy::default(),
        )
        .unwrap()
    };
    let a = make(&quad);
    let b = make(&triangles);
    assert_eq!(a.frame(), b.frame());
    let local = [0.37, -0.51, 0.9];
    let roundtrip = a.to_local(a.to_body(local));
    for i in 0..3 {
        assert!((local[i] - roundtrip[i]).abs() < 1e-12);
    }
    let [x, y, z] = core::array::from_fn(|i| a.frame().rows.map(|r| r[i]));
    assert!(dot(cross(x, y), z) > 1.0 - 1e-12);
    assert!(dot(z, [-0.5, 0.0, 1.0]) > 0.0);
}

#[test]
fn nonplanar_degenerate_and_disconnected_selections_are_refused() {
    let bent = patch(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.1],
            [0.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    );
    let make = |body| {
        face_workplane(
            body,
            WorkplaneSelection::Region(0),
            [0.0; 3],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default(),
        )
    };
    assert_eq!(make(&bent).unwrap_err().kind, WorkplaneError::NonPlanar);
    let separate = patch(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2], &[3, 4, 5]],
    );
    assert_eq!(
        make(&separate).unwrap_err().kind,
        WorkplaneError::AmbiguousSelection
    );
    let zero = patch(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], &[&[0, 1, 2]]);
    assert!(matches!(
        make(&zero),
        Err(failure) if matches!(failure.kind, WorkplaneError::InvalidFace | WorkplaneError::InvalidGeometry)
    ));
}

#[test]
fn revisions_and_budgets_are_checked() {
    let mut body = block();
    let p = workplane(&body);
    let vertex = body.mesh.vertices().next().unwrap();
    let mut edit = body.mesh.edit();
    exedra_mesh::op::set_vertex_position(&mut edit, vertex, [0.1, 0.0, 0.0]).unwrap();
    let _: () = edit.finish();
    assert_eq!(p.check(&body.mesh), Err(WorkplaneError::StaleSource));
    assert_eq!(
        face_workplane(
            &body,
            WorkplaneSelection::Region(REGION_CAP_END),
            [0.0, 0.0, 2.0],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default()
        )
        .unwrap_err()
        .kind,
        WorkplaneError::StaleSource
    );
    let body = block();
    let limited = WorkplanePolicy {
        max_corners: 3,
        ..Default::default()
    };
    assert_eq!(
        face_workplane(
            &body,
            WorkplaneSelection::Region(REGION_CAP_END),
            [0.0, 0.0, 2.0],
            [1.0, 0.0, 0.0],
            &limited
        )
        .unwrap_err()
        .kind,
        WorkplaneError::BudgetExceeded
    );
    assert_eq!(
        face_workplane(
            &body,
            WorkplaneSelection::Face(FaceId::OUTSIDE),
            [0.0; 3],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default()
        )
        .unwrap_err()
        .kind,
        WorkplaneError::InvalidFace
    );
}

#[test]
fn operand_qualified_regions_resolve_reused_region_numbers() {
    let mut body = patch(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2], &[0, 2, 3]],
    );
    body.source_map = crate::source_map::SourceMap::new(
        &body.mesh,
        alloc::vec![
            Feature::BooleanFace { operand: 0 },
            Feature::BooleanFace { operand: 1 }
        ],
        body.mesh.vertices().map(|_| Feature::Imported).collect(),
    );
    let make = |selection| {
        face_workplane(
            &body,
            selection,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default(),
        )
    };
    assert_eq!(
        make(WorkplaneSelection::Region(0)).unwrap_err().kind,
        WorkplaneError::AmbiguousSelection
    );
    for operand in [0, 1] {
        assert_eq!(
            make(WorkplaneSelection::OperandRegion(OperandRegion {
                operand,
                region: 0
            }))
            .unwrap()
            .faces()
            .len(),
            1
        );
    }
}

#[test]
fn holed_refined_cap_and_reflection_preserve_outward_normal() {
    use crate::profile::{Loop2, Profile2, Seg2};
    let outer = rect(4.0, 3.0).unwrap();
    let inner = Loop2::new(alloc::vec![
        Seg2::line((1.0, 1.0)),
        Seg2::line((1.0, 2.0)),
        Seg2::line((2.0, 2.0)),
        Seg2::line((2.0, 1.0))
    ])
    .unwrap();
    let profile = Profile2::new(outer.outer().clone(), alloc::vec![inner]).unwrap();
    let placement =
        Placement3::from_axes([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.2, 0.0, 1.0], [0.0; 3]);
    let make = |policy| {
        let body = tessellate_extrude(&profile, &placement, 2.0, CapMode::Both, &policy).unwrap();
        face_workplane(
            &body,
            WorkplaneSelection::Region(REGION_CAP_END),
            [0.4, 0.0, 2.0],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default(),
        )
        .unwrap()
    };
    let a = make(EvalPolicy::default());
    let b = make(
        EvalPolicy::default().with_cap_refinement(exedra_triangulate::RefineParams::default()),
    );
    assert_eq!(a.frame(), b.frame());
    assert!(a.faces().len() > 1);
    assert!(dot(a.frame().rows.map(|r| r[2]), [0.0, 0.0, 1.0]) > 1.0 - 1e-12);
}

#[test]
fn invalid_authored_inputs_empty_selection_and_face_budget() {
    let body = block();
    let make = |selection, origin, x, policy: &WorkplanePolicy| {
        face_workplane(&body, selection, origin, x, policy)
    };
    let selection = WorkplaneSelection::Region(REGION_CAP_END);
    assert_eq!(
        make(
            WorkplaneSelection::Region(u32::MAX),
            [0.0; 3],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default()
        )
        .unwrap_err()
        .kind,
        WorkplaneError::EmptySelection
    );
    for x in [[0.0; 3], [f64::NAN, 0.0, 0.0], [f64::INFINITY, 0.0, 0.0]] {
        assert_eq!(
            make(
                selection.clone(),
                [0.0, 0.0, 2.0],
                x,
                &WorkplanePolicy::default()
            )
            .unwrap_err()
            .kind,
            WorkplaneError::InvalidInput
        );
    }
    let invalid = WorkplanePolicy {
        distance_tolerance: f64::NAN,
        ..Default::default()
    };
    assert_eq!(
        make(
            selection.clone(),
            [0.0, 0.0, 2.0],
            [1.0, 0.0, 0.0],
            &invalid
        )
        .unwrap_err()
        .kind,
        WorkplaneError::InvalidInput
    );
    let triangles = patch(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2], &[0, 2, 3]],
    );
    let limited = WorkplanePolicy {
        max_faces: 1,
        ..Default::default()
    };
    assert_eq!(
        face_workplane(
            &triangles,
            WorkplaneSelection::Region(0),
            [0.0; 3],
            [1.0, 0.0, 0.0],
            &limited
        )
        .unwrap_err()
        .kind,
        WorkplaneError::BudgetExceeded
    );
}

#[test]
fn parallel_axis_cannot_be_created_from_projection_roundoff() {
    let body = patch(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 5.0]],
        &[&[0, 1, 2]],
    );
    let policy = WorkplanePolicy {
        min_axis_sine: 1e-20,
        ..Default::default()
    };
    assert_eq!(
        face_workplane(
            &body,
            WorkplaneSelection::Region(0),
            [0.0; 3],
            [-1.0, -5.0, 1.0],
            &policy
        )
        .unwrap_err()
        .kind,
        WorkplaneError::ParallelAxis
    );
}

#[test]
fn inspection_and_resolution_share_planarity_evidence() {
    let body = patch(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.1],
            [0.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    );
    let failure = face_workplane(
        &body,
        WorkplaneSelection::Region(0),
        [0.0; 3],
        [1.0, 0.0, 0.0],
        &WorkplanePolicy::default(),
    )
    .unwrap_err();
    assert_eq!(failure.selection, WorkplaneSelection::Region(0));
    assert!(
        matches!(failure.evidence, WorkplaneEvidence::PlaneDeviation { measured, tolerance } if measured > tolerance && tolerance == 1e-6)
    );
    let inventory = inspect_surfaces(&body, &SurfaceInventoryPolicy::default()).unwrap();
    let entry = inventory
        .entries()
        .iter()
        .find(|entry| entry.selector == SurfaceSelector::Region(0))
        .unwrap();
    assert_eq!(entry.status, Err(failure.clone()));
    assert_eq!(entry.patches[0].plane, Err(failure));
    assert_eq!(entry.origins[0].feature, Feature::Imported);
    assert_eq!(inventory.stats().corners_examined, 8);
}

#[test]
fn detailed_budgets_count_completed_work_and_never_return_partial_inventory() {
    let body = block();
    let select = WorkplaneSelection::EndCap;
    let failure = face_workplane(
        &body,
        select.clone(),
        [0.0, 0.0, 2.0],
        [1.0, 0.0, 0.0],
        &WorkplanePolicy {
            max_corners: 3,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(failure.selection, select);
    assert_eq!(
        failure.evidence,
        WorkplaneEvidence::Budget {
            resource: SurfaceResource::Corners,
            completed: 3,
            limit: 3
        }
    );
    for (policy, resource, limit) in [
        (
            SurfaceInventoryPolicy {
                max_faces: 1,
                ..Default::default()
            },
            SurfaceResource::ScannedFaces,
            1,
        ),
        (
            SurfaceInventoryPolicy {
                max_selectors: 1,
                ..Default::default()
            },
            SurfaceResource::Selectors,
            1,
        ),
        (
            SurfaceInventoryPolicy {
                max_corners: 3,
                ..Default::default()
            },
            SurfaceResource::Corners,
            3,
        ),
    ] {
        assert_eq!(
            inspect_surfaces(&body, &policy).unwrap_err(),
            SurfaceInventoryError::Budget {
                resource,
                completed: limit,
                limit
            }
        );
    }
}

#[test]
fn inventory_preserves_disconnected_and_duplicate_label_evidence() {
    let mut body = patch(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2], &[3, 4, 5]],
    );
    body.source_map = crate::source_map::SourceMap::new(
        &body.mesh,
        alloc::vec![Feature::CapEnd;2],
        body.mesh.vertices().map(|_| Feature::Imported).collect(),
    );
    body.source_map.bind_source("panel");
    let attachment = WorkplaneAttachment {
        surface: SurfaceSelector::SourceEndCap("panel".into()),
        anchor: [0.0; 3],
        projection: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    };
    let failure = attachment
        .resolve(&body, &WorkplanePolicy::default())
        .unwrap_err();
    assert!(
        matches!(&failure.evidence,WorkplaneEvidence::Disconnected {representatives} if representatives.len()==2)
    );
    let inventory = inspect_surfaces(&body, &SurfaceInventoryPolicy::default()).unwrap();
    let entry = inventory
        .entries()
        .iter()
        .find(|entry| entry.selector == attachment.surface)
        .unwrap();
    assert_eq!(entry.status, Err(failure));
    assert_eq!(entry.patches.len(), 2);
    assert!(entry.patches.iter().all(|patch| patch.plane.is_ok()));
    body.source_map.set_ambiguous_sources(&["panel".into()]);
    let failure = attachment
        .resolve(&body, &WorkplanePolicy::default())
        .unwrap_err();
    assert_eq!(
        failure.evidence,
        WorkplaneEvidence::DuplicateSource {
            source: "panel".into()
        }
    );
    let inventory = inspect_surfaces(&body, &SurfaceInventoryPolicy::default()).unwrap();
    assert_eq!(
        inventory
            .entries()
            .iter()
            .find(|entry| entry.selector == attachment.surface)
            .unwrap()
            .status,
        Err(failure)
    );
}

#[test]
fn inventory_reports_mixed_operands_unknown_ancestry_missing_and_stale_separately() {
    let mut body = patch(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2], &[0, 2, 3]],
    );
    body.source_map = crate::source_map::SourceMap::new(
        &body.mesh,
        alloc::vec![
            Feature::BooleanFace { operand: 0 },
            Feature::BooleanFace { operand: 1 }
        ],
        body.mesh.vertices().map(|_| Feature::Imported).collect(),
    );
    let inventory = inspect_surfaces(&body, &SurfaceInventoryPolicy::default()).unwrap();
    let entry = inventory
        .entries()
        .iter()
        .find(|entry| entry.selector == SurfaceSelector::Region(0))
        .unwrap();
    assert_eq!(
        entry.status.as_ref().unwrap_err().evidence,
        WorkplaneEvidence::MixedOperands {
            operands: alloc::vec![Some(0), Some(1)]
        }
    );
    assert_eq!(entry.unknown_origin_faces, 2);
    assert!(entry.origins.is_empty());
    assert_eq!(inventory.stats().unknown_origin_faces, 2);
    assert!(
        inventory
            .entries()
            .iter()
            .filter(|entry| matches!(entry.selector, SurfaceSelector::OperandRegion(_)))
            .all(|entry| entry.status.is_ok())
    );
    let missing = WorkplaneAttachment {
        surface: SurfaceSelector::SourceEndCap("absent".into()),
        anchor: [0.0; 3],
        projection: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    };
    let failure = missing
        .resolve(&body, &WorkplanePolicy::default())
        .unwrap_err();
    assert_eq!(failure.selection, missing.surface.selection());
    assert_eq!(failure.kind, WorkplaneError::EmptySelection);
    let edit = body.mesh.edit();
    let _: () = edit.finish();
    assert_eq!(
        inventory.check(&body.mesh),
        Err(WorkplaneError::StaleSource)
    );
    assert_eq!(
        inspect_surfaces(&body, &SurfaceInventoryPolicy::default()).unwrap_err(),
        SurfaceInventoryError::StaleSource
    );
}
