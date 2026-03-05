// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fluent face-edit workflow builder over Cambium operator plans.
//!
//! [`MeshEdit`] is an ergonomics layer over the explicit operator lifecycle.
//! It compiles deterministic per-step plans and can then preview/apply the
//! compiled workflow.

use alloc::vec::Vec;

use exedra::{DeletePolicy, FaceId, HalfEdgeId, Mesh};

use crate::plan::{EditPlan, PlanFingerprint, PlanHasher};
use crate::{
    DeleteFaces, DeleteFacesOutput, DeleteFacesParams, DeleteFacesPlan, ExtrudeFaces,
    ExtrudeFacesParams, ExtrudeMode, FaceSet, InsetFaces, InsetFacesParams, InsetFacesPlan,
    OpError, OpReport, OperatorRunner, PreviewResult, Selection, SelectionDomainError,
    TagFaceRegion, TagFaceRegionParams, canonicalize_face_set, flood_fill_faces_by_region,
    select_boundary_edge_loop, select_faces_by_region,
};

#[derive(Clone, Debug)]
enum MeshEditStep {
    Extrude { distance: f32 },
    Inset { factor: f32 },
    Tag { region_id: u32 },
    Delete { policy: DeletePolicy },
}

/// Deterministic compiled workflow plan for [`MeshEdit`].
#[derive(Clone, Debug)]
pub struct MeshEditPlan {
    /// Initial canonical face selection used by the fluent chain.
    pub initial_faces: FaceSet,
    /// Deterministic per-step plans in compile order.
    pub steps: Vec<MeshEditStepPlan>,
    /// Final canonical face selection after replaying all steps.
    pub final_faces: FaceSet,
    /// Deterministic workflow fingerprint.
    pub fingerprint: PlanFingerprint,
}

/// One compiled workflow step inside [`MeshEditPlan`].
#[derive(Clone, Debug)]
pub enum MeshEditStepPlan {
    /// Compiled `edit.face.extrude` plan.
    Extrude(EditPlan<ExtrudeFacesParams>),
    /// Compiled `edit.face.inset` plan.
    Inset(EditPlan<InsetFacesPlan>),
    /// Compiled `tag.face.region` plan.
    Tag(EditPlan<TagFaceRegionParams>),
    /// Compiled `edit.delete.faces` plan.
    Delete(EditPlan<DeleteFacesPlan>),
}

/// Result from [`MeshEdit::apply`] / [`MeshEdit::apply_with_plan`].
#[derive(Clone, Debug)]
pub struct MeshEditResult {
    /// Per-step operator reports in execution order.
    pub reports: Vec<OpReport>,
    /// Final fluent selection for follow-on flows.
    pub selection: Selection,
}

/// Result from [`MeshEdit::preview`] / [`MeshEdit::preview_with_plan`].
#[derive(Clone, Debug)]
pub struct MeshEditPreview {
    /// Preview mesh produced by running the full chain on a clone.
    pub preview_mesh: Mesh,
    /// Per-step operator reports in execution order.
    pub reports: Vec<OpReport>,
    /// Final fluent selection for follow-on flows.
    pub selection: Selection,
}

