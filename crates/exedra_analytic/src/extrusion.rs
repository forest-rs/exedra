// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Closed solids extruded from line and arc profiles.
//!
//! An [`Extrusion`] sweeps a validated [`Profile`] along local +Z for a
//! positive length, placed by a rigid frame. Its boundary is exact: planar
//! caps at both ends, pocket floors, and one wall per boundary segment, planar
//! for a line and cylindrical for an arc. Geometry stays in local coordinates; the frame
//! places it.
//!
//! [`Extrusion::with_pocket`] cuts a [`Pocket`] from either end to a depth:
//! a hole that stops, with its own walls and a planar floor. A round pocket
//! is a blind bore. Pockets on one end are validated as holes of the profile;
//! pockets on opposite ends may overlap only where material remains between
//! them.
//!
//! Topology is shared, not duplicated per face. Every [`Boundary`] (profile
//! loop or pocket outline) is a band of walls between two levels: each
//! segment gives one edge per level and each vertex one seam along Z. Every
//! edge is used by exactly two faces through oriented [`Coedge`]s, once in
//! each direction. [`Extrusion::is_closed`] verifies that incidence: an
//! extrusion is a closed solid with outward-facing boundary loops.
//!
//! [`Extrusion::tessellate`] realizes the solid as a mesh from shared edge
//! samples and reports how closely the emitted coordinates follow it.
//!
//! ```
//! use exedra_analytic::extrusion::{Extrusion, TessellationPolicy};
//! use exedra_math::Placement3;
//! use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
//!
//! let disk = Loop::new(vec![Segment::arc([-1.0, 0.0], 1.0), Segment::arc([1.0, 0.0], 1.0)])?;
//! let profile = Profile::new(disk, vec![], &ProfilePolicy::new(1e-9))?;
//! let solid = Extrusion::new(profile, Placement3::IDENTITY, 2.0)?;
//! assert_eq!(solid.faces().len(), 4, "two caps and two cylindrical walls");
//! let realized = solid.tessellate(&TessellationPolicy::default())?;
//! assert!(realized.mesh.boundary_loops()?.is_empty());
//! assert!(realized.report.within_tolerance());
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

use alloc::vec::Vec;

use exedra_math::{FrameError, Placement3};
use exedra_profile::{Arc, Curve, Loop, LoopRef, Profile, ProfileError, ProfilePolicy, SegTag};

mod query;
mod selection;
mod tessellate;
mod workplane;

pub use query::SurfacePoint;
pub use workplane::{FaceSelectionError, Workplane, WorkplaneError};

pub use tessellate::{
    TessellateExtrusionError, TessellatedExtrusion, TessellationPolicy, TessellationReport,
    VertexSource,
};

/// Absolute tolerance on frame axis lengths and dot products.
pub const FRAME_TOLERANCE: f64 = 1e-12;

macro_rules! solid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(pub u32);

        impl $name {
            /// The zero-based index.
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

solid_id!(
    /// A vertex of one [`Extrusion`].
    VertexId
);
solid_id!(
    /// An edge of one [`Extrusion`].
    EdgeId
);
solid_id!(
    /// A face of one [`Extrusion`].
    FaceId
);

/// One end of an extrusion.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum End {
    /// The profile plane, `z = 0`.
    Start,
    /// The far end, `z = length`.
    End,
}

/// A height at which boundary edges and vertices lie.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    /// The start cap, `z = 0`.
    Start,
    /// The end cap, `z = length`.
    End,
    /// A pocket's floor, at its depth from its end.
    Floor,
}

impl From<End> for Level {
    fn from(end: End) -> Self {
        match end {
            End::Start => Self::Start,
            End::End => Self::End,
        }
    }
}

/// A closed boundary of the solid's cross-section.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// A loop of the extruded profile, running the full length.
    Profile(LoopRef),
    /// A pocket's outline, running from its end to its floor.
    Pocket(usize),
}

/// A segment of a [`Boundary`], indexed in its stored order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BoundarySegment {
    /// The boundary.
    pub boundary: Boundary,
    /// The segment's index in its loop.
    pub segment: usize,
}

