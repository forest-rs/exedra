// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Closed solids extruded from line and arc profiles.
//!
//! An [`Extrusion`] sweeps a validated [`Profile`] along local +Z for a
//! positive length, placed by a rigid frame. Its boundary is exact: planar
//! caps at both ends, and one wall per profile segment, planar for a line and
//! cylindrical for an arc. Geometry stays in local coordinates; the frame
//! places it.
//!
//! Topology is shared, not duplicated per face. Every profile segment gives
//! one edge at each end, every profile vertex gives one seam edge along Z, and
//! each edge is used by exactly two faces through oriented [`Coedge`]s, once
//! in each direction. [`Extrusion::is_closed`] verifies that incidence: an
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
use exedra_profile::{Arc, Curve, LoopRef, Profile, SegmentRef, SegmentTag};

mod query;
mod tessellate;

pub use query::SurfacePoint;

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

/// A profile vertex: the end of `segment` in its loop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VertexRole {
    /// The profile segment ending at this vertex.
    pub segment: SegmentRef,
    /// Which end of the extrusion.
    pub end: End,
}

/// What an edge was generated from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EdgeRole {
    /// A profile segment at one end.
    Profile {
        /// The source segment.
        segment: SegmentRef,
        /// Which end.
        end: End,
    },
    /// The Z-parallel seam at the end vertex of a profile segment.
    Seam {
        /// The segment ending at the seam.
        segment: SegmentRef,
    },
}

/// What a face was generated from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FaceRole {
    /// The cap at one end.
    Cap(End),
    /// The wall swept by a profile segment.
    Wall {
        /// The source segment.
        segment: SegmentRef,
        /// The segment's caller tag.
        tag: Option<SegmentTag>,
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

/// Why an extrusion cannot be built.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ExtrusionError {
    /// The length is zero, negative or non-finite.
    InvalidLength,
    /// The frame is not a finite right-handed orthonormal frame within
    /// [`FRAME_TOLERANCE`].
    SingularFrame(FrameError),
    /// The profile has more vertices than the identifier space allows.
    TooLarge,
}

impl core::fmt::Display for ExtrusionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidLength => write!(f, "extrusion length must be finite and positive"),
            Self::SingularFrame(error) => write!(f, "extrusion frame: {error}"),
            Self::TooLarge => write!(f, "profile is too large to extrude"),
        }
    }
}

impl core::error::Error for ExtrusionError {}

