// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fluent face-edit workflow builder over Cambium operator plans.
//!
//! [`MeshEdit`] is an ergonomics layer over the explicit operator lifecycle.
//! It compiles deterministic per-step plans and can then preview/apply the
//! compiled workflow.

use alloc::vec;
use alloc::vec::Vec;

use exedra::{DeletePolicy, FaceId, HalfEdgeId, Mesh};

use crate::plan::{EditPlan, PlanFingerprint, PlanHasher};
use crate::{Artifacts, DiagCode, DiagLevel, Diagnostic};
use crate::{
    CutRectFace, CutRectFaceOutput, CutRectFaceParams, CutRectFacePlan, SolidifyFaces,
    SolidifyFacesParams, SolidifyMode,
};
use crate::{
    DeleteEdges, DeleteEdgesOutput, DeleteEdgesParams, DeleteFaces, DeleteFacesOutput,
    DeleteFacesParams, DeleteFacesPlan, DeleteVertices, DeleteVerticesOutput, DeleteVerticesParams,
    DissolveEdges, DissolveEdgesOutput, DissolveEdgesParams, DissolveVertices,
    DissolveVerticesOutput, DissolveVerticesParams, EdgeSet, ExtrudeFaces, ExtrudeFacesParams,
    ExtrudeMode, FaceSet, InsetFaces, InsetFacesParams, InsetFacesPlan, MarkEdgeSeam,
    MarkEdgeSeamParams, MarkEdgeSharp, MarkEdgeSharpParams, OpError, OpErrorKind, OpReport,
    OperatorRunner, PreviewResult, Selection, SelectionKind, TagFaceRegion, TagFaceRegionParams,
    VertexSet, canonicalize_face_set, flood_fill_faces_by_region, select_boundary_edge_loop,
    select_faces_by_region,
};

#[derive(Clone, Debug)]
enum MeshEditStep {
    SelectFacesByRegion {
        region_id: u32,
    },
    FloodFacesByRegion {
        seed_face: FaceId,
    },
    SelectBoundaryEdgeLoop {
        seed_edge: HalfEdgeId,
    },
    Extrude {
        distance: f32,
    },
    Inset {
        factor: f32,
    },
    Solidify {
        thickness: f32,
        mode: SolidifyMode,
    },
    CutRect {
        frame_origin: [f32; 3],
        frame_u: [f32; 3],
        frame_v: [f32; 3],
        rect_min: [f32; 2],
        rect_max: [f32; 2],
    },
    Tag {
        region_id: u32,
    },
    MarkSeam {
        seam: bool,
    },
    MarkSharp {
        sharp: f32,
    },
    DeleteFaces {
        policy: DeletePolicy,
    },
    DeleteEdges {
        policy: DeletePolicy,
    },
    DissolveEdges,
    DeleteVertices,
    DissolveVertices,
}

/// Deterministic compiled workflow plan for [`MeshEdit`].
#[derive(Clone, Debug)]
pub struct MeshEditPlan {
    /// Initial canonical selection used by the fluent chain.
    pub initial_selection: Selection,
    /// Deterministic per-step plans in compile order.
    pub steps: Vec<MeshEditStepPlan>,
    /// Final canonical selection after replaying all steps.
    pub final_selection: Selection,
    /// Deterministic workflow fingerprint.
    pub fingerprint: PlanFingerprint,
}

/// One compiled workflow step inside [`MeshEditPlan`].
#[derive(Clone, Debug)]
pub enum MeshEditStepPlan {
    /// Compiled selection query step.
    Select(SelectionStepPlan),
    /// Compiled `edit.face.extrude` plan.
    Extrude(EditPlan<ExtrudeFacesParams>),
    /// Compiled `edit.face.inset` plan.
    Inset(EditPlan<InsetFacesPlan>),
    /// Compiled `edit.face.solidify` plan.
    Solidify(EditPlan<ExtrudeFacesParams>),
    /// Compiled `edit.face.cut.rect` plan.
    CutRect(EditPlan<CutRectFacePlan>),
    /// Compiled `tag.face.region` plan.
    Tag(EditPlan<TagFaceRegionParams>),
    /// Compiled `mark.edge.seam` plan.
    MarkSeam(EditPlan<MarkEdgeSeamParams>),
    /// Compiled `mark.edge.sharp` plan.
    MarkSharp(EditPlan<MarkEdgeSharpParams>),
    /// Compiled `edit.delete.faces` plan.
    DeleteFaces(EditPlan<DeleteFacesPlan>),
    /// Compiled `edit.delete.edges` plan.
    DeleteEdges(EditPlan<DeleteEdgesParams>),
    /// Compiled `edit.dissolve.edges` plan.
    DissolveEdges(EditPlan<DissolveEdgesParams>),
    /// Compiled `edit.delete.vertices` plan.
    DeleteVertices(EditPlan<DeleteVerticesParams>),
    /// Compiled `edit.dissolve.vertices` plan.
    DissolveVertices(EditPlan<DissolveVerticesParams>),
}