/// A vertex: the end of a boundary segment at one level.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VertexRole {
    /// The boundary segment ending at this vertex.
    pub segment: BoundarySegment,
    /// The vertex's level.
    pub level: Level,
}

/// What an edge was generated from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EdgeRole {
    /// A boundary segment at one level.
    Boundary {
        /// The source segment.
        segment: BoundarySegment,
        /// The edge's level.
        level: Level,
    },
    /// The Z-parallel seam at the end vertex of a boundary segment, spanning
    /// its boundary's wall.
    Seam {
        /// The segment ending at the seam.
        segment: BoundarySegment,
    },
}

/// What a face was generated from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FaceRole {
    /// The cap at one end, with any pockets on that end as holes.
    Cap(End),
    /// The wall swept by a boundary segment.
    Wall {
        /// The source segment.
        segment: BoundarySegment,
        /// The segment's caller tag.
        tag: Option<SegTag>,
    },
    /// A pocket's floor.
    Floor {
        /// The pocket.
        pocket: usize,
    },
}

/// The exact curve of an edge, in local coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum EdgeCurve {
    /// A straight edge between its endpoints.
    Line {
        /// Start point.
        from: [f64; 3],
        /// End point.
        to: [f64; 3],
    },
    /// A circular arc in the plane `z`.
    Arc {
        /// The profile arc.
        arc: Arc,
        /// Height of its plane.
        z: f64,
    },
}

/// The exact support of a face, in local coordinates, with its outward sense.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Surface {
    /// The plane `dot(normal, p) = offset`, with `normal` unit and outward.
    Plane {
        /// Outward unit normal.
        normal: [f64; 3],
        /// Plane offset along the normal.
        offset: f64,
    },
    /// A cylinder about the line through `center` parallel to local Z.
    Cylinder {
        /// Axis position in the profile plane.
        center: [f64; 2],
        /// Radius; positive.
        radius: f64,
        /// Whether the outward normal points away from the axis (a convex
        /// wall) rather than toward it.
        outward: bool,
    },
}

/// An edge used by a face loop, in that loop's direction.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Coedge {
    /// The shared edge.
    pub edge: EdgeId,
    /// Whether the loop traverses the edge from its end to its start.
    pub reversed: bool,
}

/// One edge: its curve, endpoints and source.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Edge {
    /// The exact curve, from `start` to `end`.
    pub curve: EdgeCurve,
    /// Start vertex.
    pub start: VertexId,
    /// End vertex.
    pub end: VertexId,
    /// Source of the edge.
    pub role: EdgeRole,
}

/// One face: its support, boundary loops and source.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    /// The exact support, oriented outward.
    pub surface: Surface,
    /// Boundary loops, outer first, each counter-clockwise seen from outside.
    pub loops: Vec<Vec<Coedge>>,
    /// Source of the face.
    pub role: FaceRole,
}

/// A region cut from one end of an extrusion to a depth: a hole that stops.
///
/// The outline winds clockwise, like a profile hole, so the solid's material
/// stays to the left of every boundary. A round outline is a blind bore.
#[derive(Clone, Debug, PartialEq)]
pub struct Pocket {
    /// The clockwise outline in the profile plane.
    pub outline: Loop,
    /// The end the pocket opens on.
    pub end: End,
    /// Distance from that end to the floor; positive, and leaving at least
    /// the profile policy's separation before the other end.
    pub depth: f64,
    /// Caller feature label for the floor, walls and rims. Labels may repeat
    /// across pockets and do not replace this solid's local topology IDs.
    pub tag: Option<SegTag>,
}

impl Pocket {
    /// Removes a filled profile from an extrusion end, preserving its outer
    /// boundary tags. A region containing holes would leave islands, which
    /// this bounded pocket model does not support.
    ///
    /// # Errors
    ///
    /// Returns [`PocketError::RegionHasHoles`] with the number of holes.
    pub fn from_profile(region: &Profile, end: End, depth: f64) -> Result<Self, PocketError> {
        if !region.holes().is_empty() {
            return Err(PocketError::RegionHasHoles {
                count: region.holes().len(),
            });
        }
        Ok(Self::from_region(region.outer(), end, depth))
    }

