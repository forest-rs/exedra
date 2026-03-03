// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator and growth layer for the exedra mesh kernel.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod artifact;
pub mod context;
pub mod diag;
pub mod dirty;
pub mod error;
pub mod operator;
pub mod policy;
pub mod region;
pub mod report;
pub mod runner;
pub mod selection;
mod timing;

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
pub use selection::{FaceSet, canonicalize_face_set};
