// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! STEP exchange of retained analytic boundaries, independent of display meshes.
//!
//! The exporter owns serialization, explicit exchange units and bounded output.
//! The analytic head owns geometry and incidence; callers own scene hierarchy,
//! feature metadata and occurrence placement. Single-shell export supports
//! planes, cylinders, cones, spheres and regular ring tori,
//! bounded by lines, circles and ellipses.
//!
//! Input incidence and conservative support agreement are checked. Those checks
//! do not certify general solid validity or trim nesting. Multiple-shell export
//! additionally requires an independent rectilinear material certificate: one
//! connected solid, disjoint strictly owned cavities and exact face transitions.
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
    /// Maximum conservative native support gap plus export frame/coordinate
    /// and support-parameter conversion allowances.
    /// Also emitted as the representation's declared length uncertainty.
    pub tolerance: f64,
    /// Largest entity count and maximum input-table work budget.
    pub max_entities: u32,
    /// Largest complete UTF-8/ASCII STEP text buffer, including header and trailer.
    pub max_bytes: usize,
    /// Independent material/shell certificate limits, used only for multiple shells.
    pub shell_validation: exedra_analytic::rectilinear::MaterialValidationPolicy,
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
            shell_validation: exedra_analytic::rectilinear::MaterialValidationPolicy::default(),
        }
    }
}

/// A positive entity identifier local to one complete STEP document.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepEntityId(pub u32);

/// Exact STEP representation of one used native boundary edge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StepEdgeEntity {
    /// A regular shared `EDGE_CURVE` entity.
    Curve(StepEntityId),
    /// Exact shared curve fragments emitted in canonical native-edge order.
    /// Fragment IDs are consecutive within this document; use
    /// [`Self::curve_entities`] to enumerate them.
    CurveChain {
        /// First emitted `EDGE_CURVE` fragment.
        first: StepEntityId,
        /// Number of fragments, between two and 128 for exporter output.
        count: u32,
    },
    /// A certified collapsed boundary represented by its authoritative
    /// `VERTEX_POINT`; no fictitious three-dimensional curve is emitted.
    CollapsedVertex(StepEntityId),
}
impl StepEdgeEntity {
    /// All regular `EDGE_CURVE` entities in native start-to-end order.
    /// Collapsed boundaries return an empty iterator. Coedge reversal changes
    /// traversal of this sequence, never the shared fragment identities.
    pub fn curve_entities(self) -> impl DoubleEndedIterator<Item = StepEntityId> {
        let (first, count) = match self {
            Self::Curve(id) => (id, 1),
            Self::CurveChain { first, count } => (first, count),
            Self::CollapsedVertex(_) => (StepEntityId(0), 0),
        };
        (0..count).filter_map(move |offset| first.0.checked_add(offset).map(StepEntityId))
    }
}

/// Native table indices mapped to exported topological entity IDs.
/// Unused native entries are `None`; identities are local to this source/document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepCorrespondence {
    /// Native vertices to `VERTEX_POINT` entities.
    pub vertices: Vec<Option<StepEntityId>>,
    /// Native edges to regular curves/chains or explicitly collapsed vertices.
    pub edges: Vec<Option<StepEdgeEntity>>,
    /// Native loops to `EDGE_LOOP` or pole-only `VERTEX_LOOP` entities.
    pub loops: Vec<Option<StepEntityId>>,
    /// Native faces to `ADVANCED_FACE` entities.
    pub faces: Vec<Option<StepEntityId>>,
    /// Faces whose base closed-shell sense opposes the native material sense.
    /// The enclosing void wrapper reverses them back in the represented solid.
    pub reversed_faces: Vec<FaceId>,
}
impl StepCorrespondence {
    /// Exported topological entity for a native vertex, if used.
    #[must_use]
    pub fn vertex(&self, id: VertexId) -> Option<StepEntityId> {
        self.vertices.get(id.index()).copied().flatten()
    }
    /// Exported regular curve/chain or collapsed vertex for a native edge, if used.
    #[must_use]
    pub fn edge(&self, id: EdgeId) -> Option<StepEdgeEntity> {
        self.edges.get(id.index()).copied().flatten()
    }
    /// Exported loop for a native loop, if used.
    #[must_use]
    pub fn boundary_loop(&self, id: LoopId) -> Option<StepEntityId> {
        self.loops.get(id.index()).copied().flatten()
    }
    /// Exported supporting face for a native face, if used.
    /// For voids, [`Self::face_reversed`] describes its base-shell orientation.
    #[must_use]
    pub fn face(&self, id: FaceId) -> Option<StepEntityId> {
        self.faces.get(id.index()).copied().flatten()
    }
    /// Whether a used face's base shell is reversed relative to native material.
    /// The enclosing `ORIENTED_CLOSED_SHELL` restores material orientation.
    #[must_use]
    pub fn face_reversed(&self, id: FaceId) -> Option<bool> {
        self.face(id)
            .map(|_| self.reversed_faces.binary_search(&id).is_ok())
    }
}