/// One compiled selection-query step inside [`MeshEditPlan`].
#[derive(Clone, Debug)]
pub enum SelectionStepPlan {
    /// Compiled `select.faces.by_region` transition.
    FacesByRegion {
        /// Queried region ID.
        region_id: u32,
        /// Resolved selection after this step.
        selection: Selection,
    },
    /// Compiled region flood-fill transition.
    FloodFacesByRegion {
        /// Seed face used for flood fill.
        seed_face: FaceId,
        /// Resolved selection after this step.
        selection: Selection,
    },
    /// Compiled boundary-loop transition.
    BoundaryEdgeLoop {
        /// Seed edge used for boundary-loop traversal.
        seed_edge: HalfEdgeId,
        /// Resolved selection after this step.
        selection: Selection,
    },
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
/// 3. append fluent steps (`extrude`, `inset`, `solidify`, `cut_rect`, `tag`, `delete`),
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
///     .extrude(0.25)
///     .inset(0.4)
///     .tag(7)
///     .delete_faces(DeletePolicy::KeepIsolated);
///
/// let mut runner = OperatorRunner::new();
/// let plan = flow.plan(&mut runner, &mesh).expect("plan should compile");
/// let preview = flow
///     .preview_with_plan(&mut runner, &mesh, &plan)
///     .expect("preview should succeed");
/// assert!(matches!(preview.selection, Selection::Faces(_)));
/// ```
///
/// "Boxy hat" workflow:
///
/// ```rust
/// use cambium::{MeshEdit, OperatorRunner};
///
/// let mut mesh = exedra::Mesh::from_polygons(
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
/// // The selection handoff is the point:
/// // extrude -> cap face selection
/// // inset   -> inner face selection
/// let flow = MeshEdit::new()
///     .select_faces(vec![face])
///     .extrude(0.25)
///     .inset(0.4);
///
/// let mut runner = OperatorRunner::new();
/// let result = flow.apply(&mut runner, &mut mesh).expect("flow should succeed");
/// assert!(matches!(result.selection, cambium::Selection::Faces(_)));
/// # Ok::<(), cambium::OpError>(())
/// ```
///
/// Wall opening workflow:
///
/// ```rust
/// use cambium::{DeletePolicy, MeshEdit, OperatorRunner};
///
/// let mut mesh = exedra::Mesh::from_polygons(
///     &[
///         [0.0, 0.0, 0.0],
///         [4.0, 0.0, 0.0],
///         [4.0, 3.0, 0.0],
///         [0.0, 3.0, 0.0],
///     ],
///     &[&[0, 1, 2, 3]],
/// )
/// .expect("wall mesh should build");
/// let wall = mesh.faces().next().expect("wall face should exist");
///
/// // cut_rect selects the newly created inner face.
/// // delete_faces then removes it, leaving an opening.
/// let flow = MeshEdit::new()
///     .select_faces(vec![wall])
///     .cut_rect(
///         [0.0, 0.0, 0.0],
///         [1.0, 0.0, 0.0],
///         [0.0, 1.0, 0.0],
///         [1.2, 0.4],
///         [2.8, 2.2],
///     )
///     .delete_faces(DeletePolicy::KeepIsolated);
///
/// let mut runner = OperatorRunner::new();
/// let result = flow.apply(&mut runner, &mut mesh).expect("flow should succeed");
/// assert!(matches!(result.selection, cambium::Selection::Faces(ref faces) if faces.is_empty()));
/// # Ok::<(), cambium::OpError>(())
/// ```
///
/// Selection handoff rules:
/// - `select_faces_by_region` / `flood_faces_by_region` select faces,
/// - `select_boundary_edge_loop` selects edges,
/// - `extrude` selects `cap_faces`,
/// - `inset` selects `inner_faces`,
/// - `solidify` selects `cap_faces`,
/// - `cut_rect` selects `inner_faces`,
/// - `tag` keeps selected faces,
/// - `mark_seam` / `mark_sharp` keep selected edges,
/// - `delete_faces` / `delete_edges` / `delete_vertices` clear selection,
/// - `dissolve_edges` selects merged faces.
/// - `dissolve_vertices` selects rebuilt faces.
///
/// Query helpers are compiled as explicit selection-transition steps.
#[derive(Clone, Debug)]
pub struct MeshEdit {
    selection: Selection,
    steps: Vec<MeshEditStep>,
}

impl Default for MeshEdit {
    fn default() -> Self {
        Self {
            selection: Selection::from(FaceSet::new()),
            steps: Vec::new(),
        }
    }
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
        self.selection = Selection::from(faces);
        self
    }

    /// Replaces the fluent selection from the generic [`Selection`] bridge.
    #[must_use]
    pub fn select(mut self, mut selection: Selection) -> Self {
        let _ = selection.canonicalize();
        self.selection = selection;
        self
    }

    /// Adds one query step that replaces selection with faces tagged `region_id`.
    #[must_use]
    pub fn select_faces_by_region(mut self, region_id: u32) -> Self {
        self.steps
            .push(MeshEditStep::SelectFacesByRegion { region_id });
        self
    }

    /// Adds one query step that replaces selection with region flood-fill results.
    #[must_use]
    pub fn flood_faces_by_region(mut self, seed_face: FaceId) -> Self {
        self.steps
            .push(MeshEditStep::FloodFacesByRegion { seed_face });
        self
    }

