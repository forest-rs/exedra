// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checked extrusion of native planar curve loops, including rational splines.
//!
//! Unlike [`crate::extrusion::Extrusion`], this constructor accepts bounded native
//! curves rather than a line/arc profile. Curves and their parameter ranges stay
//! authoritative; subdivision is used only to check the outline, never to replace
//! a spline with polygonal geometry. Caps retain explicit planar trims and curved
//! walls are exact ruled patches. The ordinary body realization, compilation,
//! native archive and STEP paths consume the result.
//!
//! The conservative outline check proves coordinate monotonicity of rational
//! Bezier pieces and separates their outward-rounded control enclosures.
//! Neighboring hulls may meet only at their shared endpoint. A checked chord
//! profile then establishes winding and hole containment. Difficult valid outlines
//! can be refused as unresolved. Subdivision propagates interval roundoff and
//! shared endpoint identity. No endpoint is snapped or healed.
//!
//! The initial supported sources are unit-range lines with exactly representable
//! endpoint sums and full-domain clamped single-span NURBS of order at most 16.
//! Multi-span splines, partial ranges and conics are explicit typed refusals.
//! Source controls must lie exactly in XY; support tolerance is used only by the
//! final native-boundary checks.

use crate::body::{Body, BodyError, Face, FaceId};
use crate::extrusion::{EdgeRole, FaceRole};
use crate::geometry::{CurveSpan, GeometryError};
use alloc::vec::Vec;
use exedra_curves::profile::{LoopRef, ProfileError, SegTag};
use exedra_math::{FrameError, Placement3};

mod build;
mod chart;
pub use chart::PlanarExtrusionUvError;
mod outline;
#[cfg(test)]
mod tests;

/// A bounded native curve in local XY, traversed in its authored direction.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanarSegment {
    /// Exact source geometry, parameter interval and traversal direction.
    pub span: CurveSpan,
    /// Caller feature carried by the generated wall.
    pub tag: Option<SegTag>,
}
impl PlanarSegment {
    /// Creates an untagged segment from an already checked native span.
    #[must_use]
    pub fn new(span: CurveSpan) -> Self {
        Self { span, tag: None }
    }
    /// Creates a straight segment with these exact local XY endpoints.
    ///
    /// Uses a native line when its coefficients represent the requested endpoint
    /// exactly; otherwise an order-two NURBS retains the two endpoint controls.
    /// This representation choice does not move or heal either endpoint.
    ///
    /// # Errors
    /// Returns [`GeometryError::ZeroDirection`] for identical endpoints and
    /// [`GeometryError::NonFinite`] for nonfinite or unrepresentable geometry.
    pub fn line(from: [f64; 2], to: [f64; 2]) -> Result<Self, GeometryError> {
        if from.iter().chain(&to).any(|v| !v.is_finite()) {
            return Err(GeometryError::NonFinite);
        }
        if from == to {
            return Err(GeometryError::ZeroDirection);
        }
        let a = [from[0], from[1], 0.0];
        let b = [to[0], to[1], 0.0];
        let direction = exedra_math::sub(b, a);
        let exact = (0..3).all(|k| {
            let sum = a[k] + direction[k];
            sum == b[k] && outline::sum_error(a[k], direction[k], sum) == 0.0
        });
        let curve = if exact {
            crate::geometry::Curve::line(a, direction)?
        } else {
            crate::geometry::Curve::nurbs(
                crate::geometry::NurbsCurve::line(a, b).ok_or(GeometryError::NonFinite)?,
            )?
        };
        Ok(Self::new(CurveSpan::new(
            curve,
            crate::geometry::ParameterRange::UNIT,
            false,
        )?))
    }
    /// Labels the generated wall without changing its geometry or local IDs.
    #[must_use]
    pub fn tagged(mut self, tag: SegTag) -> Self {
        self.tag = Some(tag);
        self
    }
}
impl From<CurveSpan> for PlanarSegment {
    fn from(span: CurveSpan) -> Self {
        Self::new(span)
    }
}

/// Model-unit agreement tolerance and independent construction work limits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlanarExtrusionPolicy {
    /// Maximum native curve/support agreement gap. Input endpoints must still
    /// agree exactly: this tolerance never joins an open outline.
    pub max_gap: f64,
    /// Minimum separation used by the checked chord profile. Control-hull
    /// separation establishes disjointness, not this quantitative curve clearance.
    pub chord_separation: f64,
    /// Maximum source segments and temporary rational pieces, independently.
    pub max_pieces: usize,
    /// Maximum control-hull pair checks over all subdivision attempts.
    pub max_pair_tests: usize,
    /// Maximum simultaneous bisections of every Bezier piece, in `0..=16`.
    pub max_subdivisions: u32,
}
impl Default for PlanarExtrusionPolicy {
    fn default() -> Self {
        Self {
            max_gap: 1e-9,
            chord_separation: 1e-9,
            max_pieces: 4096,
            max_pair_tests: 1_000_000,
            max_subdivisions: 8,
        }
    }
}