/// Fluent face-workflow builder for common modeling chains.
///
/// Typical sequence:
/// 1. start with [`MeshEdit::new`],
/// 2. seed a face selection with [`MeshEdit::select_faces`] (or [`MeshEdit::select`]),
/// 3. append fluent steps (`extrude`, `inset`, `tag`, `delete`),
/// 4. run `plan`, `preview`, or `apply`.
///
/// ```rust
/// use cambium::{MeshEdit, OperatorRunner, Selection};
/// use exedra::DeletePolicy;
///
/// let mesh = exedra::Mesh::from_polygons(
///     &[
///         [0.0, 0.0, 0.0],
///         [1.0, 0.0, 0.0],
///         [1.0, 1.0, 0.0],
///         [0.0, 1.0, 0.0],
///     ],
///     &[&[0, 1, 2, 3]],
/// )
/// .expect("quad mesh should build");
/// let face = mesh.faces().next().expect("face should exist");
///
/// // Build once, then choose preview/apply entry points.
/// let flow = MeshEdit::new()
///     .select(Selection::from(vec![face]))
///     .expect("flow requires face-domain selection")
///     .extrude(0.25)
///     .inset(0.4)
///     .tag(7)
///     .delete(DeletePolicy::KeepIsolated);
///
/// let mut runner = OperatorRunner::new();
/// let plan = flow.plan(&mut runner, &mesh).expect("plan should compile");
/// let preview = flow
///     .preview_with_plan(&mut runner, &mesh, &plan)
///     .expect("preview should succeed");
/// assert!(matches!(preview.selection, Selection::Faces(_)));
/// ```
///
/// Selection handoff rules:
/// - `extrude` selects `cap_faces`,
/// - `inset` selects `inner_faces`,
/// - `tag` keeps selected faces,
/// - `delete` clears selection.
///
/// Query helpers:
/// - [`MeshEdit::select_faces_by_region`] seeds faces from region tags.
/// - [`MeshEdit::flood_faces_by_region`] seeds connected region flood results.
/// - [`MeshEdit::query_boundary_edge_loop`] exposes an edge-domain selection for
///   composition outside this face-only fluent chain.
#[derive(Clone, Debug, Default)]
pub struct MeshEdit {
    selected_faces: FaceSet,
    steps: Vec<MeshEditStep>,
}

impl MeshEdit {
    /// Creates an empty fluent workflow.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the fluent selection with canonical face IDs.
    #[must_use]
    pub fn select_faces(mut self, mut faces: FaceSet) -> Self {
        let _ = canonicalize_face_set(&mut faces);
        self.selected_faces = faces;
        self
    }

    /// Replaces the fluent selection from the generic [`Selection`] bridge.
    pub fn select(self, selection: Selection) -> Result<Self, SelectionDomainError> {
        let faces = selection.require_faces()?.clone();
        Ok(self.select_faces(faces))
    }

    /// Replaces the fluent face selection with all faces tagged `region_id`.
    pub fn select_faces_by_region(self, mesh: &Mesh, region_id: u32) -> Result<Self, OpError> {
        let selected = select_faces_by_region(mesh, region_id)?;
        Ok(self.select_faces(selected.faces))
    }

    /// Replaces the fluent face selection with a connected region flood-fill result.
    pub fn flood_faces_by_region(self, mesh: &Mesh, seed_face: FaceId) -> Result<Self, OpError> {
        let selected = flood_fill_faces_by_region(mesh, seed_face)?;
        Ok(self.select_faces(selected.faces))
    }

    /// Returns an edge-domain boundary-loop selection for `seed_edge`.
    ///
    /// This method is intentionally query-only: `MeshEdit` step chaining is
    /// currently face-domain, so edge selections should be handed off through
    /// domain-generic composition APIs.
    pub fn query_boundary_edge_loop(
        mesh: &Mesh,
        seed_edge: HalfEdgeId,
    ) -> Result<Selection, OpError> {
        let selected = select_boundary_edge_loop(mesh, seed_edge)?;
        Ok(Selection::from(selected.edges))
    }

    /// Adds one extrude step using the current selected faces.
    #[must_use]
    pub fn extrude(mut self, distance: f32) -> Self {
        self.steps.push(MeshEditStep::Extrude { distance });
        self
    }

    /// Adds one inset step using the current selected faces.
    #[must_use]
    pub fn inset(mut self, factor: f32) -> Self {
        self.steps.push(MeshEditStep::Inset { factor });
        self
    }

    /// Adds one region-tagging step using the current selected faces.
    #[must_use]
    pub fn tag(mut self, region_id: u32) -> Self {
        self.steps.push(MeshEditStep::Tag { region_id });
        self
    }

    /// Adds one face-deletion step using the current selected faces.
    #[must_use]
    pub fn delete(mut self, policy: DeletePolicy) -> Self {
        self.steps.push(MeshEditStep::Delete { policy });
        self
    }

