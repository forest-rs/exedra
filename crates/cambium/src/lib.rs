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
//! - typed analytic workflow helpers ([`analytic`]),
//! - explicit cross-domain conversion helpers ([`convert`]),
//! - deterministic UV/tagging operators,
//! - basic face-edit operators ([`ExtrudeFaces`], [`InsetFaces`], [`PokeFaces`]).
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
//! For manual docs, see:
//! - [`manual::catalog`] for the frozen v0.1 operator catalog
//! - [`manual::operators`] for operator authoring guidance
//!
//! # Where To Find X
//! - Mesh + IDs: [`Mesh`], [`FaceId`], [`HalfEdgeId`], [`VertexId`]
//! - Build/extract helpers: [`BuildParams`], [`ExtractParams`], [`TriMesh`]
//! - Planning lifecycle: [`OperatorRunner::compile`],
//!   [`OperatorRunner::preview_on_clone`], [`OperatorRunner::apply_in_place`]
//! - Operator domains: [`OperatorDomain`]
//! - Analytic edits: [`analytic::set_face_region`], [`analytic::add_rect_opening_xy`]
//! - Explicit conversions: [`convert::analytic_shell_to_mesh`], [`convert::rect_frame_to_mesh`]
//! - Selection tools: [`FaceSet`], [`EdgeSet`], [`VertexSet`]
//! - Fluent workflows: [`MeshEdit`], [`MeshEditPlan`]
//! - Operator families:
//!   - `inspect.*`: [`InspectBounds`], [`InspectSelectionSummary`], [`ValidateMesh`]
//!   - `select.*`: [`SelectBoundaryEdgeLoop`], [`select_faces_by_region`], [`select_boundary_edge_loop`], [`flood_fill_faces_by_region`]
//!   - `tag.*`: [`TagFaceRegion`]
//!   - `mark.*`: [`MarkEdgeSeam`], [`MarkEdgeSharp`]
//!   - `uv.*`: [`UvPlanar`], [`UvBox`], [`UvCylinder`]
//!   - `edit.*`: [`DeleteFaces`], [`DeleteEdges`], [`DissolveEdges`], [`DeleteVertices`], [`DissolveVertices`], [`BridgeBoundaryLoops`], [`CutRectFace`], [`ExtrudeFaces`], [`InsetFaces`], [`PokeFaces`], [`SolidifyFaces`], [`ClearCornerNormals`], [`BakeFaceNormals`], [`BakeDerivedNormals`], [`SmoothFaceNormals`]
//! - Operator catalog + authoring map: [`manual::catalog`], [`manual::operators`]
//!
//! # Namespace Conventions
//! Operator stable IDs follow lowercase dot-separated families from the v0.1
//! taxonomy:
//! - `inspect.*`
//! - `select.*`
//! - `tag.*`
//! - `mark.*`
//! - `uv.*`
//! - `edit.*`
//!
//! See `docs/briefs/09_operator_taxonomy_v01_freeze.md` and
//! `docs/briefs/10_operator_naming_conventions.md` for the canonical contract.
//!
//! # Typical Flow
//! ```rust
//! use cambium::{
//!     EditOperator, Mesh, OperatorRunner, ValidateMesh, ValidateMeshMode, ValidateMeshParams,
//! };
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
//! assert_eq!(op.domain(), cambium::OperatorDomain::Mesh);
//! # Ok::<(), cambium::OpError>(())
//! ```
//!
//! # Migration Note
//! `run_commit` / `run_preview` were removed in favor of explicit lifecycle
//! calls: `compile` -> `apply_in_place` / `preview_on_clone`.
//!
//! `EditOperator::domain()` and [`OperatorDomain`] are additive metadata used by
//! the multi-domain architecture work. Existing mesh operators continue to
//! default to [`OperatorDomain::Mesh`].
//!
//! Analytic mutation currently lives in [`analytic`] helper APIs and
//! conversion stays explicit in [`convert`] rather than being forced through
//! the mesh-only operator runtime.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("cambium requires either the `std` or `libm` feature");

pub mod analytic;
mod artifact;
mod bounds;
mod bridge;
mod context;
pub mod convert;
mod delete;
mod diag;
mod dirty;
mod edge_mark;
mod error;
mod face_edit;
mod inspect_selection;
mod op_common;
mod operator;
mod patch;
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
mod normal_edit;
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

// Runtime and execution surface.
pub use context::{Clock, ClockBucket, OpContext, Scratch};
pub use diag::DEFAULT_MAX_DIAGNOSTICS;
pub use diag::{DiagCode, DiagLevel, DiagSpan, Diagnostic, DiagnosticsSink};
pub use dirty::{CacheDirtySet, DirtyChannel, DirtyKey};
pub use error::{OpError, OpErrorKind};
pub use operator::{EditOperator, OperatorDomain};
pub use plan::{
    EditPlan, PlanFingerprint, PlanSourceState, mesh_signature, mesh_topology_signature,
};
pub use report::{ElementCounts, OpReport, SmallCounters, Stats, TimeBucket, Timings};
pub use runner::{OpResult, OperatorRunner, PreviewResult};

