// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cambium: deterministic operator layer over Exedra.
//!
//! Cambium builds on Exedra with a curated operator/runtime API:
//! - operator trait and runner orchestration ([`EditOperator`], [`OperatorRunner`]),
//! - structured diagnostics/artifacts/reports,
//! - policy-controlled execution,
//! - deterministic region/selection helpers,
//! - deterministic UV and tagging operators.
//!
//! The intended public surface is this crate root (`cambium::...`) through
//! re-exported operator/runtime types and functions.
//!
//! For a longer operator-authoring guide, see [`manual`].
//!
//! # Typical Flow
//! ```rust
//! use cambium::{OperatorRunner, ValidateMesh, ValidateMeshMode, ValidateMeshParams};
//! use exedra::Mesh;
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
//! let preview = runner.run_preview(&mesh, &op, &params)?;
//! assert_eq!(preview.report.name, "inspect.validate.mesh");
//! # Ok::<(), cambium::OpError>(())
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("cambium requires either the `std` or `libm` feature");

mod artifact;
mod context;
mod diag;
mod dirty;
mod edge_mark;
mod error;
mod operator;
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
pub use context::{Clock, ClockBucket, OpContext, Scratch};
pub use diag::DEFAULT_MAX_DIAGNOSTICS;
pub use diag::{DiagCode, DiagLevel, DiagSpan, Diagnostic, DiagnosticsSink};
pub use dirty::{CacheDirtySet, DirtyChannel, DirtyKey};
pub use error::{OpError, OpErrorKind};
pub use operator::EditOperator;
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
pub use selection::{EdgeSet, FaceSet, canonicalize_edge_set, canonicalize_face_set};
pub use sharp::{MarkEdgeSharp, MarkEdgeSharpParams};
pub use uv_box::{UvBox, UvBoxParams};
pub use uv_cylinder::{CylinderAxis, UvCylinder, UvCylinderParams};
pub use uv_planar::{UvPlanar, UvPlanarParams, UvPlane, UvScope};
pub use validate::{ValidateMesh, ValidateMeshMode, ValidateMeshParams};