/// A closed solid swept from a profile along local +Z.
#[derive(Clone, Debug, PartialEq)]
pub struct Extrusion {
    profile: Profile,
    frame: Placement3,
    length: f64,
    vertices: Vec<[f64; 3]>,
    vertex_roles: Vec<VertexRole>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
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
        let column = |axis: usize| frame.rows.map(|row| row[axis]);
        Placement3::try_from_orthonormal_axes(
            column(0),
            column(1),
            column(2),
            column(3),
            FRAME_TOLERANCE,
        )
        .map_err(ExtrusionError::SingularFrame)?;
        let loops: Vec<(LoopRef, &exedra_profile::Loop)> =
            core::iter::once((LoopRef::Outer, profile.outer()))
                .chain(
                    profile
                        .holes()
                        .iter()
                        .enumerate()
                        .map(|(hole, loop_)| (LoopRef::Hole(hole), loop_)),
                )
                .collect();
        let count: usize = loops.iter().map(|(_, l)| l.segments().len()).sum();
        if count
            .checked_mul(3)
            .and_then(|n| n.checked_add(2))
            .is_none_or(|n| u32::try_from(n).is_err())
        {
            return Err(ExtrusionError::TooLarge);
        }
        let mut solid = Parts {
            vertices: Vec::with_capacity(2 * count),
            vertex_roles: Vec::with_capacity(2 * count),
            edges: Vec::with_capacity(3 * count),
            faces: Vec::with_capacity(count + 2),
        };
        // Vertices and edges for each end, then seams.
        let mut first_vertex = Vec::with_capacity(loops.len());
        let mut first_edge = [Vec::with_capacity(loops.len()), Vec::new()];
        for end in [End::Start, End::End] {
            let z = if end == End::Start { 0.0 } else { length };
            for (loop_ref, loop_) in &loops {
                if end == End::Start {
                    first_vertex.push(solid.vertices.len());
                }
                for (segment, s) in loop_.segments().iter().enumerate() {
                    solid.vertices.push([s.to[0], s.to[1], z]);
                    solid.vertex_roles.push(VertexRole {
                        segment: SegmentRef {
                            loop_ref: *loop_ref,
                            segment,
                        },
                        end,
                    });
                }
            }
        }
        let half = solid.vertices.len() / 2;
        let vertex = |loop_index: usize, k: usize, end: End| {
            let base = first_vertex[loop_index] + if end == End::Start { 0 } else { half };
            VertexId(crate::usize_to_u32(base + k))
        };
        for (end_index, end) in [End::Start, End::End].into_iter().enumerate() {
            let z = if end == End::Start { 0.0 } else { length };
            for (loop_index, (loop_ref, loop_)) in loops.iter().enumerate() {
                first_edge[end_index].push(solid.edges.len());
                let n = loop_.segments().len();
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
                    solid.edges.push(Edge {
                        curve,
                        start: vertex(loop_index, (k + n - 1) % n, end),
                        end: vertex(loop_index, k, end),
                        role: EdgeRole::Profile {
                            segment: SegmentRef {
                                loop_ref: *loop_ref,
                                segment: k,
                            },
                            end,
                        },
                    });
                }
            }
        }
        let mut first_seam = Vec::with_capacity(loops.len());
        for (loop_index, (loop_ref, loop_)) in loops.iter().enumerate() {
            first_seam.push(solid.edges.len());
            for (k, segment) in loop_.segments().iter().enumerate() {
                solid.edges.push(Edge {
                    curve: EdgeCurve::Line {
                        from: [segment.to[0], segment.to[1], 0.0],
                        to: [segment.to[0], segment.to[1], length],
                    },
                    start: vertex(loop_index, k, End::Start),
                    end: vertex(loop_index, k, End::End),
                    role: EdgeRole::Seam {
                        segment: SegmentRef {
                            loop_ref: *loop_ref,
                            segment: k,
                        },
                    },
                });
            }
        }
        let edge = |index: usize| EdgeId(crate::usize_to_u32(index));
        // Caps: the end cap follows the profile; the start cap, facing -Z,
        // traverses every loop backwards.
        for (end_index, end) in [End::Start, End::End].into_iter().enumerate() {
            let reversed = end == End::Start;
            let loops_of_cap = loops
                .iter()
                .enumerate()
                .map(|(loop_index, (_, loop_))| {
                    let first = first_edge[end_index][loop_index];
                    let mut coedges: Vec<Coedge> = (0..loop_.segments().len())
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
            let (normal, offset) = if reversed {
                ([0.0, 0.0, -1.0], 0.0)
            } else {
                ([0.0, 0.0, 1.0], length)
            };
            solid.faces.push(Face {
                surface: Surface::Plane { normal, offset },
                loops: loops_of_cap,
                role: FaceRole::Cap(end),
            });
        }
        // Walls: start edge forward, seam up at the segment's end, end edge
        // backward, seam down at its start.
        for (loop_index, (loop_ref, loop_)) in loops.iter().enumerate() {
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
                let boundary = alloc::vec![
                    Coedge {
                        edge: edge(first_edge[0][loop_index] + k),
                        reversed: false,
                    },
                    Coedge {
                        edge: edge(first_seam[loop_index] + k),
                        reversed: false,
                    },
                    Coedge {
                        edge: edge(first_edge[1][loop_index] + k),
                        reversed: true,
                    },
                    Coedge {
                        edge: edge(first_seam[loop_index] + (k + n - 1) % n),
                        reversed: true,
                    },
                ];
                solid.faces.push(Face {
                    surface,
                    loops: alloc::vec![boundary],
                    role: FaceRole::Wall {
                        segment: SegmentRef {
                            loop_ref: *loop_ref,
                            segment: k,
                        },
                        tag: segment.tag,
                    },
                });
            }
        }
        let Parts {
            vertices,
            vertex_roles,
            edges,
            faces,
        } = solid;
        let solid = Self {
            profile,
            frame,
            length,
            vertices,
            vertex_roles,
            edges,
            faces,
        };
        debug_assert!(
            solid.is_closed(),
            "extrusion topology is closed by construction"
        );
        Ok(solid)
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

/// Tables filled while an [`Extrusion`] is built.
struct Parts {
    vertices: Vec<[f64; 3]>,
    vertex_roles: Vec<VertexRole>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_profile::{Loop, ProfilePolicy, Segment};

    fn rect_with_round_hole() -> Profile {
        let outer = Loop::new(vec![
            Segment::line([4.0, 0.0]).tagged(SegmentTag(10)),
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
                segment: SegmentRef {
                    loop_ref: LoopRef::Outer,
                    segment: 0
                },
                tag: Some(SegmentTag(10)),
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