// Shared mesh/kernel types re-exported from Exedra.
pub use exedra::{
    BuildParams, CornerId, DeletePolicy, ExtractParams, FaceId, HalfEdgeId, Mesh, MeshBuilder,
    NormalParams, NormalWeightMode, NormalsSource, TriMesh, VertexId,
};

// Analytic workflow surface.
pub use analytic::{
    ADD_RECT_OPENING_XY_NAME, AddRectOpeningOutput, AddRectOpeningParams, AnalyticEditError,
    AnalyticFaceId, AnalyticLoopId, AnalyticRegionId, AnalyticShell, RectOpeningParams,
    SET_FACE_REGION_NAME, SetFaceRegionOutput, SetFaceRegionParams, add_rect_opening_xy,
    set_face_region,
};

// Policies and selection surface.
pub use bounds::{BoundsOutput, BoundsParams, BoundsScope, BoundsSummary, InspectBounds};
pub use bridge::{
    BridgeBoundaryLoops, BridgeBoundaryLoopsOutput, BridgeBoundaryLoopsParams,
    BridgeBoundaryLoopsPlan,
};
pub use inspect_selection::{
    InspectSelectionSummary, SelectionSummaryDetail, SelectionSummaryOutput, SelectionSummaryParams,
};
pub use policy::{
    BooleanParams, BooleanPolicy, LimitsPolicy, PolicySet, PropagatePolicy, QualityMode,
    QualityPolicy, UvPolicy, ValidatePolicy, WorkBudget,
};
pub use selection::{
    EdgeSet, FaceSet, Selection, SelectionDomainError, SelectionKind, VertexSet,
    canonicalize_edge_set, canonicalize_face_set, canonicalize_vertex_set,
};

// Fluent workflow layer.
pub use mesh_edit::{
    MeshEdit, MeshEditPlan, MeshEditPreview, MeshEditResult, MeshEditStepPlan, SelectionStepPlan,
};

// Operator families (`inspect.*`, `select.*`, `tag.*`, `mark.*`, `uv.*`, `edit.*`).
pub use delete::{
    DeleteEdges, DeleteEdgesOutput, DeleteEdgesParams, DeleteFaces, DeleteFacesOutput,
    DeleteFacesParams, DeleteFacesPlan, DeleteVertices, DeleteVerticesOutput, DeleteVerticesParams,
    DissolveEdges, DissolveEdgesOutput, DissolveEdgesParams, DissolveVertices,
    DissolveVerticesOutput, DissolveVerticesParams,
};
pub use face_edit::{
    CutRectFace, CutRectFaceOutput, CutRectFaceParams, CutRectFacePlan, ExtrudeFaces,
    ExtrudeFacesOutput, ExtrudeFacesParams, ExtrudeMode, InsetFaces, InsetFacesOutput,
    InsetFacesParams, InsetFacesPlan, PokeFaces, PokeFacesOutput, PokeFacesParams, PokeFacesPlan,
    SolidifyFaces, SolidifyFacesOutput, SolidifyFacesParams, SolidifyMode,
};
pub use normal_edit::{
    BakeDerivedNormals, BakeDerivedNormalsParams, BakeDerivedNormalsPlan, BakeFaceNormals,
    BakeFaceNormalsParams, BakeFaceNormalsPlan, ClearCornerNormals, ClearCornerNormalsParams,
    ClearCornerNormalsPlan, NormalFacesOutput, SmoothFaceNormals, SmoothFaceNormalsParams,
    SmoothFaceNormalsPlan,
};
pub use region::{
    EdgeLoopSelection, REGION_UNTAGGED, RegionFloodSelection, RegionSelection,
    SelectBoundaryEdgeLoop, SelectBoundaryEdgeLoopParams, TagFaceRegion, TagFaceRegionParams,
    flood_fill_faces_by_region, select_boundary_edge_loop, select_faces_by_region,
};
pub use seam::{MarkEdgeSeam, MarkEdgeSeamParams};
pub use sharp::{MarkEdgeSharp, MarkEdgeSharpParams};
pub use uv_box::{UvBox, UvBoxParams};
pub use uv_cylinder::{CylinderAxis, UvCylinder, UvCylinderParams};
pub use uv_planar::{UvPlanar, UvPlanarParams, UvPlane, UvScope};
pub use validate::{ValidateMesh, ValidateMeshMode, ValidateMeshOutput, ValidateMeshParams};

#[cfg(test)]
mod naming_tests {
    use alloc::collections::BTreeSet;

