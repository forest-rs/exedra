// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator and growth layer for the exedra mesh kernel.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("cambium requires either the `std` or `libm` feature");

pub mod artifact;
pub mod context;
pub mod diag;
pub mod dirty;
mod edge_mark;
pub mod error;
pub mod operator;
pub mod policy;
pub mod region;
pub mod report;
pub mod runner;
pub mod seam;
pub mod selection;

mod math;
#[cfg(test)]
mod test_support;
mod timing;
mod uv_common;

pub mod uv_box;
pub mod uv_cylinder;
pub mod uv_planar;
pub mod validate;

pub use artifact::{Artifact, Artifacts};
pub use context::{Clock, ClockBucket, OpContext, Scratch};
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
pub use uv_box::{UvBox, UvBoxParams};
pub use uv_cylinder::{CylinderAxis, UvCylinder, UvCylinderParams};
pub use uv_planar::{UvPlanar, UvPlanarParams, UvPlane, UvScope};
pub use validate::{ValidateMesh, ValidateMeshMode, ValidateMeshParams};