    /// A pocket removing the region inside `region`, a counter-clockwise
    /// outline like a filled profile's, from `end` to `depth`.
    ///
    /// The outline is stored reversed, so tags and geometry carry over and
    /// only the orientation changes. A clockwise `region` yields an outline
    /// that [`Extrusion::with_pocket`] refuses as wrongly wound.
    ///
    /// ```
    /// use exedra_analytic::extrusion::{End, Extrusion, Pocket};
    /// use exedra_math::Placement3;
    /// use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    ///
    /// let square = |min: f64, max: f64| {
    ///     Loop::new(vec![
    ///         Segment::line([max, min]),
    ///         Segment::line([max, max]),
    ///         Segment::line([min, max]),
    ///         Segment::line([min, min]),
    ///     ])
    /// };
    /// let policy = ProfilePolicy::new(1e-9);
    /// let block = Extrusion::new(
    ///     Profile::new(square(0.0, 10.0)?, vec![], &policy)?,
    ///     Placement3::IDENTITY,
    ///     4.0,
    /// )?;
    /// let recessed = block.with_pocket(Pocket::from_region(&square(2.0, 8.0)?, End::End, 1.0), &policy)?;
    /// assert_eq!(recessed.volume(), 400.0 - 36.0);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    #[must_use]
    pub fn from_region(region: &Loop, end: End, depth: f64) -> Self {
        Self {
            outline: region.reversed(),
            end,
            depth,
            tag: None,
        }
    }

    /// Labels the whole pocket without replacing its segments' source tags.
    /// Native feature queries prefer this label on its floor, walls and rims.
    #[must_use]
    pub const fn tagged(mut self, tag: SegTag) -> Self {
        self.tag = Some(tag);
        self
    }
}

/// Why a pocket cannot be cut.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PocketError {
    /// A filled pocket region contains holes, which would leave unsupported islands.
    RegionHasHoles {
        /// Number of holes in the region.
        count: usize,
    },
    /// The depth is not finite, not positive, or leaves less than the
    /// separation before the other end.
    InvalidDepth,
    /// Against the profile's loops and the other pockets on its end, the
    /// outline is not a valid hole. Holes past the profile's own index the
    /// earlier pockets on this end, then the new outline.
    Arrangement(ProfileError),
    /// The pocket meets one on the other end: their regions overlap and
    /// their floors are closer than the separation.
    BreaksThrough {
        /// The pocket on the other end.
        other: usize,
    },
}

impl core::fmt::Display for PocketError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RegionHasHoles { count } => write!(
                f,
                "pocket region has {count} holes; this pocket model requires one filled outline"
            ),
            Self::InvalidDepth => {
                write!(f, "pocket depth must be positive and less than the length")
            }
            Self::Arrangement(error) => write!(f, "pocket outline: {error}"),
            Self::BreaksThrough { other } => {
                write!(f, "pocket meets pocket {other} from the other end")
            }
        }
    }
}

impl core::error::Error for PocketError {}

/// Why an extrusion cannot be built.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ExtrusionError {
    /// The length is zero, negative or non-finite.
    InvalidLength,
    /// The frame is not a finite right-handed orthonormal frame within
    /// [`FRAME_TOLERANCE`].
    SingularFrame(FrameError),
    /// The profile and pockets have more vertices than the identifier space
    /// allows.
    TooLarge,
    /// A pocket cannot be cut.
    Pocket {
        /// The pocket's index, the count of earlier pockets.
        pocket: usize,
        /// Why.
        error: PocketError,
    },
}

impl core::fmt::Display for ExtrusionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidLength => write!(f, "extrusion length must be finite and positive"),
            Self::SingularFrame(error) => write!(f, "extrusion frame: {error}"),
            Self::TooLarge => write!(f, "profile is too large to extrude"),
            Self::Pocket { pocket, error } => write!(f, "pocket {pocket}: {error}"),
        }
    }
}

impl core::error::Error for ExtrusionError {}