    /// Compiles deterministic per-step plans for this workflow.
    pub fn plan(&self, runner: &mut OperatorRunner, mesh: &Mesh) -> Result<MeshEditPlan, OpError> {
        let mut working = mesh.clone();
        let mut current_faces = self.selected_faces.clone();
        let mut compiled_steps = Vec::with_capacity(self.steps.len());

        for step in &self.steps {
            match *step {
                MeshEditStep::Extrude { distance } => {
                    let params = ExtrudeFacesParams {
                        faces: current_faces.clone(),
                        mode: ExtrudeMode::ShellOpen,
                        distance,
                    };
                    let op = ExtrudeFaces;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_faces = result.output.cap_faces;
                    compiled_steps.push(MeshEditStepPlan::Extrude(plan));
                }
                MeshEditStep::Inset { factor } => {
                    let params = InsetFacesParams {
                        faces: current_faces.clone(),
                        factor,
                    };
                    let op = InsetFaces;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_faces = result.output.inner_faces;
                    compiled_steps.push(MeshEditStepPlan::Inset(plan));
                }
                MeshEditStep::Tag { region_id } => {
                    let params = TagFaceRegionParams {
                        region_id,
                        faces: current_faces.clone(),
                    };
                    let op = TagFaceRegion;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_faces = result.output;
                    compiled_steps.push(MeshEditStepPlan::Tag(plan));
                }
                MeshEditStep::Delete { policy } => {
                    let params = DeleteFacesParams {
                        faces: current_faces.clone(),
                        policy,
                    };
                    let op = DeleteFaces;
                    let plan = runner.compile(&working, &op, &params)?;
                    let _ = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_faces.clear();
                    compiled_steps.push(MeshEditStepPlan::Delete(plan));
                }
            }
        }

        let fingerprint =
            mesh_edit_fingerprint(&compiled_steps, &self.selected_faces, &current_faces);

        Ok(MeshEditPlan {
            initial_faces: self.selected_faces.clone(),
            steps: compiled_steps,
            final_faces: current_faces,
            fingerprint,
        })
    }

    /// Applies this workflow on a mesh clone.
    ///
    /// This convenience method compiles a fresh plan before previewing.
    pub fn preview(
        &self,
        runner: &mut OperatorRunner,
        mesh: &Mesh,
    ) -> Result<MeshEditPreview, OpError> {
        let plan = self.plan(runner, mesh)?;
        self.preview_with_plan(runner, mesh, &plan)
    }

    /// Applies a precompiled workflow plan on a mesh clone.
    pub fn preview_with_plan(
        &self,
        runner: &mut OperatorRunner,
        mesh: &Mesh,
        plan: &MeshEditPlan,
    ) -> Result<MeshEditPreview, OpError> {
        let mut preview_mesh = mesh.clone();
        let mut reports = Vec::with_capacity(plan.steps.len());
        let mut current_faces = plan.initial_faces.clone();

        for step in &plan.steps {
            match step {
                MeshEditStepPlan::Extrude(compiled) => {
                    let op = ExtrudeFaces;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_faces = output.cap_faces;
                    reports.push(report);
                }
                MeshEditStepPlan::Inset(compiled) => {
                    let op = InsetFaces;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_faces = output.inner_faces;
                    reports.push(report);
                }
                MeshEditStepPlan::Tag(compiled) => {
                    let op = TagFaceRegion;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_faces = output;
                    reports.push(report);
                }
                MeshEditStepPlan::Delete(compiled) => {
                    let op = DeleteFaces;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output: _,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_faces.clear();
                    reports.push(report);
                }
            }
        }

        Ok(MeshEditPreview {
            preview_mesh,
            reports,
            selection: Selection::from(current_faces),
        })
    }

    /// Applies this workflow in-place.
    ///
    /// This convenience method compiles a fresh plan before applying.
    pub fn apply(
        &self,
        runner: &mut OperatorRunner,
        mesh: &mut Mesh,
    ) -> Result<MeshEditResult, OpError> {
        let plan = self.plan(runner, mesh)?;
        self.apply_with_plan(runner, mesh, &plan)
    }