    use super::{
        BakeDerivedNormals, BakeFaceNormals, BridgeBoundaryLoops, ClearCornerNormals, CutRectFace,
        DeleteEdges, DeleteFaces, DeleteVertices, DissolveEdges, DissolveVertices, EditOperator,
        ExtrudeFaces, InsetFaces, InspectBounds, InspectSelectionSummary, MarkEdgeSeam,
        MarkEdgeSharp, OperatorDomain, PokeFaces, SelectBoundaryEdgeLoop, SmoothFaceNormals,
        SolidifyFaces, TagFaceRegion, UvBox, UvCylinder, UvPlanar, ValidateMesh,
    };

    #[test]
    fn operator_names_are_unique_and_use_allowed_families() {
        let names = [
            DeleteEdges.name(),
            DissolveEdges.name(),
            DeleteFaces.name(),
            DeleteVertices.name(),
            DissolveVertices.name(),
            BridgeBoundaryLoops.name(),
            CutRectFace.name(),
            ExtrudeFaces.name(),
            InsetFaces.name(),
            PokeFaces.name(),
            SolidifyFaces.name(),
            InspectBounds.name(),
            InspectSelectionSummary.name(),
            ValidateMesh.name(),
            MarkEdgeSeam.name(),
            MarkEdgeSharp.name(),
            ClearCornerNormals.name(),
            BakeFaceNormals.name(),
            BakeDerivedNormals.name(),
            SmoothFaceNormals.name(),
            TagFaceRegion.name(),
            SelectBoundaryEdgeLoop.name(),
            UvPlanar.name(),
            UvBox.name(),
            UvCylinder.name(),
        ];

        let unique = names.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), names.len(), "operator names must be unique");

        for &name in &names {
            let prefix = name
                .split('.')
                .next()
                .expect("operator name must be non-empty");
            assert!(
                matches!(
                    prefix,
                    "edit" | "inspect" | "mark" | "select" | "tag" | "uv"
                ),
                "unexpected operator family prefix in `{name}`"
            );
            assert!(
                name.chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.'),
                "operator names must be lowercase ascii dot-separated: `{name}`"
            );
        }
    }

    #[test]
    fn frozen_v01_operator_names_match_contract() {
        let actual = [
            DeleteEdges.name(),
            DissolveEdges.name(),
            DeleteFaces.name(),
            DeleteVertices.name(),
            DissolveVertices.name(),
            BridgeBoundaryLoops.name(),
            CutRectFace.name(),
            ExtrudeFaces.name(),
            InsetFaces.name(),
            PokeFaces.name(),
            SolidifyFaces.name(),
            InspectBounds.name(),
            InspectSelectionSummary.name(),
            ValidateMesh.name(),
            MarkEdgeSeam.name(),
            MarkEdgeSharp.name(),
            TagFaceRegion.name(),
            SelectBoundaryEdgeLoop.name(),
            UvPlanar.name(),
            UvBox.name(),
            UvCylinder.name(),
        ];
        let expected = [
            "edit.delete.edges",
            "edit.dissolve.edges",
            "edit.delete.faces",
            "edit.delete.vertices",
            "edit.dissolve.vertices",
            "edit.bridge.loops",
            "edit.face.cut.rect",
            "edit.face.extrude",
            "edit.face.inset",
            "edit.face.poke",
            "edit.face.solidify",
            "inspect.bounds",
            "inspect.select.summary",
            "inspect.validate.mesh",
            "mark.edge.seam",
            "mark.edge.sharp",
            "tag.face.region",
            "select.edgeloop.boundary",
            "uv.planar",
            "uv.box",
            "uv.cylinder",
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn frozen_v01_operator_domains_are_mesh() {
        let domains = [
            DeleteEdges.domain(),
            DissolveEdges.domain(),
            DeleteFaces.domain(),
            DeleteVertices.domain(),
            DissolveVertices.domain(),
            BridgeBoundaryLoops.domain(),
            CutRectFace.domain(),
            ExtrudeFaces.domain(),
            InsetFaces.domain(),
            PokeFaces.domain(),
            SolidifyFaces.domain(),
            InspectBounds.domain(),
            InspectSelectionSummary.domain(),
            ValidateMesh.domain(),
            MarkEdgeSeam.domain(),
            MarkEdgeSharp.domain(),
            ClearCornerNormals.domain(),
            BakeFaceNormals.domain(),
            BakeDerivedNormals.domain(),
            SmoothFaceNormals.domain(),
            TagFaceRegion.domain(),
            SelectBoundaryEdgeLoop.domain(),
            UvPlanar.domain(),
            UvBox.domain(),
            UvCylinder.domain(),
        ];

        assert!(domains.iter().all(|domain| *domain == OperatorDomain::Mesh));
    }
}
