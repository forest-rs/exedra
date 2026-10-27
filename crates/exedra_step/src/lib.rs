// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! STEP exchange of retained analytic boundaries, independent of display meshes.
//!
//! The exporter owns serialization, explicit exchange units and bounded output.
//! The analytic head owns geometry and incidence; callers own scene hierarchy,
//! feature metadata and occurrence placement. This first family exports one
//! connected closed shell of planes/cylinders bounded by lines/circles.
//!
//! Input incidence and conservative support agreement are checked. Those checks
//! do not certify global solid validity, trim nesting or shell containment.
//! The declared schema is AP214 `AUTOMOTIVE_DESIGN`; an independent STEP reader
//! is an integration validation tool rather than a core dependency.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

use alloc::{string::String, vec::Vec};
use exedra_analytic::body::{Body, BodyError, EdgeId, FaceId, LoopId, VertexId};
use exedra_math::{FrameError, Placement3};

mod preflight;
mod write;

/// Declared STEP application schema, using its advanced `BRep` entities.
pub const SCHEMA: &str = "AUTOMOTIVE_DESIGN";

/// Meaning of one input coordinate unit. Export does not rescale coordinates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LengthUnit {
    /// One coordinate unit is one millimetre.
    Millimetre,
    /// One coordinate unit is one metre.
    Metre,
}
impl LengthUnit {
    /// Physical metres represented by one coordinate unit.
    #[must_use]
    pub const fn metres_per_unit(self) -> f64 {
        match self {
            Self::Millimetre => 0.001,
            Self::Metre => 1.0,
        }
    }
}

/// Explicit exchange metadata, native agreement tolerance and output work limits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct StepOptions<'a> {
    /// Human-readable product and representation name; Unicode is encoded as STEP text.
    pub name: &'a str,
    /// Caller-supplied ISO 8601 creation timestamp. The exporter has no clock.
    pub timestamp: &'a str,
    /// Meaning of source coordinates and every reported length.
    pub units: LengthUnit,
    /// Maximum conservative native support gap plus export frame/coordinate allowance.
    /// Also emitted as the representation's declared length uncertainty.
    pub tolerance: f64,
    /// Largest entity count and maximum input-table work budget.
    pub max_entities: u32,
    /// Largest complete UTF-8/ASCII STEP text buffer, including header and trailer.
    pub max_bytes: usize,
}
impl Default for StepOptions<'_> {
    fn default() -> Self {
        Self {
            name: "Exedra",
            timestamp: "1970-01-01T00:00:00Z",
            units: LengthUnit::Millimetre,
            tolerance: 1e-7,
            max_entities: 1_000_000,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A positive entity identifier local to one complete STEP document.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepEntityId(pub u32);

/// Native table indices mapped to exported topological entity IDs.
/// Unused native entries are `None`; identities are local to this source/document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepCorrespondence {
    /// Native vertices to `VERTEX_POINT` entities.
    pub vertices: Vec<Option<StepEntityId>>,
    /// Native edges to shared `EDGE_CURVE` entities.
    pub edges: Vec<Option<StepEntityId>>,
    /// Native loops to `EDGE_LOOP` entities.
    pub loops: Vec<Option<StepEntityId>>,
    /// Native faces to `ADVANCED_FACE` entities.
    pub faces: Vec<Option<StepEntityId>>,
}
impl StepCorrespondence {
    /// Exported topological entity for a native vertex, if used.
    #[must_use]
    pub fn vertex(&self, id: VertexId) -> Option<StepEntityId> {
        self.vertices.get(id.index()).copied().flatten()
    }
    /// Exported shared curve edge for a native edge, if used.
    #[must_use]
    pub fn edge(&self, id: EdgeId) -> Option<StepEntityId> {
        self.edges.get(id.index()).copied().flatten()
    }
    /// Exported loop for a native loop, if used.
    #[must_use]
    pub fn boundary_loop(&self, id: LoopId) -> Option<StepEntityId> {
        self.loops.get(id.index()).copied().flatten()
    }
    /// Exported supporting face for a native face, if used.
    #[must_use]
    pub fn face(&self, id: FaceId) -> Option<StepEntityId> {
        self.faces.get(id.index()).copied().flatten()
    }
}

/// Work actually represented in the complete output document.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StepCounts {
    /// All STEP data-section entities, including geometry, topology and product metadata.
    pub entities: u32,
    /// Used authoritative native vertices.
    pub vertices: u32,
    /// Shared bounded native edges.
    pub edges: u32,
    /// Oriented edge occurrences across face loops.
    pub coedges: u32,
    /// Used native loops.
    pub loops: u32,
    /// Native supporting faces.
    pub faces: u32,
    /// Connected closed shells; one in the initial supported family.
    pub shells: u32,
    /// Exported manifold-solid records; one in the initial supported family.
    pub solids: u32,
}

/// Explicit geometric and serialization evidence; not independent CAD certification.
#[derive(Clone, Debug, PartialEq)]
pub struct StepReport {
    /// Declared application schema.
    pub schema: &'static str,
    /// Meaning of source/output coordinates and reported lengths.
    pub units: LengthUnit,
    /// Emitted representation uncertainty, in source length units.
    pub declared_uncertainty: f64,
    /// Conservative input endpoint/curve/support agreement bound.
    pub max_native_support_gap: f64,
    /// Allowance for normalized STEP frame directions and placed f64 coordinates.
    pub frame_roundoff_bound: f64,
    /// Sum of input support agreement and export frame/coordinate allowance.
    pub max_deviation: f64,
    /// Output work counts.
    pub counts: StepCounts,
    /// Source-to-output topological correspondence.
    pub correspondence: StepCorrespondence,
}