/// Where one boundary's wall band sits among the solid's tables.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Band {
    pub(crate) boundary: Boundary,
    /// Lower and upper level, and their heights.
    pub(crate) levels: [Level; 2],
    pub(crate) heights: [f64; 2],
    /// First vertex and first edge of each level's ring.
    pub(crate) vertices: [usize; 2],
    pub(crate) edges: [usize; 2],
    pub(crate) seams: usize,
    pub(crate) walls: usize,
}

/// A planar face as rings of bands: caps and pocket floors.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlanarFace {
    pub(crate) face: usize,
    /// `(band, level slot)` per ring, the face's outer boundary first.
    pub(crate) rings: Vec<(usize, usize)>,
    /// Whether the face's region lies right of its stored rings (a floor)
    /// rather than left (a cap).
    pub(crate) region_right: bool,
    /// Whether the face looks along +Z.
    pub(crate) upward: bool,
}

/// A closed solid swept from a profile along local +Z, with pockets.
#[derive(Clone, Debug, PartialEq)]
pub struct Extrusion {
    profile: Profile,
    frame: Placement3,
    length: f64,
    pockets: Vec<Pocket>,
    /// `[end pocket, start pocket]` pairs whose regions overlap, leaving a
    /// web of material between their floors.
    webs: Vec<[usize; 2]>,
    vertices: Vec<[f64; 3]>,
    vertex_roles: Vec<VertexRole>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
    bands: Vec<Band>,
    planes: Vec<PlanarFace>,
}

impl Extrusion {
    /// Extrudes `profile` from its plane to `z = length`, placed by `frame`.
    ///
    /// # Errors
    ///
    /// Returns [`ExtrusionError::InvalidLength`] for a nonpositive or
    /// non-finite length and [`ExtrusionError::SingularFrame`] for a frame
    /// that is not right-handed and orthonormal.
    pub fn new(profile: Profile, frame: Placement3, length: f64) -> Result<Self, ExtrusionError> {
        if !(length.is_finite() && length > 0.0) {
            return Err(ExtrusionError::InvalidLength);
        }
        Workplane::new(frame).map_err(ExtrusionError::SingularFrame)?;
        Self::build(profile, frame, length, Vec::new(), Vec::new())
    }

    /// This solid with `pocket` cut from its end.
    ///
    /// The outline must be a valid hole of the profile, apart from the other
    /// pockets on the same end under `policy`. Its floor must stay at least
    /// the policy's separation from the other end, and a pocket on the other
    /// end may overlap it only where their floors are at least that far
    /// apart.
    ///
    /// # Errors
    ///
    /// Returns [`ExtrusionError::Pocket`] naming the new pocket and the
    /// failed check.
    pub fn with_pocket(
        self,
        pocket: Pocket,
        policy: &ProfilePolicy,
    ) -> Result<Self, ExtrusionError> {
        let index = self.pockets.len();
        let refuse = |error| ExtrusionError::Pocket {
            pocket: index,
            error,
        };
        let separation = policy.min_separation;
        // Compare the remaining material directly: adding a small separation
        // to a depth can round back to that depth and admit a zero web.
        if !(pocket.depth.is_finite()
            && pocket.depth > 0.0
            && self.length - pocket.depth >= separation)
        {
            return Err(refuse(PocketError::InvalidDepth));
        }
        let holes_with = |others: &mut dyn Iterator<Item = &Loop>| {
            let mut holes = self.profile.holes().to_vec();
            holes.extend(others.cloned());
            holes.push(pocket.outline.clone());
            Profile::new(self.profile.outer().clone(), holes, policy)
        };
        holes_with(
            &mut self
                .pockets
                .iter()
                .filter(|p| p.end == pocket.end)
                .map(|p| &p.outline),
        )
        .map_err(|error| refuse(PocketError::Arrangement(error)))?;
        // Opposite pockets whose regions overlap share a web of material
        // between their floors.
        let mut webs = self.webs.clone();
        for (other, existing) in self.pockets.iter().enumerate() {
            if existing.end == pocket.end
                || holes_with(&mut core::iter::once(&existing.outline)).is_ok()
            {
                continue;
            }
            let (end_depth, start_depth) = if pocket.end == End::End {
                (pocket.depth, existing.depth)
            } else {
                (existing.depth, pocket.depth)
            };
            // Use the same floor coordinates stored by build().
            if (self.length - end_depth) - start_depth < separation {
                return Err(refuse(PocketError::BreaksThrough { other }));
            }
            webs.push(if pocket.end == End::End {
                [index, other]
            } else {
                [other, index]
            });
        }
        let Self {
            profile,
            frame,
            length,
            mut pockets,
            ..
        } = self;
        pockets.push(pocket);
        Self::build(profile, frame, length, pockets, webs)
    }

