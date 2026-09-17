// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

#[test]
fn boundary_validator_rejects_crossings_contacts_backtracking_and_nested_holes() {
    let mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap();
    let face = mesh.faces().next().unwrap();
    let source = BoundarySource {
        face,
        edge: mesh.face_edge(face).unwrap(),
        adjacent_face: None,
    };
    let make = |points: &[[f64; 2]]| PatchLoop {
        points: points.to_vec(),
        sources: alloc::vec![source;points.len()],
    };
    let policy = BoundaryPolicy::default();
    for mut loops in [
        alloc::vec![make(&[[0.0, 0.0], [3.0, 3.0], [0.0, 2.0], [3.0, 0.0]])],
        alloc::vec![make(&[
            [0.0, 0.0],
            [3.0, 0.0],
            [2.0, 0.0],
            [3.0, 3.0],
            [0.0, 3.0]
        ])],
        alloc::vec![
            make(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]),
            make(&[[0.0, 1.0], [0.0, 2.0], [1.0, 2.0], [1.0, 1.0]])
        ],
        alloc::vec![
            make(&[[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [0.0, 6.0]]),
            make(&[[1.0, 1.0], [1.0, 5.0], [5.0, 5.0], [5.0, 1.0]]),
            make(&[[2.0, 2.0], [2.0, 3.0], [3.0, 3.0], [3.0, 2.0]])
        ],
    ] {
        assert!(matches!(
            validate_loops(&mut loops, &policy, &mut BoundaryStats::default()),
            Err(ClearanceError::InvalidBoundary | ClearanceError::BoundaryContact { .. })
        ));
    }
}
#[test]
fn source_binding_preserves_geometry_queries_and_snapshot_lifetime() {
    use crate::workplane::{WorkplanePolicy, face_workplane};
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap();
    let faces: Vec<_> = mesh.faces().collect();
    let frame = face_workplane(
        &mesh,
        &faces,
        [0.0; 3],
        None,
        [1.0, 0.0, 0.0],
        &WorkplanePolicy::default(),
    )
    .unwrap();
    let patch = PlanarPatch::from_workplane(&mesh, &frame, &BoundaryPolicy::default()).unwrap();
    let before = patch.circle_clearance([1.0, 1.25], 0.2).unwrap();
    assert!((before.clearance - 0.8).abs() < 1e-12);
    assert_eq!(before.nearest.source.adjacent_face, None);
    let points = patch.loops()[0].points().as_ptr();
    let measured = patch.measure().unwrap();
    let stats = patch.stats();
    let bound = patch
        .try_map_sources(|source| Ok::<_, ()>((source.edge, 42_u32)))
        .unwrap();
    assert_eq!(
        bound.loops()[0].points().as_ptr(),
        points,
        "binding reuses checked point buffers"
    );
    assert_eq!(bound.stats(), stats);
    assert_eq!(bound.measure().unwrap(), measured);
    let after = bound.circle_clearance([1.0, 1.25], 0.2).unwrap();
    assert_eq!(after.clearance, before.clearance);
    assert_eq!(after.nearest.point, before.nearest.point);
    assert_eq!(after.nearest.source, (before.nearest.source.edge, 42));
    let vertex = mesh.vertices().next().unwrap();
    let mut edit = mesh.edit();
    exedra_mesh::op::set_vertex_position(&mut edit, vertex, [10.0, 10.0, 10.0]).unwrap();
    let _: () = edit.finish();
    assert_eq!(
        PlanarPatch::from_workplane(&mesh, &frame, &BoundaryPolicy::default()).unwrap_err(),
        ClearanceError::Workplane(WorkplaneError::StaleSource)
    );
    drop(mesh);
    assert_eq!(bound.circle_clearance([1.0, 1.25], 0.2).unwrap(), after);
}