/// Complete exchange text and its evidence. Failure never returns partial text.
#[derive(Clone, Debug, PartialEq)]
pub struct StepDocument {
    /// ISO 10303-21 clear-text exchange document.
    pub text: String,
    /// Native agreement, uncertainty, counts and source correspondence.
    pub report: StepReport,
}

/// What prevented a bounded native STEP export.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StepError {
    /// Positive finite tolerance and nonzero output budgets are required.
    InvalidOptions,
    /// Creation timestamp is not a supported ISO 8601 date/time spelling.
    Timestamp,
    /// Occurrence placement violates the rigid-frame contract.
    Placement(FrameError),
    /// Native references, endpoints or supporting geometry are inconsistent.
    Boundary(BodyError),
    /// A solid export requires at least one face.
    EmptyBody,
    /// Every used edge must occur once in each direction.
    EdgeIncidence {
        /// Refused native edge.
        edge: EdgeId,
        /// Uses in increasing and decreasing curve direction.
        uses: [u32; 2],
    },
    /// Multiple shells require explicit solid/cavity ownership, outside this family.
    ShellCount {
        /// Connected components found among the faces.
        found: usize,
    },
    /// Supporting surface is outside this exact exchange family.
    UnsupportedSurface {
        /// Refused source face.
        face: FaceId,
        /// Surface family name.
        family: &'static str,
    },
    /// Circle trims must span at most one period and have consistent closure.
    CircleRange {
        /// Refused native edge.
        edge: EdgeId,
        /// Increasing source parameter bounds.
        range: [f64; 2],
    },
    /// A cylindrical face domain spans more than one angular period.
    CylinderDomain {
        /// Refused native face.
        face: FaceId,
    },
    /// Finite coordinates, directions or their export allowance are unrepresentable.
    Numeric,
    /// Requested uncertainty cannot cover native agreement and export arithmetic.
    Accuracy {
        /// Conservative required allowance.
        required: f64,
        /// Requested declaration.
        tolerance: f64,
    },
    /// Input work or complete output would exceed a caller budget.
    Budget {
        /// Storage/work category.
        resource: &'static str,
        /// Minimum required amount observed before refusal, saturating on overflow.
        required: usize,
        /// Allowed amount.
        maximum: usize,
    },
}
impl core::fmt::Display for StepError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidOptions => write!(
                f,
                "STEP options require positive finite tolerance and nonzero budgets"
            ),
            Self::Timestamp => write!(f, "STEP creation timestamp must be an ISO 8601 date/time"),
            Self::Placement(e) => write!(f, "STEP occurrence placement: {e}"),
            Self::Boundary(e) => write!(f, "STEP native boundary: {e}"),
            Self::EmptyBody => write!(f, "STEP solid needs a nonempty boundary"),
            Self::EdgeIncidence { edge, uses } => write!(
                f,
                "STEP edge {edge:?} needs one use in each direction, found {uses:?}"
            ),
            Self::ShellCount { found } => {
                write!(f, "STEP export needs one connected shell, found {found}")
            }
            Self::UnsupportedSurface { face, family } => {
                write!(f, "STEP face {face:?} has unsupported {family} support")
            }
            Self::CircleRange { edge, range } => write!(
                f,
                "STEP circle edge {edge:?} has unsupported range {range:?}"
            ),
            Self::CylinderDomain { face } => write!(
                f,
                "STEP cylindrical face {face:?} spans more than one angular period"
            ),
            Self::Numeric => write!(f, "STEP geometry or arithmetic is not representable"),
            Self::Accuracy {
                required,
                tolerance,
            } => write!(
                f,
                "STEP needs allowance {required}, exceeding requested uncertainty {tolerance}"
            ),
            Self::Budget {
                resource,
                required,
                maximum,
            } => write!(f, "STEP {resource} needs {required}, exceeding {maximum}"),
        }
    }
}
impl core::error::Error for StepError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Placement(e) => Some(e),
            Self::Boundary(e) => Some(e),
            _ => None,
        }
    }
}

/// Exports one connected, closed native shell with exact supported defining geometry.
///
/// Applies the complete rigid occurrence placement in f64. Output units state
/// the physical meaning of one source coordinate; no implicit conversion occurs.
/// The exporter preserves shared native topology and serializes planes/cylinders
/// and line/circle curves directly. It does not read a display mesh, approximate
/// unsupported supports, heal input, or establish global solid validity.
///
/// ```
/// use exedra_analytic::body::Body;
/// use exedra_math::Placement3;
/// use exedra_step::{export_body, LengthUnit, StepDocument, StepError, StepOptions};
///
/// // The caller supplies the retained body and its complete occurrence frame.
/// fn export_part<F, E>(body: &Body<F, E>, frame: Placement3)
///     -> Result<StepDocument, StepError>
/// {
///     export_body(body, frame, &StepOptions {
///         name: "Pocketed panel",
///         timestamp: "2026-10-27T12:00:00Z",
///         units: LengthUnit::Millimetre,
///         tolerance: 1e-7,
///         ..StepOptions::default()
///     })
/// }
/// ```
///
/// # Errors
/// Returns precise native-boundary, unsupported-family, uncertainty and bounded
/// work/output refusals. No source is changed and no partial document is returned.
pub fn export_body<F, E>(
    body: &Body<F, E>,
    placement: Placement3,
    options: &StepOptions<'_>,
) -> Result<StepDocument, StepError> {
    let prepared = preflight::prepare(body, placement, options)?;
    write::export(body, placement, options, prepared)
}

#[cfg(test)]
mod tests;