/// Work actually represented in the complete output document.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StepCounts {
    /// All STEP data-section entities, including geometry, topology and product metadata.
    pub entities: u32,
    /// Emitted topological vertices, including exact elliptic-arc subdivision points.
    pub vertices: u32,
    /// Emitted regular `EDGE_CURVE` entities; collapsed boundaries map to vertices.
    pub edges: u32,
    /// Emitted `ORIENTED_EDGE` occurrences; collapsed coedges are omitted.
    pub coedges: u32,
    /// Used native loops.
    pub loops: u32,
    /// Native supporting faces.
    pub faces: u32,
    /// Connected closed shells, including certified cavity boundaries.
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
    /// Length allowance for converting native support parameters to STEP parameters.
    /// Zero for surfaces whose defining parameters are serialized unchanged.
    pub parameter_roundoff_bound: f64,
    /// Maximum of serialized support/frame/parameter agreement and the independent
    /// multi-shell material certificate's support/arithmetic allowance.
    pub max_deviation: f64,
    /// Output work counts.
    pub counts: StepCounts,
    /// Source-to-output topological correspondence.
    pub correspondence: StepCorrespondence,
    /// Independent exact rectilinear material evidence for multiple shells.
    /// Single-shell export retains its existing local-support validation envelope.
    pub material_validation: Option<exedra_analytic::rectilinear::MaterialValidationReport>,
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
    /// Every used regular edge must occur once in each direction.
    EdgeIncidence {
        /// Refused native edge.
        edge: EdgeId,
        /// Uses in increasing and decreasing curve direction.
        uses: [u32; 2],
    },
    /// Multiple shells did not establish one bounded rectilinear solid with voids.
    ShellValidation(exedra_analytic::rectilinear::MaterialValidationError),
    /// Supporting surface is outside this exact exchange family.
    UnsupportedSurface {
        /// Refused source face.
        face: FaceId,
        /// Surface family name.
        family: &'static str,
    },
    /// Periodic curve trims span at most one period and have consistent closure.
    PeriodicCurveRange {
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
    /// A cone falls outside the current exact-interchange angle policy.
    /// Small representable angles are refused to avoid an observed reader clamp;
    /// extreme slopes are refused when angle conversion is poorly conditioned.
    ConeAngle {
        /// Refused native face.
        face: FaceId,
        /// Native signed radial change per unit axial height.
        slope: f64,
        /// Native signed semi-angle in radians.
        semi_angle: f64,
        /// Allowed absolute angle interval, lower inclusive and upper exclusive.
        /// The slope must additionally satisfy the conversion conditioning check.
        angle_range: [f64; 2],
    },
    /// A periodic supporting surface spans more than one period.
    PeriodicSurfaceDomain {
        /// Refused native face.
        face: FaceId,
        /// Periodic native parameter, `u` or `v`.
        parameter: &'static str,
        /// Increasing native parameter bounds.
        range: [f64; 2],
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
            Self::ShellValidation(error) => write!(f, "STEP shell ownership: {error}"),
            Self::UnsupportedSurface { face, family } => {
                write!(f, "STEP face {face:?} has unsupported {family} support")
            }
            Self::PeriodicCurveRange { edge, range } => write!(
                f,
                "STEP periodic edge {edge:?} has unsupported range {range:?}"
            ),
            Self::CylinderDomain { face } => write!(
                f,
                "STEP cylindrical face {face:?} spans more than one angular period"
            ),
            Self::ConeAngle {
                face,
                slope,
                semi_angle,
                angle_range,
            } => write!(
                f,
                "STEP face {face:?} cone slope {slope}, semi-angle {semi_angle} is outside current exact-interchange policy: absolute angle in [{}, {}) radians and safely conditioned slope required",
                angle_range[0], angle_range[1]
            ),
            Self::PeriodicSurfaceDomain {
                face,
                parameter,
                range,
            } => write!(
                f,
                "STEP face {face:?} periodic parameter {parameter} spans more than one period: {range:?}"
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
/// Checks structural STEP support without allocating output or realizing a mesh.
///
/// This checks supported native geometry and boundary kinds. A successful result
/// does not validate incidence, trims, placement, requested tolerance, timestamp
/// or output budgets; [`export_body`] checks those independently. The same
/// geometry policy is used by the exporter's full preflight. Zero-slope cones
/// have exact cylindrical support; extreme slopes can be refused if their
/// semi-angle is unrepresentable or poorly conditioned.
/// # Errors
/// Returns [`StepError::UnsupportedSurface`] with the native face and family,
/// or [`StepError::ConeAngle`] with a slope outside the current exact-interchange
/// policy, including the observed reader's small-angle clamp range.
pub fn check_geometry_support<F, E>(body: &Body<F, E>) -> Result<(), StepError> {
    for (index, face) in body.faces().iter().enumerate() {
        preflight::surface_support(
            &face.surface,
            FaceId(u32::try_from(index).expect("native face IDs fit u32")),
        )?;
    }
    Ok(())
}

/// Exports one native solid with exact supported defining geometry.
///
/// Applies the complete rigid occurrence placement in f64. Output units state
/// the physical meaning of one source coordinate; no implicit conversion occurs.
/// The exporter preserves shared native topology and serializes supported
/// elementary surfaces and line/circle/ellipse curves directly. It does not read a display mesh, approximate
/// unsupported supports or heal input. Single shells retain local support checks;
/// multiple shells require a separate exact rectilinear material certificate
/// establishing a connected solid and disjoint cavity ownership.
/// Elliptic edges can emit exact shared arc chains to mitigate observed reader
/// reconstruction errors. Reported deviation covers the serialized defining
/// geometry, not a reader's subsequently reconstructed surface parameter curves.
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
