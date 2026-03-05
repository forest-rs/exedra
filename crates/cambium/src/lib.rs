// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cambium: deterministic operator layer over Exedra.
//!
//! Cambium is the workflow-facing SDK tier for mesh operations.
//!
//! Cambium builds on Exedra with a curated operator/runtime API:
//! - operator trait and runner orchestration ([`EditOperator`], [`OperatorRunner`]),
//! - structured diagnostics/artifacts/reports,
//! - policy-controlled execution,
//! - deterministic region/selection helpers,
//! - deterministic UV/tagging operators,
//! - basic face-edit operators ([`ExtrudeFaces`], [`InsetFaces`]).
//!
//! API tiers:
//! - SDK tier (`cambium`): workflows, operators, planning lifecycle, reporting.
//! - Engine tier (`exedra`): topology/attributes kernel and invariants.
//!   Workflow users should start in `cambium::...`.
//!
//! Tagging note:
//! - [`MarkEdgeSharp`] / [`MarkEdgeSharpParams`] use numeric `f32`
//!   sharpness (`0.0` smooth, `> 0.0` sharp),
//! - [`MarkEdgeSeam`] / [`MarkEdgeSeamParams`] use boolean seam tags.
//!
//! The intended public surface is this crate root (`cambium::...`) through
//! re-exported operator/runtime types and functions.
//!
//! For a longer operator-authoring guide, see [`manual`].
//!
//! # Where To Find X
//! - Mesh + IDs: [`Mesh`], [`FaceId`], [`HalfEdgeId`], [`VertexId`]
//! - Build/extract helpers: [`BuildParams`], [`ExtractParams`], [`TriMesh`]
//! - Planning lifecycle: [`OperatorRunner::compile`],
//!   [`OperatorRunner::preview_on_clone`], [`OperatorRunner::apply_in_place`]
//! - Selection tools: [`FaceSet`], [`EdgeSet`], [`VertexSet`]
//! - Fluent workflows: [`MeshEdit`], [`MeshEditPlan`]
//! - Inspection: [`InspectBounds`], [`ValidateMesh`]
//! - Editing operators: [`DeleteFaces`], [`DeleteEdges`], [`DeleteVertices`],
//!   [`ExtrudeFaces`], [`InsetFaces`], [`TagFaceRegion`], [`MarkEdgeSeam`], [`MarkEdgeSharp`]
//! - UV operators: [`UvPlanar`], [`UvBox`], [`UvCylinder`]
//!
//! # Typical Flow
//! ```rust
//! use cambium::{Mesh, OperatorRunner, ValidateMesh, ValidateMeshMode, ValidateMeshParams};
//!
//! // Start from any Exedra mesh (empty is fine for this flow example).
//! let mesh = Mesh::new();
//! // Reuse one runner/context across operator invocations.
//! let mut runner = OperatorRunner::new();
//! let op = ValidateMesh;
//! let params = ValidateMeshParams {
//!     mode: ValidateMeshMode::FastAndDeep,
//! };
//!
//! // Preview runs on a clone and leaves `mesh` unchanged.
//! let plan = runner.compile(&mesh, &op, &params)?;
//! let preview = runner.preview_on_clone(&mesh, &op, &plan)?;
//! assert_eq!(preview.report.name, "inspect.validate.mesh");
//! # Ok::<(), cambium::OpError>(())
//! ```
//!
//! # Migration Note
//! `run_commit` / `run_preview` were removed in favor of explicit lifecycle
//! calls: `compile` -> `apply_in_place` / `preview_on_clone`.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("cambium requires either the `std` or `libm` feature");

mod artifact;
mod bounds;
mod context;
mod delete;
mod diag;
mod dirty;
mod edge_mark;
mod error;
mod face_edit;
mod op_common;
mod operator;
mod plan;
mod policy;
mod region;
mod report;
mod runner;
mod seam;
mod selection;
mod sharp;

#[cfg(doc)]
pub mod manual;
mod math;
mod mesh_edit;
#[cfg(test)]
mod test_support;
mod timing;
mod uv_common;

mod uv_box;
mod uv_cylinder;
mod uv_planar;
mod validate;

pub use artifact::{Artifact, Artifacts};
pub use artifact::{DEFAULT_MAX_ARTIFACT_BYTES, DEFAULT_MAX_ARTIFACT_ITEMS};
pub use bounds::{BoundsOutput, BoundsParams, BoundsScope, BoundsSummary, InspectBounds};
pub use context::{Clock, ClockBucket, OpContext, Scratch};
pub use delete::{
    DeleteEdges, DeleteEdgesOutput, DeleteEdgesParams, DeleteFaces, DeleteFacesOutput,
    DeleteFacesParams, DeleteFacesPlan, DeleteVertices, DeleteVerticesOutput, DeleteVerticesParams,
};
pub use diag::DEFAULT_MAX_DIAGNOSTICS;
pub use diag::{DiagCode, DiagLevel, DiagSpan, Diagnostic, DiagnosticsSink};
pub use dirty::{CacheDirtySet, DirtyChannel, DirtyKey};
pub use error::{OpError, OpErrorKind};
pub use exedra::{
    BuildParams, CornerId, DeletePolicy, ExtractParams, FaceId, HalfEdgeId, Mesh, MeshBuilder,
    TriMesh, VertexId,
};
pub use face_edit::{
    ExtrudeFaces, ExtrudeFacesOutput, ExtrudeFacesParams, InsetFaces, InsetFacesOutput,
    InsetFacesParams, InsetFacesPlan,
};
pub use mesh_edit::{MeshEdit, MeshEditPlan, MeshEditPreview, MeshEditResult, MeshEditStepPlan};
pub use operator::EditOperator;
pub use plan::{EditPlan, PlanFingerprint, mesh_signature};
pub use policy::{
    BooleanParams, BooleanPolicy, LimitsPolicy, PolicySet, PropagatePolicy, QualityMode,
    QualityPolicy, UvPolicy, ValidatePolicy, WorkBudget,
};
pub use region::{
    REGION_UNTAGGED, RegionSelection, TagFaceRegion, TagFaceRegionParams, select_faces_by_region,
};
pub use report::{ElementCounts, OpReport, SmallCounters, Stats, TimeBucket, Timings};
pub use runner::{OpResult, OperatorRunner, PreviewResult};
pub use seam::{MarkEdgeSeam, MarkEdgeSeamParams};
pub use selection::{
    EdgeSet, FaceSet, Selection, SelectionDomainError, SelectionKind, VertexSet,
    canonicalize_edge_set, canonicalize_face_set, canonicalize_vertex_set,
};
pub use sharp::{MarkEdgeSharp, MarkEdgeSharpParams};
pub use uv_box::{UvBox, UvBoxParams};
pub use uv_cylinder::{CylinderAxis, UvCylinder, UvCylinderParams};
pub use uv_planar::{UvPlanar, UvPlanarParams, UvPlane, UvScope};
pub use validate::{ValidateMesh, ValidateMeshMode, ValidateMeshOutput, ValidateMeshParams};
