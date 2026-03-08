// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator Catalog (v0.1)
//!
//! Curated catalog of the frozen v0.1 operator set.
//!
//! Conventions:
//! - all operators follow the explicit lifecycle:
//!   [`OperatorRunner::compile`](crate::OperatorRunner::compile) ->
//!   [`OperatorRunner::preview_on_clone`](crate::OperatorRunner::preview_on_clone) /
//!   [`OperatorRunner::apply_in_place`](crate::OperatorRunner::apply_in_place)
//! - `report.name` must match the stable `name()` in this catalog
//! - timings are best-effort and bucketed; counters/artifacts are deterministic
//!   and bounded
//!
//! # Families
//!
//! ## `inspect.*`
//!
//! | Stable Name | Operator | Params | Output | Reporting Notes |
//! | --- | --- | --- | --- | --- |
//! | `inspect.bounds` | [`InspectBounds`](crate::InspectBounds) | [`BoundsParams`](crate::BoundsParams) | [`BoundsOutput`](crate::BoundsOutput) | Primarily counters + summary artifacts; read-only inspection. |
//! | `inspect.select.summary` | [`InspectSelectionSummary`](crate::InspectSelectionSummary) | [`SelectionSummaryParams`](crate::SelectionSummaryParams) | [`SelectionSummaryOutput`](crate::SelectionSummaryOutput) | Summarizes selection liveness/domain counts without mutation. |
//! | `inspect.validate.mesh` | [`ValidateMesh`](crate::ValidateMesh) | [`ValidateMeshParams`](crate::ValidateMeshParams) | [`ValidateMeshOutput`](crate::ValidateMeshOutput) | Validation diagnostics plus summary counters. |
//!
//! ## `select.*`
//!
//! | Stable Name | API | Params | Output | Reporting Notes |
//! | --- | --- | --- | --- | --- |
//! | `select.faces.by_region` | [`select_faces_by_region`](crate::select_faces_by_region) | `(&Mesh, region_id)` | [`RegionSelection`](crate::RegionSelection) | Query helper (not an `EditOperator`), deterministic selection/counters. |
//! | `select.edgeloop.boundary` | [`SelectBoundaryEdgeLoop`](crate::SelectBoundaryEdgeLoop) | [`SelectBoundaryEdgeLoopParams`](crate::SelectBoundaryEdgeLoopParams) | [`EdgeSet`](crate::EdgeSet) | Deterministic boundary-loop query wrapped as `EditOperator`. |
//! | `select.faces.flood.region` | [`flood_fill_faces_by_region`](crate::flood_fill_faces_by_region) | `(&Mesh, seed_face)` | [`RegionFloodSelection`](crate::RegionFloodSelection) | Connected component query constrained to the seed face's region tag. |
//!
//! ## `tag.*`
//!
//! | Stable Name | Operator | Params | Output | Reporting Notes |
//! | --- | --- | --- | --- | --- |
//! | `tag.face.region` | [`TagFaceRegion`](crate::TagFaceRegion) | [`TagFaceRegionParams`](crate::TagFaceRegionParams) | [`FaceSet`](crate::FaceSet) | Tracks canonicalization + faces processed. |
//!
//! ## `mark.*`
//!
//! | Stable Name | Operator | Params | Output | Reporting Notes |
//! | --- | --- | --- | --- | --- |
//! | `mark.edge.seam` | [`MarkEdgeSeam`](crate::MarkEdgeSeam) | [`MarkEdgeSeamParams`](crate::MarkEdgeSeamParams) | [`EdgeSet`](crate::EdgeSet) | Uses edge-domain counters (`edges_written`). |
//! | `mark.edge.sharp` | [`MarkEdgeSharp`](crate::MarkEdgeSharp) | [`MarkEdgeSharpParams`](crate::MarkEdgeSharpParams) | [`EdgeSet`](crate::EdgeSet) | Uses edge-domain counters (`edges_written`). |
//!
//! ## `uv.*`
//!
//! | Stable Name | Operator | Params | Output | Reporting Notes |
//! | --- | --- | --- | --- | --- |
//! | `uv.planar` | [`UvPlanar`](crate::UvPlanar) | [`UvPlanarParams`](crate::UvPlanarParams) | [`FaceSet`](crate::FaceSet) | Typical timing buckets: `select`, `compute`, `attrs`. |
//! | `uv.box` | [`UvBox`](crate::UvBox) | [`UvBoxParams`](crate::UvBoxParams) | [`FaceSet`](crate::FaceSet) | Deterministic affected-face reporting. |
//! | `uv.cylinder` | [`UvCylinder`](crate::UvCylinder) | [`UvCylinderParams`](crate::UvCylinderParams) | [`FaceSet`](crate::FaceSet) | Deterministic affected-face reporting. |
//!
//! ## `edit.*`
//!
//! | Stable Name | Operator | Params | Output | Reporting Notes |
//! | --- | --- | --- | --- | --- |
//! | `edit.delete.faces` | [`DeleteFaces`](crate::DeleteFaces) | [`DeleteFacesParams`](crate::DeleteFacesParams) | [`DeleteFacesOutput`](crate::DeleteFacesOutput) | Deletion counters + selection canonicalization. |
//! | `edit.delete.edges` | [`DeleteEdges`](crate::DeleteEdges) | [`DeleteEdgesParams`](crate::DeleteEdgesParams) | [`DeleteEdgesOutput`](crate::DeleteEdgesOutput) | Includes impacted face set in output. |
//! | `edit.dissolve.edges` | [`DissolveEdges`](crate::DissolveEdges) | [`DissolveEdgesParams`](crate::DissolveEdgesParams) | [`DissolveEdgesOutput`](crate::DissolveEdgesOutput) | Merges adjacent face pairs and hands off merged faces for follow-on modeling. |
//! | `edit.delete.vertices` | [`DeleteVertices`](crate::DeleteVertices) | [`DeleteVerticesParams`](crate::DeleteVerticesParams) | [`DeleteVerticesOutput`](crate::DeleteVerticesOutput) | Valid only for isolated vertices. |
//! | `edit.dissolve.vertices` | [`DissolveVertices`](crate::DissolveVertices) | [`DissolveVerticesParams`](crate::DissolveVerticesParams) | [`DissolveVerticesOutput`](crate::DissolveVerticesOutput) | Dissolves interior valence-2 vertices and hands off rebuilt faces. |
//! | `edit.face.cut.rect` | [`CutRectFace`](crate::CutRectFace) | [`CutRectFaceParams`](crate::CutRectFaceParams) | [`CutRectFaceOutput`](crate::CutRectFaceOutput) | Cuts one rectangular inner face from one quad face for opening workflows. |
//! | `edit.face.extrude` | [`ExtrudeFaces`](crate::ExtrudeFaces) | [`ExtrudeFacesParams`](crate::ExtrudeFacesParams) (`mode`: [`ExtrudeMode`](crate::ExtrudeMode)) | [`ExtrudeFacesOutput`](crate::ExtrudeFacesOutput) | Face-edit counters + generated cap/wall outputs; mode controls source-face retention. |
//! | `edit.face.inset` | [`InsetFaces`](crate::InsetFaces) | [`InsetFacesParams`](crate::InsetFacesParams) | [`InsetFacesOutput`](crate::InsetFacesOutput) | Face-edit counters + generated inner/frame outputs. |
//! | `edit.face.poke` | [`PokeFaces`](crate::PokeFaces) | [`PokeFacesParams`](crate::PokeFacesParams) | [`PokeFacesOutput`](crate::PokeFacesOutput) | Replaces each selected face with a triangle fan around one new center vertex. |
//! | `edit.face.solidify` | [`SolidifyFaces`](crate::SolidifyFaces) | [`SolidifyFacesParams`](crate::SolidifyFacesParams) (`mode`: [`SolidifyMode`](crate::SolidifyMode)) | [`SolidifyFacesOutput`](crate::SolidifyFacesOutput) | Explicit shell-thickness operation over selected faces. |
//!
//! Additional normal-editing operators:
//! - `edit.normal.clear`: [`ClearCornerNormals`](crate::ClearCornerNormals)
//! - `edit.normal.face`: [`BakeFaceNormals`](crate::BakeFaceNormals)
//! - `edit.normal.average`: [`BakeDerivedNormals`](crate::BakeDerivedNormals)
//!
//! # Fluent Chains
//!
//! For user-facing multi-step modeling flows, prefer [`MeshEdit`](crate::MeshEdit)
//! over manually threading operator outputs:
//! - "boxy hat": `select_faces -> extrude -> inset`
//! - "wall opening": `select_faces -> cut_rect -> delete_faces`
//! - shelling: `select_faces -> solidify`
//!
//! `MeshEdit` preserves deterministic compile/preview/apply behavior while
//! making the selection handoff between these face-edit operators explicit.
//!
//! # Reporting Expectations
//!
//! All catalog operators should:
//! - return bounded artifacts via [`Artifacts`](crate::Artifacts)
//! - keep counters deterministic and monotonic within one run
//! - use stable timing bucket names for major phases
//! - attach diagnostics through [`DiagnosticsSink`](crate::DiagnosticsSink)
//!
//! See:
//! - [`operators`](super::operators) for authoring rules
//! - [`reporting`](super::reporting) for timings/stats/artifact patterns
//! - `docs/briefs/09_operator_taxonomy_v01_freeze.md` for frozen v0.1 scope