    /// Adds one query step that replaces selection with a boundary edge loop.
    #[must_use]
    pub fn select_boundary_edge_loop(mut self, seed_edge: HalfEdgeId) -> Self {
        self.steps
            .push(MeshEditStep::SelectBoundaryEdgeLoop { seed_edge });
        self
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

    /// Adds one solidify step using the current selected faces.
    ///
    /// Uses [`SolidifyMode::KeepSource`] by default.
    #[must_use]
    pub fn solidify(mut self, thickness: f32) -> Self {
        self.steps.push(MeshEditStep::Solidify {
            thickness,
            mode: SolidifyMode::KeepSource,
        });
        self
    }

    /// Adds one solidify step with explicit source-face retention mode.
    #[must_use]
    pub fn solidify_with_mode(mut self, thickness: f32, mode: SolidifyMode) -> Self {
        self.steps.push(MeshEditStep::Solidify { thickness, mode });
        self
    }

    /// Adds one rectangular face-cut step using the currently selected face.
    ///
    /// Exactly one face must be selected when this step compiles.
    #[must_use]
    pub fn cut_rect(
        mut self,
        frame_origin: [f32; 3],
        frame_u: [f32; 3],
        frame_v: [f32; 3],
        rect_min: [f32; 2],
        rect_max: [f32; 2],
    ) -> Self {
        self.steps.push(MeshEditStep::CutRect {
            frame_origin,
            frame_u,
            frame_v,
            rect_min,
            rect_max,
        });
        self
    }

    /// Adds one region-tagging step using the current selected faces.
    #[must_use]
    pub fn tag(mut self, region_id: u32) -> Self {
        self.steps.push(MeshEditStep::Tag { region_id });
        self
    }

    /// Adds one seam-tagging step using the current selected edges.
    #[must_use]
    pub fn mark_seam(mut self, seam: bool) -> Self {
        self.steps.push(MeshEditStep::MarkSeam { seam });
        self
    }

    /// Adds one sharpness-tagging step using the current selected edges.
    #[must_use]
    pub fn mark_sharp(mut self, sharp: f32) -> Self {
        self.steps.push(MeshEditStep::MarkSharp { sharp });
        self
    }

    /// Adds one face-deletion step using the current selected faces.
    #[must_use]
    pub fn delete_faces(mut self, policy: DeletePolicy) -> Self {
        self.steps.push(MeshEditStep::DeleteFaces { policy });
        self
    }

    /// Adds one edge-deletion step using the current selected edges.
    #[must_use]
    pub fn delete_edges(mut self, policy: DeletePolicy) -> Self {
        self.steps.push(MeshEditStep::DeleteEdges { policy });
        self
    }

    /// Adds one edge-dissolve step using the current selected edges.
    #[must_use]
    pub fn dissolve_edges(mut self) -> Self {
        self.steps.push(MeshEditStep::DissolveEdges);
        self
    }

    /// Adds one vertex-deletion step using the current selected vertices.
    #[must_use]
    pub fn delete_vertices(mut self) -> Self {
        self.steps.push(MeshEditStep::DeleteVertices);
        self
    }

    /// Adds one vertex-dissolve step using the current selected vertices.
    #[must_use]
    pub fn dissolve_vertices(mut self) -> Self {
        self.steps.push(MeshEditStep::DissolveVertices);
        self
    }

    /// Compiles deterministic per-step plans for this workflow.
    pub fn plan(&self, runner: &mut OperatorRunner, mesh: &Mesh) -> Result<MeshEditPlan, OpError> {
        let mut working = mesh.clone();
        let mut current_selection = self.selection.clone();
        let mut compiled_steps = Vec::with_capacity(self.steps.len());

        for step in &self.steps {
            match *step {
                MeshEditStep::SelectFacesByRegion { region_id } => {
                    let selected = select_faces_by_region(&working, region_id)?;
                    current_selection = Selection::from(selected.faces);
                    compiled_steps.push(MeshEditStepPlan::Select(
                        SelectionStepPlan::FacesByRegion {
                            region_id,
                            selection: current_selection.clone(),
                        },
                    ));
                }
                MeshEditStep::FloodFacesByRegion { seed_face } => {
                    let selected = flood_fill_faces_by_region(&working, seed_face)?;
                    current_selection = Selection::from(selected.faces);
                    compiled_steps.push(MeshEditStepPlan::Select(
                        SelectionStepPlan::FloodFacesByRegion {
                            seed_face,
                            selection: current_selection.clone(),
                        },
                    ));
                }
                MeshEditStep::SelectBoundaryEdgeLoop { seed_edge } => {
                    let selected = select_boundary_edge_loop(&working, seed_edge)?;
                    current_selection = Selection::from(selected.edges);
                    compiled_steps.push(MeshEditStepPlan::Select(
                        SelectionStepPlan::BoundaryEdgeLoop {
                            seed_edge,
                            selection: current_selection.clone(),
                        },
                    ));
                }
                MeshEditStep::Extrude { distance } => {
                    let current_faces =
                        require_face_selection(&current_selection, "edit.face.extrude")?;
                    let params = ExtrudeFacesParams {
                        faces: current_faces,
                        mode: ExtrudeMode::ShellOpen,
                        distance,
                    };
                    let op = ExtrudeFaces;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output.cap_faces);
                    compiled_steps.push(MeshEditStepPlan::Extrude(plan));
                }
                MeshEditStep::Inset { factor } => {
                    let current_faces =
                        require_face_selection(&current_selection, "edit.face.inset")?;
                    let params = InsetFacesParams {
                        faces: current_faces,
                        factor,
                    };
                    let op = InsetFaces;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output.inner_faces);
                    compiled_steps.push(MeshEditStepPlan::Inset(plan));
                }
                MeshEditStep::Solidify { thickness, mode } => {
                    let current_faces =
                        require_face_selection(&current_selection, "edit.face.solidify")?;
                    let params = SolidifyFacesParams {
                        faces: current_faces,
                        mode,
                        thickness,
                    };
                    let op = SolidifyFaces;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output.cap_faces);
                    compiled_steps.push(MeshEditStepPlan::Solidify(plan));
                }
                MeshEditStep::CutRect {
                    frame_origin,
                    frame_u,
                    frame_v,
                    rect_min,
                    rect_max,
                } => {
                    let current_faces =
                        require_face_selection(&current_selection, "edit.face.cut.rect")?;
                    if current_faces.len() != 1 {
                        return Err(OpError::new(
                            OpErrorKind::PreconditionFailed,
                            vec![Diagnostic::new(
                                DiagLevel::Error,
                                DiagCode::PreconditionFailed,
                                "edit.face.cut.rect requires exactly one selected face",
                            )],
                            Artifacts::default(),
                        ));
                    }
                    let params = CutRectFaceParams {
                        face: current_faces[0],
                        frame_origin,
                        frame_u,
                        frame_v,
                        rect_min,
                        rect_max,
                    };
                    let op = CutRectFace;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output.inner_faces);
                    compiled_steps.push(MeshEditStepPlan::CutRect(plan));
                }
                MeshEditStep::Tag { region_id } => {
                    let current_faces =
                        require_face_selection(&current_selection, "tag.face.region")?;
                    let params = TagFaceRegionParams {
                        region_id,
                        faces: current_faces,
                    };
                    let op = TagFaceRegion;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output);
                    compiled_steps.push(MeshEditStepPlan::Tag(plan));
                }
                MeshEditStep::MarkSeam { seam } => {
                    let current_edges =
                        require_edge_selection(&current_selection, "mark.edge.seam")?;
                    let params = MarkEdgeSeamParams {
                        edges: current_edges,
                        seam,
                    };
                    let op = MarkEdgeSeam;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output);
                    compiled_steps.push(MeshEditStepPlan::MarkSeam(plan));
                }
                MeshEditStep::MarkSharp { sharp } => {
                    let current_edges =
                        require_edge_selection(&current_selection, "mark.edge.sharp")?;
                    let params = MarkEdgeSharpParams {
                        edges: current_edges,
                        sharp,
                    };
                    let op = MarkEdgeSharp;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output);
                    compiled_steps.push(MeshEditStepPlan::MarkSharp(plan));
                }
                MeshEditStep::DeleteFaces { policy } => {
                    let current_faces =
                        require_face_selection(&current_selection, "edit.delete.faces")?;
                    let params = DeleteFacesParams {
                        faces: current_faces,
                        policy,
                    };
                    let op = DeleteFaces;
                    let plan = runner.compile(&working, &op, &params)?;
                    let _ = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(FaceSet::new());
                    compiled_steps.push(MeshEditStepPlan::DeleteFaces(plan));
                }
                MeshEditStep::DeleteEdges { policy } => {
                    let current_edges =
                        require_edge_selection(&current_selection, "edit.delete.edges")?;
                    let params = DeleteEdgesParams {
                        edges: current_edges,
                        policy,
                    };
                    let op = DeleteEdges;
                    let plan = runner.compile(&working, &op, &params)?;
                    let _ = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(EdgeSet::new());
                    compiled_steps.push(MeshEditStepPlan::DeleteEdges(plan));
                }
                MeshEditStep::DissolveEdges => {
                    let current_edges =
                        require_edge_selection(&current_selection, "edit.dissolve.edges")?;
                    let params = DissolveEdgesParams {
                        edges: current_edges,
                    };
                    let op = DissolveEdges;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output.faces);
                    compiled_steps.push(MeshEditStepPlan::DissolveEdges(plan));
                }
                MeshEditStep::DeleteVertices => {
                    let current_vertices =
                        require_vertex_selection(&current_selection, "edit.delete.vertices")?;
                    let params = DeleteVerticesParams {
                        vertices: current_vertices,
                    };
                    let op = DeleteVertices;
                    let plan = runner.compile(&working, &op, &params)?;
                    let _ = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(VertexSet::new());
                    compiled_steps.push(MeshEditStepPlan::DeleteVertices(plan));
                }
                MeshEditStep::DissolveVertices => {
                    let current_vertices =
                        require_vertex_selection(&current_selection, "edit.dissolve.vertices")?;
                    let params = DissolveVerticesParams {
                        vertices: current_vertices,
                    };
                    let op = DissolveVertices;
                    let plan = runner.compile(&working, &op, &params)?;
                    let result = runner.apply_in_place(&mut working, &op, &plan)?;
                    current_selection = Selection::from(result.output.faces);
                    compiled_steps.push(MeshEditStepPlan::DissolveVertices(plan));
                }
            }
        }

        let fingerprint =
            mesh_edit_fingerprint(&compiled_steps, &self.selection, &current_selection);

        Ok(MeshEditPlan {
            initial_selection: self.selection.clone(),
            steps: compiled_steps,
            final_selection: current_selection,
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
        let mut current_selection = plan.initial_selection.clone();

        for step in &plan.steps {
            match step {
                MeshEditStepPlan::Select(selection_step) => {
                    current_selection = match selection_step {
                        SelectionStepPlan::FacesByRegion { selection, .. }
                        | SelectionStepPlan::FloodFacesByRegion { selection, .. }
                        | SelectionStepPlan::BoundaryEdgeLoop { selection, .. } => {
                            selection.clone()
                        }
                    };
                }
                MeshEditStepPlan::Extrude(compiled) => {
                    let op = ExtrudeFaces;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(output.cap_faces);
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
                    current_selection = Selection::from(output.inner_faces);
                    reports.push(report);
                }
                MeshEditStepPlan::Solidify(compiled) => {
                    let op = SolidifyFaces;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(output.cap_faces);
                    reports.push(report);
                }
                MeshEditStepPlan::CutRect(compiled) => {
                    let op = CutRectFace;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output: CutRectFaceOutput { inner_faces, .. },
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(inner_faces);
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
                    current_selection = Selection::from(output);
                    reports.push(report);
                }
                MeshEditStepPlan::MarkSeam(compiled) => {
                    let op = MarkEdgeSeam;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(output);
                    reports.push(report);
                }
                MeshEditStepPlan::MarkSharp(compiled) => {
                    let op = MarkEdgeSharp;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(output);
                    reports.push(report);
                }
                MeshEditStepPlan::DeleteFaces(compiled) => {
                    let op = DeleteFaces;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output: _,
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(FaceSet::new());
                    reports.push(report);
                }
                MeshEditStepPlan::DeleteEdges(compiled) => {
                    let op = DeleteEdges;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output: DeleteEdgesOutput { .. },
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(EdgeSet::new());
                    reports.push(report);
                }
                MeshEditStepPlan::DissolveEdges(compiled) => {
                    let op = DissolveEdges;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output: DissolveEdgesOutput { faces, .. },
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(faces);
                    reports.push(report);
                }
                MeshEditStepPlan::DeleteVertices(compiled) => {
                    let op = DeleteVertices;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output: DeleteVerticesOutput { .. },
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(VertexSet::new());
                    reports.push(report);
                }
                MeshEditStepPlan::DissolveVertices(compiled) => {
                    let op = DissolveVertices;
                    let PreviewResult {
                        preview_mesh: next_mesh,
                        report,
                        output: DissolveVerticesOutput { faces, .. },
                    } = runner.preview_on_clone(&preview_mesh, &op, compiled)?;
                    preview_mesh = next_mesh;
                    current_selection = Selection::from(faces);
                    reports.push(report);
                }
            }
        }

        Ok(MeshEditPreview {
            preview_mesh,
            reports,
            selection: current_selection,
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
        let mut current_selection = plan.initial_selection.clone();

        for step in &plan.steps {
            match step {
                MeshEditStepPlan::Select(selection_step) => {
                    current_selection = match selection_step {
                        SelectionStepPlan::FacesByRegion { selection, .. }
                        | SelectionStepPlan::FloodFacesByRegion { selection, .. }
                        | SelectionStepPlan::BoundaryEdgeLoop { selection, .. } => {
                            selection.clone()
                        }
                    };
                }
                MeshEditStepPlan::Extrude(compiled) => {
                    let op = ExtrudeFaces;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output.cap_faces);
                    reports.push(result.report);
                }
                MeshEditStepPlan::Inset(compiled) => {
                    let op = InsetFaces;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output.inner_faces);
                    reports.push(result.report);
                }
                MeshEditStepPlan::Solidify(compiled) => {
                    let op = SolidifyFaces;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output.cap_faces);
                    reports.push(result.report);
                }
                MeshEditStepPlan::CutRect(compiled) => {
                    let op = CutRectFace;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output.inner_faces);
                    reports.push(result.report);
                }
                MeshEditStepPlan::Tag(compiled) => {
                    let op = TagFaceRegion;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output);
                    reports.push(result.report);
                }
                MeshEditStepPlan::MarkSeam(compiled) => {
                    let op = MarkEdgeSeam;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output);
                    reports.push(result.report);
                }
                MeshEditStepPlan::MarkSharp(compiled) => {
                    let op = MarkEdgeSharp;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output);
                    reports.push(result.report);
                }
                MeshEditStepPlan::DeleteFaces(compiled) => {
                    let op = DeleteFaces;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    let DeleteFacesOutput { .. } = result.output;
                    current_selection = Selection::from(FaceSet::new());
                    reports.push(result.report);
                }
                MeshEditStepPlan::DeleteEdges(compiled) => {
                    let op = DeleteEdges;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    let DeleteEdgesOutput { .. } = result.output;
                    current_selection = Selection::from(EdgeSet::new());
                    reports.push(result.report);
                }
                MeshEditStepPlan::DissolveEdges(compiled) => {
                    let op = DissolveEdges;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output.faces);
                    reports.push(result.report);
                }
                MeshEditStepPlan::DeleteVertices(compiled) => {
                    let op = DeleteVertices;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    let DeleteVerticesOutput { .. } = result.output;
                    current_selection = Selection::from(VertexSet::new());
                    reports.push(result.report);
                }
                MeshEditStepPlan::DissolveVertices(compiled) => {
                    let op = DissolveVertices;
                    let result = runner.apply_in_place(mesh, &op, compiled)?;
                    current_selection = Selection::from(result.output.faces);
                    reports.push(result.report);
                }
            }
        }

        Ok(MeshEditResult {
            reports,
            selection: current_selection,
        })
    }
}