    /// Applies a precompiled workflow plan in-place.
    pub fn apply_with_plan(
        &self,
        runner: &mut OperatorRunner,
        mesh: &mut Mesh,
        plan: &MeshEditPlan,
    ) -> Result<MeshEditResult, OpError> {
        let mut reports = Vec::with_capacity(plan.steps.len());
        let mut current_faces = plan.initial_faces.clone();

        for step in &plan.steps {
            match step {
                MeshEditStepPlan::Extrude(compiled) => {
                    let op = ExtrudeFaces;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_faces = result.output.cap_faces;
                    reports.push(result.report);
                }
                MeshEditStepPlan::Inset(compiled) => {
                    let op = InsetFaces;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_faces = result.output.inner_faces;
                    reports.push(result.report);
                }
                MeshEditStepPlan::Tag(compiled) => {
                    let op = TagFaceRegion;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_faces = result.output;
                    reports.push(result.report);
                }
                MeshEditStepPlan::Delete(compiled) => {
                    let op = DeleteFaces;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    let DeleteFacesOutput { .. } = result.output;
                    current_faces.clear();
                    reports.push(result.report);
                }
            }
        }

        Ok(MeshEditResult {
            reports,
            selection: Selection::from(current_faces),
        })
    }
}

fn mesh_edit_fingerprint(
    steps: &[MeshEditStepPlan],
    initial_faces: &[FaceId],
    final_faces: &[FaceId],
) -> PlanFingerprint {
    let mut hasher = PlanHasher::new();
    hasher.write_str("mesh-edit-plan/v1");
    hasher.write_face_set(initial_faces);
    hasher.write_len(steps.len());
    for step in steps {
        match step {
            MeshEditStepPlan::Extrude(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::Inset(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::Tag(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::Delete(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
        }
    }
    hasher.write_face_set(final_faces);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use exedra::{DeletePolicy, MeshBuilder};

    use super::MeshEdit;
    use crate::{
        ExtrudeFaces, ExtrudeFacesParams, ExtrudeMode, InsetFaces, InsetFacesParams,
        OperatorRunner, Selection, TagFaceRegion, TagFaceRegionParams, mesh_signature,
    };

    fn one_quad_mesh() -> (exedra::Mesh, exedra::FaceId) {
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
    fn mesh_edit_plan_preview_and_apply_match_direct_flow() {
        let (mesh, face) = one_quad_mesh();
        let flow = MeshEdit::new()
            .select_faces(vec![face])
            .extrude(0.5)
            .inset(0.2)
            .tag(9)
            .delete(DeletePolicy::KeepIsolated);

        let mut runner = OperatorRunner::new();
        let plan = flow.plan(&mut runner, &mesh).expect("plan should compile");

        let mut preview_runner = OperatorRunner::new();
        let preview = flow
            .preview_with_plan(&mut preview_runner, &mesh, &plan)
            .expect("preview should succeed");

        let mut apply_runner = OperatorRunner::new();
        let mut applied_mesh = mesh.clone();
        let applied = flow
            .apply_with_plan(&mut apply_runner, &mut applied_mesh, &plan)
            .expect("apply should succeed");

        assert_eq!(
            mesh_signature(&preview.preview_mesh),
            mesh_signature(&applied_mesh)
        );
        assert_eq!(preview.selection, applied.selection);
        assert!(matches!(applied.selection, Selection::Faces(ref faces) if faces.is_empty()));
        assert_eq!(preview.reports.len(), 4);
        assert_eq!(applied.reports.len(), 4);
    }

    #[test]
    fn mesh_edit_plan_fingerprint_is_deterministic() {
        let (mesh, face) = one_quad_mesh();
        let flow = MeshEdit::new()
            .select_faces(vec![face])
            .extrude(0.5)
            .inset(0.2)
            .tag(3);

        let mut runner_a = OperatorRunner::new();
        let mut runner_b = OperatorRunner::new();
        let plan_a = flow
            .plan(&mut runner_a, &mesh)
            .expect("plan should compile");
        let plan_b = flow
            .plan(&mut runner_b, &mesh)
            .expect("plan should compile");
        assert_eq!(plan_a.fingerprint, plan_b.fingerprint);
    }

    #[test]
    fn mesh_edit_select_accepts_bridge_selection_faces_only() {
        let (mesh, face) = one_quad_mesh();
        let flow = MeshEdit::new()
            .select(Selection::from(vec![face]))
            .expect("face selection should be accepted")
            .tag(7);

        let mut runner = OperatorRunner::new();
        let mut mesh = mesh;
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("flow should apply");
        assert!(matches!(result.selection, Selection::Faces(_)));

        let err = MeshEdit::new()
            .select(Selection::from(vec![exedra::VertexId::new(
                0,
                core::num::NonZeroU32::MIN,
            )]))
            .expect_err("vertex selection should be rejected");
        assert_eq!(err.expected, crate::SelectionKind::Faces);
        assert_eq!(err.actual, crate::SelectionKind::Vertices);
    }

    #[test]
    fn mesh_edit_apply_matches_manual_operator_sequence() {
        let (base, face) = one_quad_mesh();

        let flow = MeshEdit::new()
            .select_faces(vec![face])
            .extrude(0.25)
            .inset(0.4);

        let mut flow_runner = OperatorRunner::new();
        let mut flow_mesh = base.clone();
        let flow_result = flow
            .apply(&mut flow_runner, &mut flow_mesh)
            .expect("fluent flow should succeed");

        let mut direct_mesh = base.clone();
        let mut direct_runner = OperatorRunner::new();

        let extrude_params = ExtrudeFacesParams {
            faces: vec![face],
            mode: ExtrudeMode::ShellOpen,
            distance: 0.25,
        };
        let extrude_plan = direct_runner
            .compile(&direct_mesh, &ExtrudeFaces, &extrude_params)
            .expect("extrude compile should succeed");
        let extrude_result = direct_runner
            .apply_in_place(&mut direct_mesh, &ExtrudeFaces, &extrude_plan)
            .expect("extrude apply should succeed");

        let inset_params = InsetFacesParams {
            faces: extrude_result.output.cap_faces.clone(),
            factor: 0.4,
        };
        let inset_plan = direct_runner
            .compile(&direct_mesh, &InsetFaces, &inset_params)
            .expect("inset compile should succeed");
        let inset_result = direct_runner
            .apply_in_place(&mut direct_mesh, &InsetFaces, &inset_plan)
            .expect("inset apply should succeed");

        assert_eq!(mesh_signature(&flow_mesh), mesh_signature(&direct_mesh));
        assert_eq!(
            flow_result.selection,
            Selection::from(inset_result.output.inner_faces)
        );
    }

    #[test]
    fn mesh_edit_can_seed_selection_from_region_query() {
        let (mut mesh, face) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let tag_params = TagFaceRegionParams {
            region_id: 7,
            faces: vec![face],
        };
        let tag_plan = runner
            .compile(&mesh, &TagFaceRegion, &tag_params)
            .expect("tag compile should succeed");
        let _ = runner
            .apply_in_place(&mut mesh, &TagFaceRegion, &tag_plan)
            .expect("tagging should succeed");

        let flow = MeshEdit::new()
            .select_faces_by_region(&mesh, 7)
            .expect("region query should succeed")
            .tag(9);

        let mut apply_runner = OperatorRunner::new();
        let result = flow
            .apply(&mut apply_runner, &mut mesh)
            .expect("flow should succeed");
        assert!(matches!(result.selection, Selection::Faces(ref faces) if faces == &vec![face]));
    }

    #[test]
    fn mesh_edit_can_seed_selection_from_region_flood() {
        let mesh = exedra::Mesh::from_polygons(
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
        let faces = mesh.faces().collect::<Vec<_>>();

        let flow = MeshEdit::new()
            .flood_faces_by_region(&mesh, faces[0])
            .expect("flood query should succeed")
            .tag(5);

        let mut runner = OperatorRunner::new();
        let mut mesh = mesh;
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("flow should succeed");
        assert!(matches!(result.selection, Selection::Faces(_)));
    }

    #[test]
    fn mesh_edit_boundary_query_returns_edge_selection() {
        let (mesh, face) = one_quad_mesh();
        let seed = mesh
            .face_loop(face)
            .next()
            .expect("quad face should have a boundary edge");
        let selection = MeshEdit::query_boundary_edge_loop(&mesh, seed)
            .expect("boundary loop query should succeed");
        assert!(matches!(selection, Selection::Edges(ref edges) if !edges.is_empty()));
    }
}