    #[expect(clippy::too_many_lines, reason = "one pass over every band")]
    fn build(
        profile: Profile,
        frame: Placement3,
        length: f64,
        pockets: Vec<Pocket>,
        webs: Vec<[usize; 2]>,
    ) -> Result<Self, ExtrusionError> {
        let mut boundaries: Vec<(Boundary, [Level; 2], [f64; 2], &Loop)> =
            core::iter::once((LoopRef::Outer, profile.outer()))
                .chain(
                    profile
                        .holes()
                        .iter()
                        .enumerate()
                        .map(|(hole, loop_)| (LoopRef::Hole(hole), loop_)),
                )
                .map(|(loop_ref, loop_)| {
                    (
                        Boundary::Profile(loop_ref),
                        [Level::Start, Level::End],
                        [0.0, length],
                        loop_,
                    )
                })
                .collect();
        for (index, pocket) in pockets.iter().enumerate() {
            let (levels, heights) = match pocket.end {
                End::Start => ([Level::Start, Level::Floor], [0.0, pocket.depth]),
                End::End => ([Level::Floor, Level::End], [length - pocket.depth, length]),
            };
            boundaries.push((Boundary::Pocket(index), levels, heights, &pocket.outline));
        }
        let count: usize = boundaries.iter().map(|(.., l)| l.segments().len()).sum();
        if count
            .checked_mul(3)
            .and_then(|n| n.checked_add(2 + pockets.len()))
            .is_none_or(|n| u32::try_from(n).is_err())
        {
            return Err(ExtrusionError::TooLarge);
        }
        let mut vertices = Vec::with_capacity(2 * count);
        let mut vertex_roles = Vec::with_capacity(2 * count);
        let mut edges = Vec::with_capacity(3 * count);
        let mut bands = Vec::with_capacity(boundaries.len());
        let vertex_id = |index: usize| VertexId(crate::usize_to_u32(index));
        // Two caps first, then one wall per boundary segment, then floors.
        let mut walls = 2;
        for &(boundary, levels, heights, loop_) in &boundaries {
            let n = loop_.segments().len();
            let mut band = Band {
                boundary,
                levels,
                heights,
                vertices: [0; 2],
                edges: [0; 2],
                seams: 0,
                walls,
            };
            walls += n;
            for slot in 0..2 {
                band.vertices[slot] = vertices.len();
                for (segment, s) in loop_.segments().iter().enumerate() {
                    vertices.push([s.to[0], s.to[1], heights[slot]]);
                    vertex_roles.push(VertexRole {
                        segment: BoundarySegment { boundary, segment },
                        level: levels[slot],
                    });
                }
            }
            for slot in 0..2 {
                band.edges[slot] = edges.len();
                let z = heights[slot];
                for (k, (from, segment)) in loop_.iter_with_starts().enumerate() {
                    let curve = match segment.curve {
                        Curve::Line => EdgeCurve::Line {
                            from: [from[0], from[1], z],
                            to: [segment.to[0], segment.to[1], z],
                        },
                        Curve::Arc { .. } => EdgeCurve::Arc {
                            arc: loop_.arc(k).expect("validated arc"),
                            z,
                        },
                    };
                    edges.push(Edge {
                        curve,
                        start: vertex_id(band.vertices[slot] + (k + n - 1) % n),
                        end: vertex_id(band.vertices[slot] + k),
                        role: EdgeRole::Boundary {
                            segment: BoundarySegment {
                                boundary,
                                segment: k,
                            },
                            level: levels[slot],
                        },
                    });
                }
            }
            band.seams = edges.len();
            for (k, segment) in loop_.segments().iter().enumerate() {
                edges.push(Edge {
                    curve: EdgeCurve::Line {
                        from: [segment.to[0], segment.to[1], heights[0]],
                        to: [segment.to[0], segment.to[1], heights[1]],
                    },
                    start: vertex_id(band.vertices[0] + k),
                    end: vertex_id(band.vertices[1] + k),
                    role: EdgeRole::Seam {
                        segment: BoundarySegment {
                            boundary,
                            segment: k,
                        },
                    },
                });
            }
            bands.push(band);
        }
        // Planar faces as rings of bands: each cap holds every profile loop
        // and the pockets opening on it; each floor is its pocket's outline.
        let mut planes = Vec::with_capacity(2 + pockets.len());
        for (face, end) in [End::Start, End::End].into_iter().enumerate() {
            let slot = usize::from(end == End::End);
            let rings = bands
                .iter()
                .enumerate()
                .filter(|(_, band)| match band.boundary {
                    Boundary::Profile(_) => true,
                    Boundary::Pocket(pocket) => pockets[pocket].end == end,
                })
                .map(|(index, _)| (index, slot))
                .collect();
            planes.push(PlanarFace {
                face,
                rings,
                region_right: false,
                upward: end == End::End,
            });
        }
        for (pocket, p) in pockets.iter().enumerate() {
            let band = boundaries.len() - pockets.len() + pocket;
            planes.push(PlanarFace {
                face: walls + pocket,
                rings: alloc::vec![(band, usize::from(p.end == End::Start))],
                region_right: true,
                upward: p.end == End::End,
            });
        }
        let edge = |index: usize| EdgeId(crate::usize_to_u32(index));
        let mut faces = Vec::with_capacity(walls + pockets.len());
        for plane in &planes[..2] {
            faces.push(plane_face(plane, &bands, &boundaries, edge));
        }
        for (band, &(boundary, _, _, loop_)) in bands.iter().zip(&boundaries) {
            let n = loop_.segments().len();
            for (k, (from, segment)) in loop_.iter_with_starts().enumerate() {
                let surface = match segment.curve {
                    Curve::Line => {
                        let direction = [segment.to[0] - from[0], segment.to[1] - from[1]];
                        let span = exedra_math::norm([direction[0], direction[1], 0.0]);
                        // Material is left of the travel direction, so the
                        // outward normal is to its right.
                        let normal = [direction[1] / span, -direction[0] / span, 0.0];
                        Surface::Plane {
                            normal,
                            offset: normal[0] * from[0] + normal[1] * from[1],
                        }
                    }
                    Curve::Arc { .. } => {
                        let arc = loop_.arc(k).expect("validated arc");
                        Surface::Cylinder {
                            center: arc.center,
                            radius: arc.radius,
                            outward: arc.sweep > 0.0,
                        }
                    }
                };
                // Lower edge forward, seam up at the segment's end, upper
                // edge backward, seam down at its start.
                let boundary_loop = alloc::vec![
                    Coedge {
                        edge: edge(band.edges[0] + k),
                        reversed: false,
                    },
                    Coedge {
                        edge: edge(band.seams + k),
                        reversed: false,
                    },
                    Coedge {
                        edge: edge(band.edges[1] + k),
                        reversed: true,
                    },
                    Coedge {
                        edge: edge(band.seams + (k + n - 1) % n),
                        reversed: true,
                    },
                ];
                faces.push(Face {
                    surface,
                    loops: alloc::vec![boundary_loop],
                    role: FaceRole::Wall {
                        segment: BoundarySegment {
                            boundary,
                            segment: k,
                        },
                        tag: segment.tag,
                    },
                });
            }
        }
        for plane in &planes[2..] {
            faces.push(plane_face(plane, &bands, &boundaries, edge));
        }
        let solid = Self {
            profile,
            frame,
            length,
            pockets,
            webs,
            vertices,
            vertex_roles,
            edges,
            faces,
            bands,
            planes,
        };
        debug_assert!(
            solid.is_closed(),
            "extrusion topology is closed by construction"
        );
        Ok(solid)
    }