/// Outline checks actually performed before building native tables.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanarExtrusionReport {
    /// Authored curve segments, including all holes.
    pub source_segments: usize,
    /// Rational pieces in the accepted outline check; not output mesh segments.
    pub checked_pieces: usize,
    /// Control-hull pairs examined, including earlier subdivision attempts.
    pub pair_tests: usize,
    /// Simultaneous bisections used by the accepted outline check.
    pub subdivisions: u32,
}

/// A precise construction refusal. No partial body is returned.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PlanarExtrusionError {
    /// Nonfinite/nonpositive tolerances, zero budgets or excessive subdivision.
    InvalidPolicy,
    /// Length must be positive and finite.
    InvalidLength {
        /// Requested length.
        length: f64,
    },
    /// The occurrence frame is not a checked rigid placement.
    Frame(FrameError),
    /// An outline needs at least two bounded curves.
    EmptyLoop {
        /// Outer or indexed hole.
        loop_ref: LoopRef,
    },
    /// A curve is not in local XY, cannot be converted/bounded, or exceeds
    /// the exact-predicate coordinate envelope.
    Geometry {
        /// Offending outline.
        loop_ref: LoopRef,
        /// Source segment.
        segment: usize,
        /// Native geometry failure.
        source: GeometryError,
    },
    /// Consecutive authoritative endpoints differ. They are never moved.
    OpenLoop {
        /// Offending outline.
        loop_ref: LoopRef,
        /// Segment whose start differs from its predecessor's end.
        segment: usize,
        /// Model-unit endpoint distance.
        gap: f64,
    },
    /// A source is outside the checked line/single-span rational Bezier subset,
    /// including partial ranges or inexact line endpoint arithmetic.
    UnsupportedSource {
        /// Source outline.
        loop_ref: LoopRef,
        /// Source segment.
        segment: usize,
    },
    /// A source curve exceeds the bounded order supported by outline checking.
    UnsupportedOrder {
        /// Source outline.
        loop_ref: LoopRef,
        /// Source segment.
        segment: usize,
        /// Requested curve order.
        order: u32,
        /// Maximum accepted order.
        maximum: u32,
    },
    /// Source controls exceed the permitted distance from local XY.
    NonPlanar {
        /// Source outline.
        loop_ref: LoopRef,
        /// Source segment.
        segment: usize,
        /// Largest control distance from XY.
        distance: f64,
        /// Permitted support gap.
        maximum: f64,
    },
    /// A regular ruled wall could not be established.
    Wall {
        /// Source outline.
        loop_ref: LoopRef,
        /// Source segment.
        segment: usize,
        /// Geometry failure.
        source: GeometryError,
    },
    /// Simplicity or separation could not be proved within the subdivision limit.
    Unresolved {
        /// First source outline.
        loop_ref: LoopRef,
        /// First source segment.
        segment: usize,
        /// Other outline and source segment, for a pair refusal.
        other: Option<(LoopRef, usize)>,
    },
    /// A proved-simple outline has incorrect winding or hole containment, or its
    /// chord profile cannot establish the requested chord separation.
    Profile(ProfileError),
    /// A work or storage budget stopped construction before output allocation.
    Budget {
        /// Exhausted resource.
        resource: &'static str,
        /// Permitted maximum.
        maximum: usize,
    },
    /// Final retained support/topology/trim agreement failed.
    Body(BodyError),
}
impl core::fmt::Display for PlanarExtrusionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPolicy => write!(f, "planar extrusion policy is invalid"),
            Self::InvalidLength { length } => write!(
                f,
                "planar extrusion length must be positive and finite, got {length}"
            ),
            Self::Frame(e) => write!(f, "planar extrusion frame: {e}"),
            Self::EmptyLoop { loop_ref } => {
                write!(f, "planar extrusion {loop_ref:?} needs at least two curves")
            }
            Self::Geometry {
                loop_ref,
                segment,
                source,
            } => write!(
                f,
                "planar extrusion {loop_ref:?} segment {segment}: {source}"
            ),
            Self::OpenLoop {
                loop_ref,
                segment,
                gap,
            } => write!(
                f,
                "planar extrusion {loop_ref:?} segment {segment} has an open endpoint gap of {gap}"
            ),
            Self::UnsupportedSource { loop_ref, segment } => write!(
                f,
                "planar extrusion {loop_ref:?} segment {segment} is outside the checked line/Bezier subset"
            ),
            Self::UnsupportedOrder {
                loop_ref,
                segment,
                order,
                maximum,
            } => write!(
                f,
                "planar extrusion {loop_ref:?} segment {segment} order {order} exceeds {maximum}"
            ),
            Self::NonPlanar {
                loop_ref,
                segment,
                distance,
                maximum,
            } => write!(
                f,
                "planar extrusion {loop_ref:?} segment {segment} distance {distance} from XY exceeds {maximum}"
            ),
            Self::Wall {
                loop_ref,
                segment,
                source,
            } => write!(
                f,
                "planar extrusion {loop_ref:?} segment {segment} wall: {source}"
            ),
            Self::Unresolved {
                loop_ref,
                segment,
                other,
            } => write!(
                f,
                "planar extrusion could not establish {loop_ref:?} segment {segment} separation from {other:?}"
            ),
            Self::Profile(e) => write!(f, "planar extrusion outline: {e}"),
            Self::Budget { resource, maximum } => {
                write!(f, "planar extrusion {resource} exceeds {maximum}")
            }
            Self::Body(e) => write!(f, "planar extrusion boundary: {e}"),
        }
    }
}
impl core::error::Error for PlanarExtrusionError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Frame(e) => Some(e),
            Self::Geometry { source, .. } | Self::Wall { source, .. } => Some(source),
            Self::Profile(e) => Some(e),
            Self::Body(e) => Some(e),
            _ => None,
        }
    }
}

