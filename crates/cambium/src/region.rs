// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Region-tagging operators and helpers.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec;

use exedra::{FaceId, HalfEdgeId};

use crate::op_common::op_error;
use crate::selection::{EdgeSet, FaceSet, canonicalize_edge_set, canonicalize_face_set};
use crate::{
    Artifact, Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, OpContext, OpError,
    OpErrorKind, OpReport, SmallCounters,
};

/// Default untagged face region.
pub const REGION_UNTAGGED: u32 = 0;

/// Parameters for [`TagFaceRegion`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TagFaceRegionParams {
    /// Region identifier to write.
    pub region_id: u32,
    /// Faces to tag.
    pub faces: FaceSet,
}

/// Edit operator that writes face-region tags.
#[derive(Copy, Clone, Debug, Default)]
pub struct TagFaceRegion;

impl EditOperator for TagFaceRegion {
    type Params = TagFaceRegionParams;
    type Plan = TagFaceRegionParams;
    type Output = FaceSet;

    fn name(&self) -> &'static str {
        "tag.face.region"
    }

    fn apply(
        &self,
        txn: &mut exedra::EditSession<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut faces = params.faces.clone();
        let canonicalized = canonicalize_face_set(&mut faces);
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        if canonicalized {
            report.stats.counters.selections_canonicalized = 1;
        }
        if txn
            .mesh()
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .is_none()
        {
            return Err(op_error(
                ctx,
                OpErrorKind::MissingAttribute,
                DiagCode::MissingRequiredAttribute,
                "missing required dense face.region layer",
            ));
        }

        let mut tagged = FaceSet::new();
        for face in faces {
            if face == FaceId::OUTSIDE {
                continue;
            }
            if exedra::op::set_face_region(txn, face, params.region_id).is_err() {
                return Err(op_error(
                    ctx,
                    OpErrorKind::PreconditionFailed,
                    DiagCode::PreconditionFailed,
                    "face set contains invalid/stale face id",
                ));
            }
            report.stats.counters.faces_processed =
                report.stats.counters.faces_processed.saturating_add(1);
            tagged.push(face);
        }

        Ok((report, tagged))
    }

    fn compile(
        &self,
        _mesh: &exedra::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan(
        &self,
        txn: &mut exedra::EditSession<'_>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        self.apply(txn, plan, ctx)
    }
}

/// Parameters for [`SelectBoundaryEdgeLoop`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectBoundaryEdgeLoopParams {
    /// Boundary seed edge used to trace one full loop.
    pub seed_edge: HalfEdgeId,
}

/// Deterministic boundary-loop selection operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct SelectBoundaryEdgeLoop;

impl EditOperator for SelectBoundaryEdgeLoop {
    type Params = SelectBoundaryEdgeLoopParams;
    type Plan = SelectBoundaryEdgeLoopParams;
    type Output = EdgeSet;

    fn name(&self) -> &'static str {
        "select.edgeloop.boundary"
    }

    fn apply(
        &self,
        txn: &mut exedra::EditSession<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let selection = select_boundary_edge_loop(txn.mesh(), params.seed_edge)?;
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        report.stats.counters.selections_canonicalized =
            selection.counters.selections_canonicalized;
        report.stats.elements_touched.half_edges =
            u64::try_from(selection.edges.len()).expect("edge count should fit u64");
        let _ = report.artifacts.push(Artifact::EdgeSet {
            name: self.name().into(),
            half_edges: selection.edges.clone(),
        });
        Ok((report, selection.edges))
    }

    fn compile(
        &self,
        _mesh: &exedra::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan(
        &self,
        txn: &mut exedra::EditSession<'_>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        self.apply(txn, plan, ctx)
    }
}

/// Deterministic face-selection query result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionSelection {
    /// Canonical face IDs matching the requested region.
    pub faces: FaceSet,
    /// Query counters.
    pub counters: SmallCounters,
}

/// Deterministic boundary-loop edge-selection query result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EdgeLoopSelection {
    /// Canonical undirected edge IDs in the selected loop.
    pub edges: EdgeSet,
    /// Query counters.
    pub counters: SmallCounters,
}

/// Deterministic region flood-fill query result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionFloodSelection {
    /// Region identifier used by the flood fill.
    pub region_id: u32,
    /// Canonical connected face IDs matching `region_id`.
    pub faces: FaceSet,
    /// Query counters.
    pub counters: SmallCounters,
}