    /// The pockets cut so far, in order.
    #[must_use]
    pub fn pockets(&self) -> &[Pocket] {
        &self.pockets
    }

    pub(crate) fn bands(&self) -> &[Band] {
        &self.bands
    }

    pub(crate) fn webs(&self) -> &[[usize; 2]] {
        &self.webs
    }

    pub(crate) fn planes(&self) -> &[PlanarFace] {
        &self.planes
    }

    /// The stored loop of `boundary`: the profile loop, or the pocket outline.
    pub(crate) fn boundary_loop(&self, boundary: Boundary) -> &Loop {
        match boundary {
            Boundary::Profile(LoopRef::Outer) => self.profile.outer(),
            Boundary::Profile(LoopRef::Hole(hole)) => &self.profile.holes()[hole],
            Boundary::Pocket(pocket) => &self.pockets[pocket].outline,
        }
    }

    /// The extruded profile.
    #[must_use]
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// The rigid frame placing local coordinates.
    #[must_use]
    pub fn frame(&self) -> Placement3 {
        self.frame
    }

    /// Distance along local +Z from the start cap to the end cap.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.length
    }

    /// Vertex positions in local coordinates.
    #[must_use]
    pub fn vertices(&self) -> &[[f64; 3]] {
        &self.vertices
    }

    /// The profile vertex and end each vertex was generated from.
    #[must_use]
    pub fn vertex_role(&self, vertex: VertexId) -> Option<VertexRole> {
        self.vertex_roles.get(vertex.index()).copied()
    }

    /// Edges in deterministic order: start-end profile edges, end profile
    /// edges, then seams, each in outer-then-hole segment order.
    #[must_use]
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Faces in deterministic order: start cap, end cap, then walls in
    /// outer-then-hole segment order.
    #[must_use]
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// Whether every edge is used once in each direction by face loops whose
    /// coedges chain end to start: a closed, consistently oriented boundary.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        let mut uses = alloc::vec![[0_u32; 2]; self.edges.len()];
        for face in &self.faces {
            for loop_ in &face.loops {
                for (index, coedge) in loop_.iter().enumerate() {
                    let next = loop_[(index + 1) % loop_.len()];
                    if self.coedge_end(*coedge) != self.coedge_start(next) {
                        return false;
                    }
                    uses[coedge.edge.index()][usize::from(coedge.reversed)] += 1;
                }
            }
        }
        uses.iter().all(|counts| *counts == [1, 1])
    }

    fn coedge_start(&self, coedge: Coedge) -> VertexId {
        let edge = &self.edges[coedge.edge.index()];
        if coedge.reversed {
            edge.end
        } else {
            edge.start
        }
    }

    fn coedge_end(&self, coedge: Coedge) -> VertexId {
        let edge = &self.edges[coedge.edge.index()];
        if coedge.reversed {
            edge.start
        } else {
            edge.end
        }
    }
}

