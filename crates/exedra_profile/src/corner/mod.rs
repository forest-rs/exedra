// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Intentional corner selection and analytic line-corner treatment.

use crate::{Curve, LoopError, Profile, ProfileError, SegTag, SegmentRef};
use alloc::vec::Vec;

mod arrangement;
#[cfg(test)]
mod tests;

/// How the filled material turns at a boundary vertex.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CornerKind {
    /// A positive turn, convex toward the filled material.
    Convex,
    /// A negative turn, concave toward the filled material.
    Concave,
    /// Tangent directions agree within a sine of `1e-12`.
    Smooth,
}

/// A corner described by the segment ending at it, within this profile only.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Corner {
    /// The segment ending at the corner. Re-select after changing the profile.
    pub segment: SegmentRef,
    /// Corner position in model units.
    pub point: [f64; 2],
    /// Convexity relative to the filled region, including on holes.
    pub kind: CornerKind,
    /// Signed turn from incoming to outgoing tangent, in radians.
    pub turn: f64,
}

/// Geometry to insert between two trimmed straight segments.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CornerShape {
    /// A circular tangent arc.
    Fillet {
        /// Positive finite circle radius in model units.
        radius: f64,
    },
    /// A straight diagonal between equal-distance trims.
    Chamfer {
        /// Positive finite trim distance along each adjacent line.
        distance: f64,
    },
}

/// A corner operation with an optional label for its generated transitions.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CornerTreatment {
    /// The geometry to construct.
    pub shape: CornerShape,
    /// Label for generated arcs or chamfers. Without one, a shared label on
    /// both adjacent segments is retained; distinct labels are not guessed.
    pub tag: Option<SegTag>,
}

impl CornerTreatment {
    /// A tangent circular fillet of `radius`.
    #[must_use]
    pub const fn fillet(radius: f64) -> Self {
        Self {
            shape: CornerShape::Fillet { radius },
            tag: None,
        }
    }
    /// An equal-distance chamfer.
    #[must_use]
    pub const fn chamfer(distance: f64) -> Self {
        Self {
            shape: CornerShape::Chamfer { distance },
            tag: None,
        }
    }
    /// Labels every generated transition without replacing the retained line tags.
    #[must_use]
    pub const fn tagged(mut self, tag: SegTag) -> Self {
        self.tag = Some(tag);
        self
    }
}

/// Why selected corners cannot receive a treatment.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CornerError {
    /// The input fails the requested validation policy.
    Input(ProfileError),
    /// The radius or distance is not positive and finite.
    InvalidSize {
        /// The requested geometry, including its supplied dimension.
        shape: CornerShape,
    },
    /// The selected loop or segment does not exist.
    UnknownCorner {
        /// Requested reference.
        corner: SegmentRef,
    },
    /// The same corner was selected more than once.
    RepeatedCorner {
        /// Duplicate reference.
        corner: SegmentRef,
    },
    /// The selected vertex is tangent-continuous rather than a corner.
    SmoothCorner {
        /// Requested reference.
        corner: SegmentRef,
    },
    /// This operation treats line-line corners; selected arc junctions are refused.
    UnsupportedCorner {
        /// Requested reference.
        corner: SegmentRef,
        /// Incoming segment geometry.
        incoming: Curve,
        /// Outgoing segment geometry.
        outgoing: Curve,
    },
    /// Neighbouring trims leave less than the requested separation on a line.
    OverlappingTrims {
        /// The line being consumed from one or both ends.
        segment: SegmentRef,
        /// Original line length.
        length: f64,
        /// Sum of requested trim distances.
        requested: f64,
        /// Required untrimmed length.
        min_separation: f64,
    },
    /// New segment geometry is not representable in f64.
    Loop(LoopError),
    /// The treated profile fails winding, clearance or contact validation.
    Output(ProfileError),
}

impl core::fmt::Display for CornerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Input(e) => write!(f, "corner input: {e}"),
            Self::InvalidSize { shape } => write!(
                f,
                "corner dimension must be finite and positive, got {shape:?}"
            ),
            Self::UnknownCorner { corner } => write!(f, "corner {corner:?} does not exist"),
            Self::RepeatedCorner { corner } => write!(f, "corner {corner:?} was selected twice"),
            Self::SmoothCorner { corner } => write!(f, "corner {corner:?} is tangent-continuous"),
            Self::UnsupportedCorner {
                corner,
                incoming,
                outgoing,
            } => write!(
                f,
                "corner {corner:?} requires adjacent lines, got {incoming:?} and {outgoing:?}"
            ),
            Self::OverlappingTrims {
                segment,
                length,
                requested,
                min_separation,
            } => write!(
                f,
                "corner trims consume {requested} of line {segment:?} length {length}, requiring at least {min_separation} remaining"
            ),
            Self::Loop(e) => write!(f, "corner segment: {e}"),
            Self::Output(e) => write!(f, "corner output: {e}"),
        }
    }
}
impl core::error::Error for CornerError {}

/// The authored source of a treated output segment.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CornerSource {
    /// A retained or trimmed segment.
    Segment {
        /// Original segment.
        segment: SegmentRef,
        /// Retained start and end fractions, in original direction.
        parameters: [f64; 2],
    },
    /// An arc or chamfer generated at a selected vertex.
    Transition {
        /// Original segment ending at that vertex.
        corner: SegmentRef,
    },
}

/// Correspondence for one output segment, in outer-then-hole order.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CornerSegment {
    /// Output segment.
    pub segment: SegmentRef,
    /// Retained geometry or generating corner.
    pub source: CornerSource,
}

/// A validated treated profile with complete segment correspondence.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerResult {
    /// Treated profile; unselected arcs remain analytic and unchanged.
    pub profile: Profile,
    /// Retained/trimmed and generated segment sources.
    pub segments: Vec<CornerSegment>,
    /// Number of treated vertices.
    pub changed_corners: usize,
}
