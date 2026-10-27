// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retained ruled lofts through corresponding placed line/arc profiles.
//!
//! Segment order is explicit. Prepare different source structures in the
//! authoring head before constructing a loft. Walls retain exact source curves
//! and regular ruled surfaces; authored section boundaries are shared.

use crate::{
    End,
    body::{Body, BodyError, EdgeId, FaceId},
    geometry::GeometryError,
    realize::{RealizationError, RealizationPolicy, RealizedBody},
};
use alloc::vec::Vec;
use exedra_math::Placement3;
use exedra_profile::{IntoProfile, LoopRef, Profile, ProfilePolicy, SegTag, SegmentRef};

mod build;
mod chart;
pub use chart::LoftUvError;

/// Native source of one loft face.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FaceRole {
    /// Exact ruled wall between corresponding source segments.
    Wall {
        /// Authored band between sections `band` and `band+1`.
        band: usize,
        /// Corresponding source segment in both sections.
        segment: SegmentRef,
        /// Feature labels at the beginning and end of the band.
        tags: [Option<SegTag>; 2],
    },
    /// Planar first or last section.
    Cap {
        /// Construction end.
        end: End,
    },
}
/// Native source of one shared loft edge.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum EdgeRole {
    /// Original source curve at an authored section.
    Section {
        /// Authored section.
        section: usize,
        /// Source segment.
        segment: SegmentRef,
    },
    /// Straight ruling between corresponding profile vertices.
    Rail {
        /// Authored section band.
        band: usize,
        /// Segment starting at this source vertex.
        vertex: SegmentRef,
    },
}
/// Input validation and finite native construction limits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LoftPolicy {
    /// Validation of each exact source profile.
    pub profile: ProfilePolicy,
    /// Largest number of authored sections; at least two.
    pub max_sections: u32,
    /// Largest combined count of native vertices, edges, loops and faces.
    pub max_native_elements: u32,
}
impl LoftPolicy {
    /// Uses 4096 sections and one million native table entries as work limits.
    #[must_use]
    pub const fn new(profile: ProfilePolicy) -> Self {
        Self {
            profile,
            max_sections: 4096,
            max_native_elements: 1_000_000,
        }
    }
}
/// An exact ruled boundary and its original placed sections.
///
/// Section changes can express taper, twist and datum motion. Each point must
/// advance through both neighboring section planes; patches must pass a
/// conservative local regularity check. Distant intersections and nesting
/// through intermediate sections are not certified.
#[derive(Clone, Debug, PartialEq)]
pub struct Loft {
    sections: Vec<(Placement3, Profile)>,
    frame: Placement3,
    body: Body<FaceRole, EdgeRole>,
    ring_offsets: Vec<usize>,
    section_vertices: usize,
}
/// A source-indexed profile conversion or native loft refusal.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LoftError<E> {
    /// Construction policy is invalid.
    InvalidPolicy,
    /// At least two corresponding sections are required.
    Sections,
    /// Original profile conversion or validation failure.
    Profile {
        /// Authored section.
        section: usize,
        /// Original diagnosis.
        error: E,
    },
    /// Section and occurrence placements must be finite rigid frames.
    Frame {
        /// Section index, or `None` for the occurrence.
        section: Option<usize>,
        /// Frame diagnosis.
        error: GeometryError,
    },
    /// Hole count/order and segment counts must correspond explicitly.
    Correspondence {
        /// First incompatible section.
        section: usize,
        /// Incompatible loop, or `None` for different hole counts.
        loop_ref: Option<LoopRef>,
    },
    /// Construction would exceed a finite work limit.
    Budget {
        /// Native storage or input table.
        storage: &'static str,
        /// Required entries.
        required: usize,
        /// Caller limit.
        maximum: u32,
    },
    /// A source vertex does not advance through both neighboring section planes.
    Direction {
        /// Authored section band.
        band: usize,
        /// Source vertex, identified by its outgoing segment.
        vertex: SegmentRef,
    },
    /// Exact wall geometry could not be constructed or certified regular.
    Wall {
        /// Authored section band.
        band: usize,
        /// Source segment in that band.
        segment: SegmentRef,
        /// Supporting geometry diagnosis.
        error: GeometryError,
    },
    /// Native support agreement or incidence failed.
    Boundary(BodyError),
}
impl<E: core::fmt::Display> core::fmt::Display for LoftError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPolicy => write!(f, "native loft policy is invalid"),
            Self::Sections => write!(f, "native loft needs at least two sections"),
            Self::Profile { section, error } => write!(f, "loft profile {section}: {error}"),
            Self::Frame { section, error } => write!(f, "loft frame {section:?}: {error}"),
            Self::Correspondence { section, loop_ref } => write!(
                f,
                "loft section {section} does not correspond at {loop_ref:?}"
            ),
            Self::Budget {
                storage,
                required,
                maximum,
            } => write!(
                f,
                "loft {storage} needs {required} entries, exceeding {maximum}"
            ),
            Self::Direction { band, vertex } => write!(
                f,
                "loft band {band} vertex {vertex:?} does not advance through its section planes"
            ),
            Self::Wall {
                band,
                segment,
                error,
            } => write!(f, "loft band {band} segment {segment:?}: {error}"),
            Self::Boundary(error) => write!(f, "loft boundary: {error}"),
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for LoftError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Profile { error, .. } => Some(error),
            Self::Frame { error, .. } | Self::Wall { error, .. } => Some(error),
            Self::Boundary(error) => Some(error),
            _ => None,
        }
    }
}
impl Loft {
    /// Converts placed source profiles and retains their exact ruled boundary.
    ///
    /// All sections need the same hole count and segment counts per loop.
    /// Segment `k` corresponds in every section; lines and arcs may be paired.
    /// Profiles wind CCW outside and CW around holes. Sections advance along
    /// their local +Z directions. No hidden matching or meshing occurs.
    ///
    /// ```
    /// use exedra_analytic::loft::{Loft, LoftPolicy};
    /// use exedra_math::Placement3;
    /// use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    /// let validation = ProfilePolicy::new(1e-9);
    /// let outline = Loop::new(vec![
    ///     Segment::line([1.0, -1.0]), Segment::line([1.0, 1.0]),
    ///     Segment::line([-1.0, 1.0]), Segment::line([-1.0, -1.0]),
    /// ])?;
    /// let square = Profile::new(outline, vec![], &validation)?;
    /// let loft = Loft::from_profiles([
    ///     (Placement3::IDENTITY, &square),
    ///     (Placement3::translate(0.5, 0.0, 4.0), &square),
    /// ], Placement3::IDENTITY, &LoftPolicy::new(validation))?;
    /// assert!(loft.body().is_closed());
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// Preserves conversion errors with their section index, correspondence and
    /// work-limit refusals, and local wall regularity/section-direction witnesses.
    pub fn from_profiles<P: IntoProfile>(
        sections: impl IntoIterator<Item = (Placement3, P)>,
        frame: Placement3,
        policy: &LoftPolicy,
    ) -> Result<Self, LoftError<P::Error>> {
        build::construct(sections, frame, policy)
    }
    /// Original placed source profiles, before occurrence placement.
    #[must_use]
    pub fn sections(&self) -> &[(Placement3, Profile)] {
        &self.sections
    }
    /// Rigid occurrence placement.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Construction-independent exact supporting geometry and shared boundaries.
    #[must_use]
    pub fn body(&self) -> &Body<FaceRole, EdgeRole> {
        &self.body
    }
    /// Native provenance of a shared boundary edge.
    #[must_use]
    pub fn edge_role(&self, edge: EdgeId) -> Option<EdgeRole> {
        self.body.edge(edge).map(|e| e.role)
    }
    /// Planar boundary at the requested construction end, independent of display quality.
    #[must_use]
    pub fn cap(&self, end: End) -> Option<FaceId> {
        self.body
            .faces()
            .iter()
            .position(|f| f.role == FaceRole::Cap { end })
            .map(|i| FaceId(crate::usize_to_u32(i)))
    }
    /// Walls carrying a label at either source section, emitted once per face.
    pub fn faces_tagged(&self, tag: SegTag) -> impl Iterator<Item = FaceId> + '_ {
        self.body
            .faces()
            .iter()
            .enumerate()
            .filter_map(move |(i, f)| {
                matches!(f.role,FaceRole::Wall {tags,..} if tags.contains(&Some(tag)))
                    .then_some(FaceId(crate::usize_to_u32(i)))
            })
    }
    /// Remeshes exact source surfaces under the requested tolerance and budgets.
    ///
    /// # Errors
    /// Returns the independent body's bounded realization refusal.
    pub fn realize(&self, policy: &RealizationPolicy) -> Result<RealizedBody, RealizationError> {
        self.body.realize(self.frame, policy)
    }
    /// Remeshes with caller-selected material/region identities per native face.
    ///
    /// # Errors
    /// Returns the same failures as [`Self::realize`].
    pub fn realize_with_regions(
        &self,
        policy: &RealizationPolicy,
        region: impl Fn(FaceId, &crate::body::Face<FaceRole>) -> u32,
    ) -> Result<RealizedBody, RealizationError> {
        self.body.realize_with_regions(self.frame, policy, region)
    }
}

fn loops(profile: &Profile) -> impl Iterator<Item = (LoopRef, &exedra_profile::Loop)> {
    core::iter::once((LoopRef::Outer, profile.outer())).chain(
        profile
            .holes()
            .iter()
            .enumerate()
            .map(|(i, r)| (LoopRef::Hole(i), r)),
    )
}
fn loop_index(r: LoopRef) -> usize {
    match r {
        LoopRef::Outer => 0,
        LoopRef::Hole(i) => i + 1,
    }
}

#[cfg(test)]
mod tests;