/// The face for one planar ring set: rings traversed so the face's region
/// lies to their left seen from outside.
fn plane_face(
    plane: &PlanarFace,
    bands: &[Band],
    boundaries: &[(Boundary, [Level; 2], [f64; 2], &Loop)],
    edge: impl Fn(usize) -> EdgeId,
) -> Face {
    // Seen from below, left and right swap; a floor's region is right of its
    // stored ring.
    let reversed = plane.upward == plane.region_right;
    let loops = plane
        .rings
        .iter()
        .map(|&(band, slot)| {
            let n = boundaries[band].3.segments().len();
            let first = bands[band].edges[slot];
            let mut coedges: Vec<Coedge> = (0..n)
                .map(|k| Coedge {
                    edge: edge(first + k),
                    reversed,
                })
                .collect();
            if reversed {
                coedges.reverse();
            }
            coedges
        })
        .collect();
    let height = bands[plane.rings[0].0].heights[plane.rings[0].1];
    let (normal, offset) = if plane.upward {
        ([0.0, 0.0, 1.0], height)
    } else {
        ([0.0, 0.0, -1.0], -height)
    };
    let role = match boundaries[plane.rings[0].0].0 {
        Boundary::Pocket(pocket) if plane.region_right => FaceRole::Floor { pocket },
        _ => FaceRole::Cap(if plane.upward { End::End } else { End::Start }),
    };
    Face {
        surface: Surface::Plane { normal, offset },
        loops,
        role,
    }
}