/// A placed constant-section solid with exact source curves and ruled walls.
///
/// Local outer loops wind counterclockwise and holes clockwise, viewed from +Z.
/// Source controls lie exactly in XY and consecutive endpoints agree exactly.
/// Unit-range lines and full-domain clamped single-span NURBS (order at most 16)
/// are supported; other curve layouts are refused. Placement
/// remains separate from body-local geometry. General pockets and exact integral
/// queries are not supplied by this constructor.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanarExtrusion {
    body: Body<FaceRole, EdgeRole>,
    frame: Placement3,
    length: f64,
    report: PlanarExtrusionReport,
}
impl PlanarExtrusion {
    /// Extrudes a checked outline from local Z=0 to `length`.
    ///
    /// Both caps contain every hole. Curve spans retain their original ranges,
    /// reversal and tags. Nonplanar, open, wrong-winding, unresolved or over-budget
    /// inputs return structured errors before a body can escape.
    ///
    /// # Errors
    /// Returns [`PlanarExtrusionError`] with loop and source-segment context.
    pub fn new(
        outer: Vec<PlanarSegment>,
        holes: Vec<Vec<PlanarSegment>>,
        frame: Placement3,
        length: f64,
        policy: &PlanarExtrusionPolicy,
    ) -> Result<Self, PlanarExtrusionError> {
        if !(length.is_finite() && length > 0.0) {
            return Err(PlanarExtrusionError::InvalidLength { length });
        }
        crate::extrusion::Workplane::new(frame).map_err(PlanarExtrusionError::Frame)?;
        let loops: Vec<_> = core::iter::once(outer).chain(holes).collect();
        let report = outline::check(&loops, policy)?;
        let body = build::body(&loops, length, policy)?;
        Ok(Self {
            body,
            frame,
            length,
            report,
        })
    }
    /// Shared native topology with existing extrusion face and edge roles.
    #[must_use]
    pub fn body(&self) -> &Body<FaceRole, EdgeRole> {
        &self.body
    }
    /// Native cap at one end. IDs belong to this solid and remain unchanged
    /// when it is realized at another display tolerance.
    #[must_use]
    pub const fn cap(&self, end: crate::End) -> FaceId {
        FaceId(match end {
            crate::End::Start => 0,
            crate::End::End => 1,
        })
    }
    /// Every wall carrying `tag`, paired with its native face ID in table order.
    /// Repeated labels return all matching walls. Caps have no feature tag.
    /// Native IDs and roles remain independent of display tessellation.
    pub fn faces_tagged(
        &self,
        tag: SegTag,
    ) -> impl Iterator<Item = (FaceId, &Face<FaceRole>)> + '_ {
        self.body
            .faces()
            .iter()
            .enumerate()
            .filter_map(move |(i, face)| {
                matches!(face.role, FaceRole::Wall { tag: Some(found), .. } if found == tag)
                    .then_some((FaceId(crate::usize_to_u32(i)), face))
            })
    }
    /// Moves the native body without cloning source curves or boundary tables.
    #[must_use]
    pub fn into_body(self) -> Body<FaceRole, EdgeRole> {
        self.body
    }
    /// Rigid occurrence placement, applied separately by realization and exchange.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Positive length along local +Z.
    #[must_use]
    pub const fn length(&self) -> f64 {
        self.length
    }
    /// Bounded outline-check work, independent of display tessellation.
    #[must_use]
    pub const fn report(&self) -> &PlanarExtrusionReport {
        &self.report
    }
}
