// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Transactional subdivision of retained native boundaries.
//!
//! [`split_ruled_face_at_u`] inserts one shared ruling into a rectangular ruled
//! face and splits its generating rims in both neighboring planar faces. Children
//! retain the **original supporting surface** and partition its native U domain.
//! This is a topology edit, not clipping, a Boolean or a new surface fit.
//!
//! Input must already be an embedded, outward-oriented solid. Local boundary
//! validation checks topology/support agreement, not that global premise. Native
//! IDs are table-local: changed face/rim slots are reused explicitly in the report;
//! source selections must be resolved again in the result's new installation scope.

use crate::body::{Body, EdgeId, FaceId, LoopId, VertexId};
use crate::geometry::{GeometryError, ParameterDomain, ParameterRange};
use crate::validation::{
    BoundaryValidationError, BoundaryValidationPolicy, BoundaryValidationReport,
};

mod ruled;
#[cfg(test)]
mod tests;

/// Tolerance, separation and work limits for one checked immutable split.
///
/// Topology budgets cover loop/link/trim inspection and are checked before
/// cloning output tables. Source limits apply to the cumulative partial
/// ruled-support proof accounts in [`RuledSplitWork`], including repeated
/// preparation of the same support. Known preparation requirements are checked
/// before cloning; later proof operations are admitted before each step while
/// the result remains private.
///
/// General endpoint, elementary support and trim validation retain their own
/// scope under `validation`; zero source work does not mean zero general geometry
/// evaluation. These limits do not bound the complete body's retained geometry,
/// caller metadata or immutable result clone, whose cost scales with input size.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RuledSplitPolicy {
    /// Local boundary checks and topology work limits.
    pub validation: BoundaryValidationPolicy,
    /// Maximum cumulative source controls admitted to partial support proofs.
    pub max_source_controls: usize,
    /// Maximum cumulative metadata, preparation and enclosure proof work.
    pub max_source_work: u64,
    /// Strict minimum native U distance from either endpoint, dimensionless.
    pub parameter_clearance: f64,
    /// Strict distance from each new rim vertex to both old ends of that rim,
    /// in model units. Must exceed `validation.max_gap`.
    pub vertex_clearance: f64,
}
impl Default for RuledSplitPolicy {
    fn default() -> Self {
        Self {
            validation: BoundaryValidationPolicy::default(),
            max_source_controls: 65_536,
            max_source_work: 1_000_000,
            parameter_clearance: 1e-8,
            vertex_clearance: 1e-8,
        }
    }
}
/// One replacement face, ordered by increasing native U in the split report.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RuledFaceFragment {
    /// Result-local identity; the lower-U child reuses the replaced source slot.
    pub face: FaceId,
    /// Rectangle on the unchanged original support. V remains `[0,1]`.
    pub domain: ParameterDomain,
}
/// One generating rim replaced consistently in the wall and its planar neighbor.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RuledRimSplit {
    /// Replaced source edge. Its slot now holds the lower-U child.
    pub source: EdgeId,
    /// Original canonical curve-parameter range.
    pub source_range: ParameterRange,
    /// Child IDs in increasing **surface U**, which can oppose curve parameters.
    pub children: [EdgeId; 2],
    /// Canonical increasing curve-parameter range of each child.
    pub ranges: [ParameterRange; 2],
    /// Split parameter in the original generating curve's parameter units.
    pub parameter: f64,
    /// New shared vertex on this rim.
    pub vertex: VertexId,
    /// Neighboring plane face whose rim occurrence and trim were split.
    pub neighbor: FaceId,
    /// Updated neighbor loop; its other coedges/trim records retain their order.
    pub neighbor_loop: LoopId,
}
/// Admitted work for one completed ruled-face split.
///
/// Input coedges and source control records are separate counts. Source proof
/// work totals `source_metadata_work + source_preparation_work +
/// source_enclosure_work`; the three terms share the policy's source-work budget.
/// Controls count cumulative preparation admissions, including repeated support
/// preparation, rather than unique controls or peak scratch capacity.
///
/// Source accounts cover partial ruled-support proofs; general endpoint,
/// elementary support and trim validation are separate. Admission timing and
/// clone-cost scope follow [`RuledSplitPolicy`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct RuledSplitWork {
    /// Input loop edge occurrences admitted before cloning, including detached loops.
    pub input_coedges: usize,
    /// Cumulative original control records admitted to proof preparation.
    pub source_controls: usize,
    /// Eligibility metadata records visited separately from preparation.
    pub source_metadata_work: u64,
    /// Conservative preparation-work bound admitted from the curve kernel.
    pub source_preparation_work: u64,
    /// Admitted enclosure/control callbacks and fixed-size 3D endpoint-box
    /// and distance comparisons.
    pub source_enclosure_work: u64,
}
/// Explicit source-to-result replacements and local boundary evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct RuledSplitReport {
    /// Replaced source face. Its slot now holds the lower-U child.
    pub source: FaceId,
    /// Source rectangle on its original supporting surface.
    pub source_domain: ParameterDomain,
    /// Partition parameter in that support's original U coordinates.
    pub u: f64,
    /// Lower-U and upper-U children. Both retain the original supporting surface.
    pub faces: [RuledFaceFragment; 2],
    /// Bottom and top generating rims, respectively.
    pub rims: [RuledRimSplit; 2],
    /// New shared edge, canonically directed from V=0 to V=1.
    pub ruling: EdgeId,
    /// Topology and partial ruled-support proof accounting.
    pub work: RuledSplitWork,
    /// Output local manifold/support and supported trim evidence. Unsupported
    /// arrangements remain explicit; this is not a global solid certificate.
    pub validation: BoundaryValidationReport,
}
/// Complete immutable edit result. Refusal returns no partial body.
#[derive(Clone, Debug, PartialEq)]
pub struct RuledSplitResult<F = (), E = ()> {
    /// Resulting native solid with untouched table IDs and metadata preserved.
    pub body: Body<F, E>,
    /// Replaced IDs, exact source ranges and output validation evidence.
    pub report: RuledSplitReport,
}
/// Why this bounded native split could not be completed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RuledSplitError {
    /// Nonfinite/nonpositive clearances or vertex clearance within support slack.
    InvalidPolicy,
    /// Input local topology or geometric agreement failed.
    InputBoundary(BoundaryValidationError),
    /// Result local topology or geometric agreement failed; input is unchanged.
    OutputBoundary(BoundaryValidationError),
    /// Source-local face ID is absent.
    MissingFace {
        /// Requested identity.
        face: FaceId,
    },
    /// Face is not a four-distinct-edge ruled rectangle with full V and U in UNIT.
    UnsupportedFace {
        /// Source face.
        face: FaceId,
        /// Specific supported-subset refusal.
        reason: &'static str,
    },
    /// A rim must be a retained nonclosed line/NURBS source span with one planar neighbor.
    UnsupportedRim {
        /// Source rim.
        edge: EdgeId,
        /// Specific supported-subset refusal.
        reason: &'static str,
    },
    /// Neighboring explicit trim cannot be restricted without changing its representation.
    UnsupportedTrim {
        /// Planar neighbor.
        face: FaceId,
        /// Its rim occurrence.
        edge: EdgeId,
    },
    /// U is nonfinite or insufficiently separated from either current endpoint.
    Parameter {
        /// Requested U.
        u: f64,
        /// Current native interval.
        domain: ParameterRange,
        /// Required strict distance from endpoints.
        clearance: f64,
    },
    /// A new rim vertex is too close to an old endpoint, or rulings collapse.
    NearVertex {
        /// Source rim.
        edge: EdgeId,
        /// Observed minimum distance.
        distance: f64,
        /// Required strict model-unit separation.
        clearance: f64,
    },
    /// Parameter arithmetic collapsed an interval or produced a nonfinite result.
    Numeric {
        /// Source rim when localized.
        edge: Option<EdgeId>,
    },
    /// Native curve construction failed.
    Geometry(GeometryError),
    /// Original-source proof work/control admission exceeded its own limit.
    SourceBudget {
        /// Source work or cumulative prepared-control category.
        resource: &'static str,
        /// Required cumulative admission, saturated on overflow.
        required: u64,
        /// Permitted cumulative admission.
        maximum: u64,
    },
    /// Output work/storage would exceed its explicit limit.
    Budget {
        /// Work or table category.
        resource: &'static str,
        /// Required amount, saturated on overflow.
        required: usize,
        /// Permitted maximum.
        maximum: usize,
    },
}
impl core::fmt::Display for RuledSplitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SourceBudget {
                resource,
                required,
                maximum,
            } => write!(
                f,
                "ruled split {resource} needs {required}, maximum {maximum}"
            ),
            Self::InvalidPolicy => write!(
                f,
                "ruled split requires positive finite clearances outside support slack"
            ),
            Self::InputBoundary(e) => write!(f, "ruled split input boundary: {e}"),
            Self::OutputBoundary(e) => write!(f, "ruled split output boundary: {e}"),
            Self::MissingFace { face } => write!(f, "ruled split missing face {face:?}"),
            Self::UnsupportedFace { face, reason } => {
                write!(f, "ruled split face {face:?}: {reason}")
            }
            Self::UnsupportedRim { edge, reason } => {
                write!(f, "ruled split rim {edge:?}: {reason}")
            }
            Self::UnsupportedTrim { face, edge } => write!(
                f,
                "ruled split neighbor {face:?} trim on {edge:?} is unsupported"
            ),
            Self::Parameter {
                u,
                domain,
                clearance,
            } => write!(
                f,
                "ruled split U {u} must lie strictly inside {domain:?} by more than {clearance}"
            ),
            Self::NearVertex {
                edge,
                distance,
                clearance,
            } => write!(
                f,
                "ruled split rim {edge:?} vertex distance {distance} must exceed {clearance}"
            ),
            Self::Numeric { edge } => write!(
                f,
                "ruled split parameter arithmetic is unresolved on {edge:?}"
            ),
            Self::Geometry(e) => write!(f, "ruled split geometry: {e}"),
            Self::Budget {
                resource,
                required,
                maximum,
            } => write!(
                f,
                "ruled split {resource} needs {required}, exceeding {maximum}"
            ),
        }
    }
}
impl core::error::Error for RuledSplitError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::InputBoundary(e) | Self::OutputBoundary(e) => Some(e),
            Self::Geometry(e) => Some(e),
            _ => None,
        }
    }
}
/// Inserts a shared ruling at original native U and replaces two generating rims.
///
/// The input is never changed. Only four-edge ruled rectangles with full UNIT V,
/// nonclosed retained line/NURBS rims and planar rim neighbors are supported.
/// Rim curves must keep the original generating curve's defining representation;
/// there is no parameter recovery, projection, fitting, flattening or healing.
/// Existing planar line/NURBS trims are restricted per coedge occurrence; legacy
/// planar loops receive explicit trims. Repeated splits address either reported
/// child using the same original support coordinates and compose transactionally.
///
/// Untouched IDs are stable. The lower-U face/rim children **replace** source
/// slots; source selections do not become valid result selections by integer ID.
/// Metadata is cloned; `ruling_role` explicitly labels the new shared edge.
///
/// # Errors
/// Returns structured input, unsupported-subset, separation, numeric, work-budget
/// or result-boundary failures. No partial body is returned.
pub fn split_ruled_face_at_u<F: Clone, E: Clone>(
    body: &Body<F, E>,
    face: FaceId,
    u: f64,
    ruling_role: E,
    policy: &RuledSplitPolicy,
) -> Result<RuledSplitResult<F, E>, RuledSplitError> {
    ruled::split(body, face, u, ruling_role, policy)
}