/// Returns all non-OUTSIDE faces tagged with `region_id`.
pub fn select_faces_by_region(
    mesh: &exedra::Mesh,
    region_id: u32,
) -> Result<RegionSelection, OpError> {
    let layer = mesh
        .attrs()
        .dense(exedra::attr::FACE_REGION)
        .ok_or_else(|| {
            OpError::new(
                OpErrorKind::MissingAttribute,
                vec![Diagnostic::new(
                    DiagLevel::Error,
                    DiagCode::MissingRequiredAttribute,
                    "missing required dense face.region layer",
                )],
                Artifacts::default(),
            )
        })?;

    let mut result = RegionSelection::default();
    for face in mesh.faces() {
        if face == FaceId::OUTSIDE {
            continue;
        }
        result.counters.faces_processed = result.counters.faces_processed.saturating_add(1);
        if layer
            .get(face.as_id())
            .is_some_and(|value| *value == region_id)
        {
            result.faces.push(face);
        }
    }
    if canonicalize_face_set(&mut result.faces) {
        result.counters.selections_canonicalized = 1;
    }
    Ok(result)
}

/// Returns the boundary loop containing `seed_edge`.
///
/// v0.1 scope supports boundary loops only. Passing an interior edge returns
/// [`OpErrorKind::PreconditionFailed`].
pub fn select_boundary_edge_loop(
    mesh: &exedra::Mesh,
    seed_edge: HalfEdgeId,
) -> Result<EdgeLoopSelection, OpError> {
    let twin = mesh.twin(seed_edge).ok_or_else(|| {
        query_error(
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            "edge loop seed is stale or missing twin",
        )
    })?;
    let seed_face = mesh.face(seed_edge).ok_or_else(|| {
        query_error(
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            "edge loop seed is stale",
        )
    })?;
    let twin_face = mesh.face(twin).ok_or_else(|| {
        query_error(
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            "edge loop seed has stale twin face",
        )
    })?;

    let start = if seed_face == FaceId::OUTSIDE {
        seed_edge
    } else if twin_face == FaceId::OUTSIDE {
        twin
    } else {
        return Err(query_error(
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            "edge loop selection currently supports boundary edges only",
        ));
    };

    let mut result = EdgeLoopSelection::default();
    let mut visited = BTreeSet::new();
    let mut current = start;
    loop {
        if !visited.insert(current) {
            if current == start {
                break;
            }
            return Err(query_error(
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                "boundary loop traversal revisited a non-start half-edge",
            ));
        }
        if mesh.face(current) != Some(FaceId::OUTSIDE) {
            return Err(query_error(
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                "boundary loop traversal encountered non-OUTSIDE half-edge",
            ));
        }
        let canonical = mesh.canonical_edge(current).ok_or_else(|| {
            query_error(
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                "boundary loop traversal encountered stale canonical edge",
            )
        })?;
        result.edges.push(canonical);
        let next = mesh.next(current).ok_or_else(|| {
            query_error(
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                "boundary loop traversal encountered missing next pointer",
            )
        })?;
        if next == start {
            break;
        }
        current = next;
    }

    if canonicalize_edge_set(&mut result.edges) {
        result.counters.selections_canonicalized = 1;
    }
    Ok(result)
}

/// Flood-fills connected faces that share the seed face's region ID.
pub fn flood_fill_faces_by_region(
    mesh: &exedra::Mesh,
    seed_face: FaceId,
) -> Result<RegionFloodSelection, OpError> {
    if seed_face == FaceId::OUTSIDE {
        return Err(query_error(
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            "region flood fill seed cannot be FaceId::OUTSIDE",
        ));
    }
    let _seed_edge = mesh.face_edge(seed_face).ok_or_else(|| {
        query_error(
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            "region flood fill seed face is stale",
        )
    })?;
    let layer = mesh
        .attrs()
        .dense(exedra::attr::FACE_REGION)
        .ok_or_else(|| {
            query_error(
                OpErrorKind::MissingAttribute,
                DiagCode::MissingRequiredAttribute,
                "missing required dense face.region layer",
            )
        })?;
    let region_id = *layer.get(seed_face.as_id()).ok_or_else(|| {
        query_error(
            OpErrorKind::InternalInvariantViolation,
            DiagCode::InternalInvariantViolation,
            "seed face missing face.region value",
        )
    })?;

    let mut visited = BTreeSet::new();
    visited.insert(seed_face);
    let mut queue = VecDeque::new();
    queue.push_back(seed_face);

    let mut result = RegionFloodSelection {
        region_id,
        ..RegionFloodSelection::default()
    };

    while let Some(face) = queue.pop_front() {
        result.counters.faces_processed = result.counters.faces_processed.saturating_add(1);
        result.faces.push(face);
        for corner in mesh.face_loop(face) {
            let twin = mesh.twin(corner).ok_or_else(|| {
                query_error(
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    "region flood fill encountered stale twin",
                )
            })?;
            let adjacent = mesh.face(twin).ok_or_else(|| {
                query_error(
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    "region flood fill encountered stale adjacent face",
                )
            })?;
            if adjacent == FaceId::OUTSIDE || visited.contains(&adjacent) {
                continue;
            }
            if layer
                .get(adjacent.as_id())
                .is_some_and(|value| *value == region_id)
            {
                visited.insert(adjacent);
                queue.push_back(adjacent);
            }
        }
    }

    if canonicalize_face_set(&mut result.faces) {
        result.counters.selections_canonicalized = 1;
    }
    Ok(result)
}

