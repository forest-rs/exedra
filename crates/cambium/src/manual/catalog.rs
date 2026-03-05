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
//! | `inspect.validate.mesh` | [`ValidateMesh`](crate::ValidateMesh) | [`ValidateMeshParams`](crate::ValidateMeshParams) | [`ValidateMeshOutput`](crate::ValidateMeshOutput) | Validation diagnostics plus summary counters. |
//!
//! ## `select.*`
//!
//! | Stable Name | API | Params | Output | Reporting Notes |
//! | --- | --- | --- | --- | --- |
//! | `select.faces.by_region` | [`select_faces_by_region`](crate::select_faces_by_region) | `(&Mesh, region_id)` | [`RegionSelection`](crate::RegionSelection) | Query helper (not an `EditOperator`), deterministic selection/counters. |
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
//! | `edit.delete.vertices` | [`DeleteVertices`](crate::DeleteVertices) | [`DeleteVerticesParams`](crate::DeleteVerticesParams) | [`DeleteVerticesOutput`](crate::DeleteVerticesOutput) | Valid only for isolated vertices. |
//! | `edit.face.extrude` | [`ExtrudeFaces`](crate::ExtrudeFaces) | [`ExtrudeFacesParams`](crate::ExtrudeFacesParams) | [`ExtrudeFacesOutput`](crate::ExtrudeFacesOutput) | Face-edit counters + generated cap/wall outputs. |
//! | `edit.face.inset` | [`InsetFaces`](crate::InsetFaces) | [`InsetFacesParams`](crate::InsetFacesParams) | [`InsetFacesOutput`](crate::InsetFacesOutput) | Face-edit counters + generated inner/frame outputs. |
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
