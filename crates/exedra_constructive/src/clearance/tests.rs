// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{
    builders,
    ir::CapMode,
    tessellate::{EvalPolicy, tessellate_extrude},
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplanePolicy},
};

fn patch(profile: &crate::profile::Profile2) -> PlanarPatch {
    let body = tessellate_extrude(
        profile,
        &Placement3::IDENTITY,
        1.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let frame = WorkplaneAttachment {
        surface: SurfaceSelector::EndCap,
        anchor: [0.0; 3],
        projection: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    }
    .resolve(&body, &WorkplanePolicy::default())
    .unwrap();
    PlanarPatch::from_workplane(&body, &frame, &BoundaryPolicy::default()).unwrap()
}

#[test]
fn rectangle_clearance_has_boundary_witness_and_explicit_threshold_band() {
    let patch = patch(&builders::rect(4.0, 3.0).unwrap());
    let result = patch.circle_clearance([1.0, 1.25], 0.2).unwrap();
    assert!(result.center_inside);
    assert!((result.clearance - 0.8).abs() < 1e-12);
    assert_eq!(result.nearest.point, [0.0, 1.25]);
    assert_eq!(result.edges_tested, 4);
    assert_eq!(result.classify(0.5, 1e-6), Ok(ClearanceDecision::Satisfied));
    assert_eq!(
        result.classify(0.8, 1e-6),
        Ok(ClearanceDecision::WithinTolerance)
    );
    assert_eq!(result.classify(0.9, 1e-6), Ok(ClearanceDecision::Violated));
    let outside = patch.circle_clearance([-1.0, 1.0], 0.2).unwrap();
    assert!(!outside.center_inside);
    assert_eq!(outside.clearance, -1.2);
    let contact = patch.circle_clearance([0.0, 1.0], 0.0).unwrap();
    assert!(!contact.center_inside);
    assert_eq!(contact.clearance, 0.0);
    assert_eq!(
        contact.classify(0.0, 0.0),
        Ok(ClearanceDecision::WithinTolerance)
    );
}

#[test]
fn holes_are_exclusions_and_nearest_witness_can_belong_to_a_hole() {
    use crate::profile::{Loop2, Profile2, Seg2};
    let profile = Profile2::new(
        builders::rect(6.0, 5.0).unwrap().outer().clone(),
        alloc::vec![
            Loop2::new(alloc::vec![
                Seg2::line((2.0, 2.0)),
                Seg2::line((2.0, 3.0)),
                Seg2::line((4.0, 3.0)),
                Seg2::line((4.0, 2.0)),
            ])
            .unwrap()
        ],
    )
    .unwrap();
    let patch = patch(&profile);
    assert_eq!(patch.loops().len(), 2);
    let result = patch.circle_clearance([1.5, 2.5], 0.25).unwrap();
    assert_eq!(result.clearance, 0.25);
    assert_eq!(result.nearest.loop_index, 1);
    assert_eq!(result.nearest.point, [2.0, 2.5]);
    let hole = patch.circle_clearance([3.0, 2.5], 0.25).unwrap();
    assert_eq!(hole.clearance, -0.75);
    assert!(!hole.center_inside);
}

#[test]
fn concavity_does_not_get_replaced_by_a_bounding_box() {
    use crate::profile::{Loop2, Profile2, Seg2};
    let profile = Profile2::simple(
        Loop2::new(alloc::vec![
            Seg2::line((0.0, 0.0)),
            Seg2::line((4.0, 0.0)),
            Seg2::line((4.0, 1.0)),
            Seg2::line((1.0, 1.0)),
            Seg2::line((1.0, 4.0)),
            Seg2::line((0.0, 4.0)),
        ])
        .unwrap(),
    )
    .unwrap();
    let patch = patch(&profile);
    assert_eq!(
        patch.circle_clearance([2.0, 2.0], 0.1).unwrap().clearance,
        -1.1
    );
    assert!((patch.circle_clearance([0.5, 2.0], 0.1).unwrap().clearance - 0.4).abs() < 1e-12);
}

#[test]
fn extraction_and_queries_reject_stale_invalid_and_over_budget_inputs() {
    let mut body = tessellate_extrude(
        &builders::rect(4.0, 3.0).unwrap(),
        &Placement3::IDENTITY,
        1.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let frame = WorkplaneAttachment {
        surface: SurfaceSelector::EndCap,
        anchor: [0.0; 3],
        projection: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    }
    .resolve(&body, &WorkplanePolicy::default())
    .unwrap();
    for policy in [
        BoundaryPolicy {
            max_corners: 1,
            ..BoundaryPolicy::default()
        },
        BoundaryPolicy {
            max_pair_checks: 1,
            ..BoundaryPolicy::default()
        },
    ] {
        assert_eq!(
            PlanarPatch::from_workplane(&body, &frame, &policy).unwrap_err(),
            ClearanceError::BudgetExceeded
        );
    }
    let patch = PlanarPatch::from_workplane(&body, &frame, &BoundaryPolicy::default()).unwrap();
    for (center, radius) in [
        ([f64::NAN, 0.0], 0.1),
        ([0.0; 2], -0.1),
        ([0.0; 2], f64::INFINITY),
    ] {
        assert_eq!(
            patch.circle_clearance(center, radius).unwrap_err(),
            ClearanceError::InvalidInput
        );
    }
    assert_eq!(
        patch
            .circle_clearance([1.0; 2], 0.1)
            .unwrap()
            .classify(0.0, -1.0),
        Err(geometry::ClearanceError::InvalidInput)
    );
    let vertex = body.mesh.vertices().next().unwrap();
    let mut edit = body.mesh.edit();
    exedra_mesh::op::set_vertex_position(&mut edit, vertex, [0.1, 0.0, 0.0]).unwrap();
    let _: () = edit.finish();
    assert_eq!(
        PlanarPatch::from_workplane(&body, &frame, &BoundaryPolicy::default()).unwrap_err(),
        ClearanceError::Workplane(WorkplaneError::StaleSource)
    );
}

#[test]
fn tilted_patch_and_retriangulated_cap_preserve_clearance() {
    let placement = Placement3::from_axes(
        [0.8, 0.0, 0.6],
        [0.0, 1.0, 0.0],
        [-0.6, 0.0, 0.8],
        [10.0, 2.0, 5.0],
    );
    let policy = EvalPolicy {
        cap_refinement: Some(exedra_triangulate::RefineParams::default()),
        ..EvalPolicy::default()
    };
    let body = tessellate_extrude(
        &builders::rect(4.0, 3.0).unwrap(),
        &placement,
        1.0,
        CapMode::Both,
        &policy,
    )
    .unwrap();
    let frame = WorkplaneAttachment {
        surface: SurfaceSelector::EndCap,
        anchor: [10.0, 2.0, 5.0],
        projection: [-0.6, 0.0, 0.8],
        x_direction: [0.8, 0.0, 0.6],
    }
    .resolve(&body, &WorkplanePolicy::default())
    .unwrap();
    let patch = PlanarPatch::from_workplane(&body, &frame, &BoundaryPolicy::default()).unwrap();
    assert!((patch.circle_clearance([1.0, 1.25], 0.2).unwrap().clearance - 0.8).abs() < 2e-6);
    assert!(patch.stats().corners_examined > patch.stats().boundary_edges);
}