fn query_error(kind: OpErrorKind, code: DiagCode, message: &'static str) -> OpError {
    OpError::new(
        kind,
        vec![Diagnostic::new(DiagLevel::Error, code, message)],
        Artifacts::default(),
    )
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra::{BuildParams, FaceId, HalfEdgeId, Id, Mesh, MeshBuilder};

    use super::{
        EdgeLoopSelection, REGION_UNTAGGED, RegionFloodSelection, SelectBoundaryEdgeLoop,
        SelectBoundaryEdgeLoopParams, TagFaceRegion, TagFaceRegionParams,
        flood_fill_faces_by_region, select_boundary_edge_loop, select_faces_by_region,
    };
    use crate::{
        EdgeSet, EditOperator, OpErrorKind, OperatorRunner, SmallCounters, canonicalize_edge_set,
        test_support::commit,
    };

    fn one_quad_mesh() -> (Mesh, FaceId) {
        let mut builder = MeshBuilder::new();
        let _ = builder.push_vertex([0.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 1.0, 0.0]);
        let _ = builder.push_vertex([0.0, 1.0, 0.0]);
        builder.add_face(&[0, 1, 2, 3]).expect("valid quad");
        let built = builder.build().expect("build should succeed");
        (built.mesh, built.face_ids[0])
    }

    #[test]
    fn tag_face_region_sets_dense_layer_and_marks_dirty() {
        let (mut mesh, face) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let op = TagFaceRegion;
        let params = TagFaceRegionParams {
            region_id: 7,
            faces: vec![face, face],
        };

        let mut result =
            commit(&mut runner, &mut mesh, &op, &params).expect("tag operator should succeed");

        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face region layer must exist")
            .get(face.as_id())
            .copied();
        assert_eq!(region, Some(7));
        assert_eq!(result.report.name, op.name());
        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.selections_canonicalized, 1);
        let mut dirty_faces = Vec::new();
        result.change_set.dirty.drain_faces_into(&mut dirty_faces);
        assert_eq!(dirty_faces, vec![face]);
    }

    #[test]
    fn tag_face_region_supports_explicit_untagged_region() {
        let (mut mesh, face) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let op = TagFaceRegion;
        let set_non_zero = TagFaceRegionParams {
            region_id: 3,
            faces: vec![face],
        };
        commit(&mut runner, &mut mesh, &op, &set_non_zero)
            .expect("tagging to non-zero should succeed");

        let set_untagged = TagFaceRegionParams {
            region_id: REGION_UNTAGGED,
            faces: vec![face],
        };
        commit(&mut runner, &mut mesh, &op, &set_untagged)
            .expect("tagging to untagged should succeed");

        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face region layer must exist")
            .get(face.as_id())
            .copied();
        assert_eq!(region, Some(REGION_UNTAGGED));
    }

    #[test]
    fn face_region_default_is_untagged() {
        let mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face region layer must exist")
            .get(face.as_id())
            .copied();
        assert_eq!(region, Some(REGION_UNTAGGED));
    }

    #[test]
    fn testkit_region_untagged_matches_cambium_constant() {
        assert_eq!(cambium_testkit::regions::REGION_UNTAGGED, REGION_UNTAGGED);
    }

    #[test]
    fn select_faces_by_region_returns_canonical_face_set() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut runner = OperatorRunner::new();

        let params = TagFaceRegionParams {
            region_id: 11,
            faces: vec![faces[1], faces[0], faces[1]],
        };
        let _ = commit(&mut runner, &mut mesh, &TagFaceRegion, &params)
            .expect("tagging should succeed");

        let selected = select_faces_by_region(&mesh, 11).expect("selection should succeed");
        assert_eq!(selected.faces, vec![faces[0], faces[1]]);
        assert_eq!(selected.counters.faces_processed, 2);
        assert_eq!(selected.counters.selections_canonicalized, 0);
    }

    #[test]
    fn tag_face_region_rejects_stale_face_id() {
        let (mut mesh, _) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let stale = FaceId::from(Id::new(999, NonZeroU32::MIN));
        let params = TagFaceRegionParams {
            region_id: 5,
            faces: vec![stale],
        };

        let error = commit(&mut runner, &mut mesh, &TagFaceRegion, &params)
            .expect_err("stale face id should fail");
        assert_eq!(error.kind, OpErrorKind::PreconditionFailed);
        let face = mesh.faces().next().expect("face should exist");
        assert!(
            mesh.attrs()
                .dense(exedra::attr::FACE_REGION)
                .expect("face region layer should exist")
                .get(face.as_id())
                .is_some_and(|v| *v == 0)
        );
    }

    fn boundary_edges(mesh: &Mesh) -> EdgeSet {
        let mut edges = EdgeSet::new();
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                let twin = mesh.twin(corner).expect("corner must have twin");
                if mesh.face(twin) == Some(FaceId::OUTSIDE) {
                    edges.push(
                        mesh.canonical_edge(corner)
                            .expect("boundary corner must have canonical edge"),
                    );
                }
            }
        }
        let _ = canonicalize_edge_set(&mut edges);
        edges
    }

    fn first_interior_edge(mesh: &Mesh) -> HalfEdgeId {
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                let twin = mesh.twin(corner).expect("corner must have twin");
                if mesh
                    .face(twin)
                    .is_some_and(|adjacent| adjacent != FaceId::OUTSIDE)
                {
                    return corner;
                }
            }
        }
        panic!("expected interior edge");
    }

    #[test]
    fn select_boundary_edge_loop_returns_canonical_boundary_edges() {
        let (mesh, face) = one_quad_mesh();
        let seed = mesh
            .face_loop(face)
            .next()
            .expect("quad should provide one boundary edge");
        let selected = select_boundary_edge_loop(&mesh, seed).expect("selection should succeed");
        let expected = boundary_edges(&mesh);
        assert_eq!(
            selected,
            EdgeLoopSelection {
                edges: expected,
                counters: SmallCounters {
                    selections_canonicalized: 1,
                    ..SmallCounters::default()
                },
            }
        );
    }

    #[test]
    fn select_boundary_edge_loop_rejects_interior_seed_edge() {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            &[&[0, 1, 4, 3], &[1, 2, 5, 4]],
        )
        .expect("mesh build should succeed");
        let interior = first_interior_edge(&mesh);
        let error = select_boundary_edge_loop(&mesh, interior)
            .expect_err("interior edge seed should be rejected");
        assert_eq!(error.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn flood_fill_faces_by_region_selects_connected_component_only() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [3.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3], &[4, 5, 6, 7]],
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();

        let mut runner = OperatorRunner::new();
        let params = TagFaceRegionParams {
            region_id: 11,
            faces: vec![faces[0], faces[1]],
        };
        let _ = commit(&mut runner, &mut mesh, &TagFaceRegion, &params)
            .expect("tagging should succeed");

        let filled = flood_fill_faces_by_region(&mesh, faces[0]).expect("flood fill should work");
        assert_eq!(
            filled,
            RegionFloodSelection {
                region_id: 11,
                faces: vec![faces[0]],
                counters: SmallCounters {
                    faces_processed: 1,
                    ..SmallCounters::default()
                },
            }
        );
    }

    #[test]
    fn flood_fill_faces_by_region_rejects_outside_seed() {
        let (mesh, _) = one_quad_mesh();
        let error = flood_fill_faces_by_region(&mesh, FaceId::OUTSIDE)
            .expect_err("outside seed should fail");
        assert_eq!(error.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn select_boundary_edge_loop_operator_returns_edge_set() {
        let (mut mesh, face) = one_quad_mesh();
        let seed = mesh
            .face_loop(face)
            .next()
            .expect("quad should provide one boundary edge");
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed },
        )
        .expect("operator should succeed");
        assert_eq!(result.report.name, "select.edgeloop.boundary");
        assert!(
            result
                .report
                .artifacts
                .iter()
                .any(|artifact| artifact.name() == "select.edgeloop.boundary")
        );
        let expected = boundary_edges(&mesh);
        assert_eq!(result.output, expected);
    }

    #[test]
    fn select_boundary_edge_loop_operator_rejects_interior_seed() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            &[&[0, 1, 4, 3], &[1, 2, 5, 4]],
        )
        .expect("mesh build should succeed");
        let interior = first_interior_edge(&mesh);
        let mut runner = OperatorRunner::new();
        let error = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams {
                seed_edge: interior,
            },
        )
        .expect_err("interior edge should fail");
        assert_eq!(error.kind, OpErrorKind::PreconditionFailed);
    }
}