fn mesh_edit_fingerprint(
    steps: &[MeshEditStepPlan],
    initial_selection: &Selection,
    final_selection: &Selection,
) -> PlanFingerprint {
    let mut hasher = PlanHasher::new();
    hasher.write_str("mesh-edit-plan/v1");
    write_selection(&mut hasher, initial_selection);
    hasher.write_len(steps.len());
    for step in steps {
        match step {
            MeshEditStepPlan::Select(step) => match step {
                SelectionStepPlan::FacesByRegion {
                    region_id,
                    selection,
                } => {
                    hasher.write_str("select.faces.by_region");
                    hasher.write_u32(*region_id);
                    write_selection(&mut hasher, selection);
                }
                SelectionStepPlan::FloodFacesByRegion {
                    seed_face,
                    selection,
                } => {
                    hasher.write_str("select.faces.flood.region");
                    hasher.write_u32(seed_face.index());
                    write_selection(&mut hasher, selection);
                }
                SelectionStepPlan::BoundaryEdgeLoop {
                    seed_edge,
                    selection,
                } => {
                    hasher.write_str("select.edgeloop.boundary");
                    hasher.write_u32(seed_edge.index());
                    write_selection(&mut hasher, selection);
                }
            },
            MeshEditStepPlan::Extrude(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::Inset(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::Solidify(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::CutRect(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::Tag(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::MarkSeam(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::MarkSharp(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::DeleteFaces(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::DeleteEdges(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::DissolveEdges(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::DeleteVertices(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
            MeshEditStepPlan::DissolveVertices(plan) => {
                hasher.write_str(plan.operator);
                hasher.write_u64(plan.fingerprint.value());
            }
        }
    }
    write_selection(&mut hasher, final_selection);
    hasher.finish()
}

fn write_selection(hasher: &mut PlanHasher, selection: &Selection) {
    match selection {
        Selection::Faces(faces) => {
            hasher.write_u8(0);
            hasher.write_face_set(faces);
        }
        Selection::Edges(edges) => {
            hasher.write_u8(1);
            hasher.write_len(edges.len());
            for edge in edges {
                hasher.write_u32(edge.index());
            }
        }
        Selection::Vertices(vertices) => {
            hasher.write_u8(2);
            hasher.write_len(vertices.len());
            for vertex in vertices {
                hasher.write_u32(vertex.index());
            }
        }
    }
}

fn require_face_selection(
    selection: &Selection,
    operator: &'static str,
) -> Result<FaceSet, OpError> {
    selection.require_faces().cloned().map_err(|mismatch| {
        let message = alloc::format!(
            "{operator} requires {} selection, got {}",
            SelectionKind::Faces,
            mismatch.actual
        );
        OpError::new(
            OpErrorKind::PreconditionFailed,
            vec![Diagnostic::new(
                DiagLevel::Error,
                DiagCode::PreconditionFailed,
                message,
            )],
            Artifacts::default(),
        )
    })
}

fn require_edge_selection(
    selection: &Selection,
    operator: &'static str,
) -> Result<EdgeSet, OpError> {
    selection.require_edges().cloned().map_err(|mismatch| {
        let message = alloc::format!(
            "{operator} requires {} selection, got {}",
            SelectionKind::Edges,
            mismatch.actual
        );
        OpError::new(
            OpErrorKind::PreconditionFailed,
            vec![Diagnostic::new(
                DiagLevel::Error,
                DiagCode::PreconditionFailed,
                message,
            )],
            Artifacts::default(),
        )
    })
}

fn require_vertex_selection(
    selection: &Selection,
    operator: &'static str,
) -> Result<VertexSet, OpError> {
    selection.require_vertices().cloned().map_err(|mismatch| {
        let message = alloc::format!(
            "{operator} requires {} selection, got {}",
            SelectionKind::Vertices,
            mismatch.actual
        );
        OpError::new(
            OpErrorKind::PreconditionFailed,
            vec![Diagnostic::new(
                DiagLevel::Error,
                DiagCode::PreconditionFailed,
                message,
            )],
            Artifacts::default(),
        )
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use exedra::{DeletePolicy, MeshBuilder};

    use super::MeshEdit;
    use crate::{
        CutRectFace, CutRectFaceParams, DeleteFaces, DeleteFacesParams, DissolveEdges,
        DissolveEdgesParams, DissolveVertices, DissolveVerticesParams, ExtrudeFaces,
        ExtrudeFacesParams, ExtrudeMode, FaceSet, InsetFaces, InsetFacesParams, OperatorRunner,
        Selection, SolidifyFaces, SolidifyFacesParams, SolidifyMode, TagFaceRegion,
        TagFaceRegionParams, mesh_signature, test_support::shared_edge_mesh,
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
            .delete_faces(DeletePolicy::KeepIsolated);

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
        let flow = MeshEdit::new().select(Selection::from(vec![face])).tag(7);

        let mut runner = OperatorRunner::new();
        let mut mesh = mesh;
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("flow should apply");
        assert!(matches!(result.selection, Selection::Faces(_)));

        let flow = MeshEdit::new()
            .select(Selection::from(vec![exedra::VertexId::new(
                0,
                core::num::NonZeroU32::MIN,
            )]))
            .tag(7);
        let mut runner = OperatorRunner::new();
        let err = flow
            .plan(&mut runner, &mesh)
            .expect_err("vertex selection should be rejected for face-domain tag op");
        assert_eq!(err.kind, crate::OpErrorKind::PreconditionFailed);
        assert!(
            err.diagnostics
                .iter()
                .any(|diag| diag.message.contains("requires faces selection"))
        );
    }

    #[test]
    fn mesh_edit_dissolve_edges_matches_direct_operator() {
        let (mesh, _, edge) = shared_edge_mesh();

        let flow = MeshEdit::new()
            .select(Selection::from(vec![edge]))
            .dissolve_edges();

        let mut flow_runner = OperatorRunner::new();
        let mut flow_mesh = mesh.clone();
        let flow_result = flow
            .apply(&mut flow_runner, &mut flow_mesh)
            .expect("fluent dissolve should succeed");

        let mut direct_runner = OperatorRunner::new();
        let mut direct_mesh = mesh.clone();
        let direct_plan = direct_runner
            .compile(
                &direct_mesh,
                &DissolveEdges,
                &DissolveEdgesParams { edges: vec![edge] },
            )
            .expect("direct dissolve compile should succeed");
        let direct = direct_runner
            .apply_in_place(&mut direct_mesh, &DissolveEdges, &direct_plan)
            .expect("direct dissolve should succeed");

        assert_eq!(mesh_signature(&flow_mesh), mesh_signature(&direct_mesh));
        assert_eq!(flow_result.selection, Selection::from(direct.output.faces));
    }

    #[test]
    fn mesh_edit_dissolve_vertices_matches_direct_operator() {
        let (mesh, _, edge) = shared_edge_mesh();
        let inserted = {
            let mut working = mesh.clone();
            let mut edit = working.edit();
            let inserted =
                exedra::op::split_edge(&mut edit, edge, &exedra::PropagatePolicy::default())
                    .expect("split should succeed");
            let _: () = edit.finish();
            (working, inserted)
        };
        let (mesh, inserted) = inserted;

        let flow = MeshEdit::new()
            .select(Selection::from(vec![inserted]))
            .dissolve_vertices();

        let mut flow_runner = OperatorRunner::new();
        let mut flow_mesh = mesh.clone();
        let flow_result = flow
            .apply(&mut flow_runner, &mut flow_mesh)
            .expect("fluent dissolve should succeed");

        let mut direct_runner = OperatorRunner::new();
        let mut direct_mesh = mesh.clone();
        let direct_plan = direct_runner
            .compile(
                &direct_mesh,
                &DissolveVertices,
                &DissolveVerticesParams {
                    vertices: vec![inserted],
                },
            )
            .expect("direct dissolve compile should succeed");
        let direct = direct_runner
            .apply_in_place(&mut direct_mesh, &DissolveVertices, &direct_plan)
            .expect("direct dissolve should succeed");

        assert_eq!(mesh_signature(&flow_mesh), mesh_signature(&direct_mesh));
        assert_eq!(flow_result.selection, Selection::from(direct.output.faces));
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
    fn mesh_edit_cut_rect_matches_manual_operator_sequence() {
        let (base, face) = one_quad_mesh();

        let flow = MeshEdit::new().select_faces(vec![face]).cut_rect(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.2, 0.25],
            [0.8, 0.75],
        );

        let mut flow_runner = OperatorRunner::new();
        let mut flow_mesh = base.clone();
        let flow_result = flow
            .apply(&mut flow_runner, &mut flow_mesh)
            .expect("fluent cut_rect flow should succeed");

        let mut direct_mesh = base.clone();
        let mut direct_runner = OperatorRunner::new();
        let params = CutRectFaceParams {
            face,
            frame_origin: [0.0, 0.0, 0.0],
            frame_u: [1.0, 0.0, 0.0],
            frame_v: [0.0, 1.0, 0.0],
            rect_min: [0.2, 0.25],
            rect_max: [0.8, 0.75],
        };
        let plan = direct_runner
            .compile(&direct_mesh, &CutRectFace, &params)
            .expect("cut_rect compile should succeed");
        let direct_result = direct_runner
            .apply_in_place(&mut direct_mesh, &CutRectFace, &plan)
            .expect("cut_rect apply should succeed");

        assert_eq!(mesh_signature(&flow_mesh), mesh_signature(&direct_mesh));
        assert_eq!(
            flow_result.selection,
            Selection::from(direct_result.output.inner_faces)
        );
    }

    #[test]
    fn mesh_edit_cut_rect_then_delete_faces_matches_manual_sequence() {
        let (base, face) = one_quad_mesh();

        let flow = MeshEdit::new()
            .select_faces(vec![face])
            .cut_rect(
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.2, 0.25],
                [0.8, 0.75],
            )
            .delete_faces(DeletePolicy::KeepIsolated);

        let mut flow_runner = OperatorRunner::new();
        let mut flow_mesh = base.clone();
        let flow_result = flow
            .apply(&mut flow_runner, &mut flow_mesh)
            .expect("fluent opening flow should succeed");

        let mut direct_mesh = base.clone();
        let mut direct_runner = OperatorRunner::new();
        let cut_params = CutRectFaceParams {
            face,
            frame_origin: [0.0, 0.0, 0.0],
            frame_u: [1.0, 0.0, 0.0],
            frame_v: [0.0, 1.0, 0.0],
            rect_min: [0.2, 0.25],
            rect_max: [0.8, 0.75],
        };
        let cut_plan = direct_runner
            .compile(&direct_mesh, &CutRectFace, &cut_params)
            .expect("cut_rect compile should succeed");
        let cut_result = direct_runner
            .apply_in_place(&mut direct_mesh, &CutRectFace, &cut_plan)
            .expect("cut_rect apply should succeed");

        let delete_params = DeleteFacesParams {
            faces: cut_result.output.inner_faces.clone(),
            policy: DeletePolicy::KeepIsolated,
        };
        let delete_plan = direct_runner
            .compile(&direct_mesh, &DeleteFaces, &delete_params)
            .expect("delete compile should succeed");
        let _ = direct_runner
            .apply_in_place(&mut direct_mesh, &DeleteFaces, &delete_plan)
            .expect("delete apply should succeed");

        assert_eq!(mesh_signature(&flow_mesh), mesh_signature(&direct_mesh));
        assert_eq!(flow_result.selection, Selection::from(FaceSet::new()));
    }

    #[test]
    fn mesh_edit_solidify_matches_manual_operator_sequence() {
        let (base, face) = one_quad_mesh();
        let flow = MeshEdit::new()
            .select_faces(vec![face])
            .solidify_with_mode(0.2, SolidifyMode::KeepSource);

        let mut flow_runner = OperatorRunner::new();
        let mut flow_mesh = base.clone();
        let flow_result = flow
            .apply(&mut flow_runner, &mut flow_mesh)
            .expect("fluent solidify should succeed");

        let mut direct_mesh = base.clone();
        let mut direct_runner = OperatorRunner::new();
        let params = SolidifyFacesParams {
            faces: vec![face],
            mode: SolidifyMode::KeepSource,
            thickness: 0.2,
        };
        let plan = direct_runner
            .compile(&direct_mesh, &SolidifyFaces, &params)
            .expect("solidify compile should succeed");
        let direct_result = direct_runner
            .apply_in_place(&mut direct_mesh, &SolidifyFaces, &plan)
            .expect("solidify apply should succeed");

        assert_eq!(mesh_signature(&flow_mesh), mesh_signature(&direct_mesh));
        assert_eq!(
            flow_result.selection,
            Selection::from(direct_result.output.cap_faces)
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

        let flow = MeshEdit::new().select_faces_by_region(7).tag(9);

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

        let flow = MeshEdit::new().flood_faces_by_region(faces[0]).tag(5);

        let mut runner = OperatorRunner::new();
        let mut mesh = mesh;
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("flow should succeed");
        assert!(matches!(result.selection, Selection::Faces(_)));
    }

    #[test]
    fn mesh_edit_boundary_query_step_can_feed_edge_ops() {
        let (mut mesh, face) = one_quad_mesh();
        let seed = mesh
            .face_loop(face)
            .next()
            .expect("quad face should have a boundary edge");
        let flow = MeshEdit::new()
            .select_boundary_edge_loop(seed)
            .mark_seam(true);
        let mut runner = OperatorRunner::new();
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("boundary-query flow should succeed");
        assert!(matches!(result.selection, Selection::Edges(ref edges) if !edges.is_empty()));
    }

    #[test]
    fn mesh_edit_edge_ops_accept_edge_selection() {
        let (mut mesh, shared, _twin) = shared_edge_mesh();
        let edge = mesh
            .canonical_edge(shared)
            .expect("shared edge should be live");
        let flow = MeshEdit::new()
            .select(Selection::from(vec![edge]))
            .mark_seam(true)
            .mark_sharp(2.0);

        let mut runner = OperatorRunner::new();
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("edge flow should succeed");
        assert!(matches!(result.selection, Selection::Edges(ref edges) if edges == &vec![edge]));
        assert_eq!(mesh.edge_seam(edge), Some(true));
        assert_eq!(mesh.edge_sharpness(edge), Some(2.0));
    }

    #[test]
    fn mesh_edit_edge_op_rejects_face_selection() {
        let (mesh, face) = one_quad_mesh();
        let flow = MeshEdit::new()
            .select(Selection::from(vec![face]))
            .mark_seam(true);
        let mut runner = OperatorRunner::new();
        let err = flow
            .plan(&mut runner, &mesh)
            .expect_err("face selection should be rejected for edge op");
        assert_eq!(err.kind, crate::OpErrorKind::PreconditionFailed);
        assert!(
            err.diagnostics
                .iter()
                .any(|diag| diag.message.contains("requires edges selection"))
        );
    }

    #[test]
    fn mesh_edit_delete_vertices_accepts_vertex_selection() {
        let mut mesh = exedra::Mesh::new();
        let v0 = {
            let mut txn = mesh.edit();
            let v0 = exedra::op::add_vertex(&mut txn, [0.0, 0.0, 0.0]);
            let _: () = txn.finish();
            v0
        };
        let flow = MeshEdit::new()
            .select(Selection::from(vec![v0]))
            .delete_vertices();

        let mut runner = OperatorRunner::new();
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("delete vertices flow should succeed");
        assert!(
            matches!(result.selection, Selection::Vertices(ref vertices) if vertices.is_empty())
        );
        assert_eq!(mesh.vertices().count(), 0);
    }

    #[test]
    fn mesh_edit_delete_edges_clears_selection() {
        let (mut mesh, shared, _twin) = shared_edge_mesh();
        let edge = mesh
            .canonical_edge(shared)
            .expect("shared edge should be live");
        let flow = MeshEdit::new()
            .select(Selection::from(vec![edge]))
            .delete_edges(DeletePolicy::KeepIsolated);
        let mut runner = OperatorRunner::new();
        let result = flow
            .apply(&mut runner, &mut mesh)
            .expect("delete edges flow should succeed");
        assert!(matches!(result.selection, Selection::Edges(ref edges) if edges.is_empty()));
    }
}