#[cfg(test)]
mod pocket_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_profile::{Loop, ProfilePolicy, Segment};

    fn rect_with_round_hole() -> Profile {
        let outer = Loop::new(vec![
            Segment::line([4.0, 0.0]).tagged(SegTag(10)),
            Segment::line([4.0, 2.0]),
            Segment::line([0.0, 2.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        let hole = Loop::new(vec![
            Segment::arc([1.5, 1.0], -1.0),
            Segment::arc([2.5, 1.0], -1.0),
        ])
        .unwrap();
        Profile::new(outer, vec![hole], &ProfilePolicy::new(1e-9)).unwrap()
    }

    #[test]
    fn walls_caps_and_seams_share_every_edge_once_each_way() {
        let solid = Extrusion::new(rect_with_round_hole(), Placement3::IDENTITY, 3.0).unwrap();
        // Six profile segments: two vertices, three edges and one wall each.
        assert_eq!(solid.vertices().len(), 12);
        assert_eq!(solid.edges().len(), 18);
        assert_eq!(solid.faces().len(), 8);
        assert!(solid.is_closed());
        assert_eq!(solid.faces()[0].role, FaceRole::Cap(End::Start));
        assert_eq!(solid.faces()[0].loops.len(), 2, "outer boundary and hole");
        assert_eq!(
            solid.faces()[2].role,
            FaceRole::Wall {
                segment: BoundarySegment {
                    boundary: Boundary::Profile(LoopRef::Outer),
                    segment: 0
                },
                tag: Some(SegTag(10)),
            }
        );
        // Any face missing a coedge leaves its partner unpaired.
        let mut open = solid.clone();
        open.faces[3].loops[0].pop();
        assert!(!open.is_closed());
    }

    #[test]
    fn supports_face_outward() {
        let solid = Extrusion::new(rect_with_round_hole(), Placement3::IDENTITY, 3.0).unwrap();
        let planes: Vec<_> = solid.faces()[2..6]
            .iter()
            .map(|face| face.surface)
            .collect();
        assert_eq!(
            planes,
            [
                Surface::Plane {
                    normal: [0.0, -1.0, 0.0],
                    offset: 0.0
                },
                Surface::Plane {
                    normal: [1.0, 0.0, 0.0],
                    offset: 4.0
                },
                Surface::Plane {
                    normal: [0.0, 1.0, 0.0],
                    offset: 2.0
                },
                Surface::Plane {
                    normal: [-1.0, 0.0, 0.0],
                    offset: 0.0
                },
            ]
        );
        // The hole's walls face into the hole, toward the axis.
        for face in &solid.faces()[6..] {
            let Surface::Cylinder {
                center,
                radius,
                outward,
            } = face.surface
            else {
                panic!("arc walls are cylinders");
            };
            assert_eq!((center, radius, outward), ([2.0, 1.0], 0.5, false));
        }
        assert_eq!(
            solid.faces()[1].surface,
            Surface::Plane {
                normal: [0.0, 0.0, 1.0],
                offset: 3.0
            }
        );
    }

    #[test]
    fn length_and_frame_are_checked() {
        for length in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Extrusion::new(rect_with_round_hole(), Placement3::IDENTITY, length),
                Err(ExtrusionError::InvalidLength)
            );
        }
        let mirrored =
            Placement3::from_axes([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
        let scaled =
            Placement3::from_axes([2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
        for frame in [mirrored, scaled] {
            assert!(matches!(
                Extrusion::new(rect_with_round_hole(), frame, 1.0),
                Err(ExtrusionError::SingularFrame(_))
            ));
        }
        let turned = Placement3::rotate_x_then_translate(1.0, 5.0, 0.0, -2.0);
        assert_eq!(
            Extrusion::new(rect_with_round_hole(), turned, 1.0)
                .unwrap()
                .frame(),
            turned
        );
    }
}
