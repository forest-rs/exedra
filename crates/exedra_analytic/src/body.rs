// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared boundary topology over construction-independent f64 geometry.
//!
//! A body owns vertices, bounded curves, loops of oriented edge uses and
//! bounded supporting surfaces. Caller metadata is carried separately through
//! `F` and `E`; queries never interpret it. IDs are local table indexes, not
//! ownership tokens. Structural validation and curve/support agreement do not
//! establish absence of self-intersections or containment of nested shells.
//! A closed boundary is not by itself a certified regular solid.

use crate::geometry::{Curve, ParameterDomain, ParameterRange, Surface, SurfaceEvaluation};
use alloc::vec::Vec;
use exedra_math::Placement3;

macro_rules! local_id {
    ($name:ident, $doc:literal) => {
        #[doc=$doc]
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(pub u32);
        impl $name {
            /// Creates a local table index; lookup still checks that it exists.
            #[must_use]
            pub const fn from_index(index: u32) -> Self {
                Self(index)
            }
            /// The zero-based table index.
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}
local_id!(VertexId, "A vertex index local to one body.");
local_id!(EdgeId, "An edge index local to one body.");
local_id!(LoopId, "A boundary loop index local to one body.");
local_id!(FaceId, "A face index local to one body.");

/// A face's orientation relative to its supporting geometry's normal.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Sense {
    /// The supporting normal is the face normal.
    Forward,
    /// The face normal opposes the supporting normal.
    Reverse,
}
impl Sense {
    /// The signed normal multiplier.
    #[must_use]
    pub const fn sign(self) -> f64 {
        match self {
            Self::Forward => 1.0,
            Self::Reverse => -1.0,
        }
    }
}

/// One bounded curve, with authoritative endpoint vertex identities.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge<E = ()> {
    /// Untrimmed 3D supporting curve.
    pub curve: Curve,
    /// Increasing curve-parameter interval, independent of face traversal.
    pub range: ParameterRange,
    /// Vertex at the interval's minimum; equal endpoints represent a closed edge.
    pub start: VertexId,
    /// Vertex at the interval's maximum.
    pub end: VertexId,
    /// Caller metadata; independent of the edge's geometric definition.
    pub role: E,
}

/// One occurrence of an edge in a face boundary.
///
/// The same edge can occur twice on one face, for example a periodic seam.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Coedge {
    /// Shared bounded edge.
    pub edge: EdgeId,
    /// Traversal from the edge's end to its start.
    pub reversed: bool,
}

/// An ordered, closed chain of edge occurrences.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryLoop {
    /// Boundary traversal in the owning face's outward sense.
    pub coedges: Vec<Coedge>,
}

/// A trimmed use of a supporting surface, with separate orientation and metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct Face<F = ()> {
    /// Untrimmed supporting geometry in body-local coordinates.
    pub surface: Surface,
    /// Finite rectangular parameter bounds; loops provide the actual trimming.
    pub domain: ParameterDomain,
    /// Face normal relative to the supporting normal.
    pub sense: Sense,
    /// Outer loop followed by inner loops, all oriented in the face's sense.
    pub loops: Vec<LoopId>,
    /// Caller metadata; geometry queries never interpret it.
    pub role: F,
}

/// Explicit input tables for a boundary representation.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyParts<F = (), E = ()> {
    /// Authoritative body-local vertex positions.
    pub vertices: Vec<[f64; 3]>,
    /// Bounded shared curves.
    pub edges: Vec<Edge<E>>,
    /// Boundary occurrence chains.
    pub loops: Vec<BoundaryLoop>,
    /// Trimmed surface uses.
    pub faces: Vec<Face<F>>,
}
impl<F, E> Default for BodyParts<F, E> {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            edges: Vec::new(),
            loops: Vec::new(),
            faces: Vec::new(),
        }
    }
}

/// Maximum allowed geometric gap when validating already allocated input tables.
///
/// This controls agreement between representations, not profile separation,
/// tessellation error or healing. Validation never changes geometry or tolerances.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ValidationPolicy {
    /// Maximum endpoint-to-curve and curve-to-surface distance in model units.
    pub max_gap: f64,
}

/// Evidence about incidence and geometric support checks.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ValidationReport {
    /// Largest measured endpoint gap or conservative curve/support gap bound.
    pub max_gap: f64,
    /// Number of face-edge occurrences checked.
    pub coedges: usize,
}

/// A boundary representation independent of its construction history.
#[derive(Clone, Debug, PartialEq)]
pub struct Body<F = (), E = ()> {
    pub(crate) parts: BodyParts<F, E>,
}

impl<F, E> Body<F, E> {
    /// Checks references, vertex-identity closure and analytic support agreement.
    ///
    /// # Errors
    /// Returns the first [`BodyError`], with the offending local IDs and bounds.
    /// This does not certify trim nesting, global self-intersection or shell containment.
    pub fn try_from_parts(
        parts: BodyParts<F, E>,
        policy: &ValidationPolicy,
    ) -> Result<Self, BodyError> {
        let body = Self { parts };
        body.validate(policy)?;
        Ok(body)
    }

    /// Consumes the body into explicit tables for editing or serialization.
    #[must_use]
    pub fn into_parts(self) -> BodyParts<F, E> {
        self.parts
    }
    /// Vertex positions in deterministic table order.
    #[must_use]
    pub fn vertices(&self) -> &[[f64; 3]] {
        &self.parts.vertices
    }
    /// Shared edges in deterministic table order.
    #[must_use]
    pub fn edges(&self) -> &[Edge<E>] {
        &self.parts.edges
    }
    /// Boundary loops in deterministic table order.
    #[must_use]
    pub fn loops(&self) -> &[BoundaryLoop] {
        &self.parts.loops
    }
    /// Faces in deterministic table order.
    #[must_use]
    pub fn faces(&self) -> &[Face<F>] {
        &self.parts.faces
    }
    /// A face by its checked local index.
    #[must_use]
    pub fn face(&self, face: FaceId) -> Option<&Face<F>> {
        self.parts.faces.get(face.index())
    }
    /// An edge by its checked local index.
    #[must_use]
    pub fn edge(&self, edge: EdgeId) -> Option<&Edge<E>> {
        self.parts.edges.get(edge.index())
    }
    /// A loop by its checked local index.
    #[must_use]
    pub fn boundary_loop(&self, loop_id: LoopId) -> Option<&BoundaryLoop> {
        self.parts.loops.get(loop_id.index())
    }

    /// Evaluates supporting geometry and applies the face's normal sense.
    ///
    /// Uses surface parameter units. Neither the face domain nor its trim is
    /// classified here. Returns `None` for an unknown face or non-finite result.
    #[must_use]
    pub fn evaluate_support(&self, face: FaceId, uv: [f64; 2]) -> Option<SurfaceEvaluation> {
        let face = self.face(face)?;
        let mut result = face.surface.evaluate(uv)?;
        result.normal = result.normal.map(|n| face.sense.sign() * n);
        Some(result)
    }

    /// Evaluates a bounded edge by a fraction, preserving authored endpoints.
    /// Returns `None` for an unknown edge or a fraction outside `[0,1]`.
    #[must_use]
    pub fn edge_point(&self, edge: EdgeId, fraction: f64) -> Option<[f64; 3]> {
        let edge = self.edge(edge)?;
        let t = edge.range.at(fraction)?;
        if fraction == 0.0 {
            return self.parts.vertices.get(edge.start.index()).copied();
        }
        if fraction == 1.0 {
            return self.parts.vertices.get(edge.end.index()).copied();
        }
        Some(edge.curve.evaluate(t)?.point)
    }

    /// Exact length of a line or circular edge, in model units.
    #[must_use]
    pub fn edge_length(&self, edge: EdgeId) -> Option<f64> {
        let edge = self.edge(edge)?;
        let speed = match &edge.curve {
            Curve::Line(line) => length(line.direction()),
            Curve::Circle(circle) => circle.radius(),
        };
        let result = speed * edge.range.span();
        result.is_finite().then_some(result)
    }

    /// A right-handed frame on a planar supporting face at `uv`, +Z in its sense.
    ///
    /// The origin is not required to be inside the trims. Curved and unknown
    /// faces, non-finite parameters and overflowed positions return `None`.
    #[must_use]
    pub fn planar_frame(&self, face: FaceId, uv: [f64; 2]) -> Option<Placement3> {
        let face = self.face(face)?;
        let Surface::Plane(plane) = &face.surface else {
            return None;
        };
        let origin = face.surface.evaluate(uv)?.point;
        let frame = plane.frame();
        let col = |i: usize| frame.rows.map(|r| r[i]);
        Some(Placement3::from_axes(
            col(0),
            col(1).map(|v| v * face.sense.sign()),
            col(2).map(|v| v * face.sense.sign()),
            origin,
        ))
    }

    /// Whether every used edge occurs once in each direction in closed loops.
    ///
    /// Detached table entries are ignored. This tests oriented incidence;
    /// it does not prove geometric non-intersection or positive enclosed volume.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        if self.parts.faces.is_empty() {
            return false;
        }
        let mut uses = alloc::vec![[0_usize;2];self.parts.edges.len()];
        for face in &self.parts.faces {
            if face.loops.is_empty() {
                return false;
            }
            for &loop_id in &face.loops {
                let Some(loop_) = self.boundary_loop(loop_id) else {
                    return false;
                };
                if loop_.coedges.is_empty() {
                    return false;
                }
                for (i, &coedge) in loop_.coedges.iter().enumerate() {
                    let next = loop_.coedges[(i + 1) % loop_.coedges.len()];
                    if self.coedge_vertices(coedge).map(|v| v[1])
                        != self.coedge_vertices(next).map(|v| v[0])
                    {
                        return false;
                    }
                    let Some(count) = uses.get_mut(coedge.edge.index()) else {
                        return false;
                    };
                    count[usize::from(coedge.reversed)] += 1;
                }
            }
        }
        uses.iter().all(|v| *v == [0, 0] || *v == [1, 1])
    }

    /// Ordered endpoint vertex identities of a boundary occurrence.
    #[must_use]
    pub fn coedge_vertices(&self, coedge: Coedge) -> Option<[VertexId; 2]> {
        let edge = self.edge(coedge.edge)?;
        Some(if coedge.reversed {
            [edge.end, edge.start]
        } else {
            [edge.start, edge.end]
        })
    }

    /// Validates incidence and conservative analytic support bounds without mutation.
    ///
    /// Circular support bounds may cover the full supporting circle, giving
    /// a conservative refusal for a short arc on a nearly coincident support.
    /// Detached entries are permitted so edits can preserve existing IDs.
    ///
    /// # Errors
    /// Returns the first [`BodyError`]. No geometry is moved or healed.
    pub fn validate(&self, policy: &ValidationPolicy) -> Result<ValidationReport, BodyError> {
        if !policy.max_gap.is_finite() || policy.max_gap < 0.0 {
            return Err(BodyError::InvalidTolerance {
                max_gap: policy.max_gap,
            });
        }
        for (table, count) in [
            ("vertices", self.parts.vertices.len()),
            ("edges", self.parts.edges.len()),
            ("loops", self.parts.loops.len()),
            ("faces", self.parts.faces.len()),
        ] {
            if u32::try_from(count).is_err() {
                return Err(BodyError::TooLarge { table, count });
            }
        }
        let mut report = ValidationReport {
            max_gap: 0.0,
            coedges: 0,
        };
        for (i, p) in self.parts.vertices.iter().enumerate() {
            if !p.iter().all(|v| v.is_finite()) {
                return Err(BodyError::NonFiniteVertex {
                    vertex: VertexId(crate::usize_to_u32(i)),
                });
            }
        }
        for (i, edge) in self.parts.edges.iter().enumerate() {
            let id = EdgeId(crate::usize_to_u32(i));
            if edge
                .curve
                .period()
                .is_some_and(|period| edge.range.span() > period)
            {
                return Err(BodyError::RepeatedCurve { edge: id });
            }
            for (vertex, t) in [
                (edge.start, edge.range.bounds()[0]),
                (edge.end, edge.range.bounds()[1]),
            ] {
                let point = self
                    .parts
                    .vertices
                    .get(vertex.index())
                    .ok_or(BodyError::MissingVertex { edge: id, vertex })?;
                let curve = edge
                    .curve
                    .evaluate(t)
                    .ok_or(BodyError::NonFiniteCurve { edge: id })?;
                let gap = length(sub(*point, curve.point));
                if !gap.is_finite() || gap > policy.max_gap {
                    return Err(BodyError::EndpointGap {
                        edge: id,
                        vertex,
                        gap,
                        allowed: policy.max_gap,
                    });
                }
                report.max_gap = report.max_gap.max(gap);
            }
        }
        for (i, loop_) in self.parts.loops.iter().enumerate() {
            let loop_id = LoopId(crate::usize_to_u32(i));
            if loop_.coedges.is_empty() {
                return Err(BodyError::EmptyLoop { loop_id });
            }
            for (index, &coedge) in loop_.coedges.iter().enumerate() {
                let edge = self.edge(coedge.edge).ok_or(BodyError::MissingEdge {
                    loop_id,
                    coedge: index,
                    edge: coedge.edge,
                })?;
                let next = loop_.coedges[(index + 1) % loop_.coedges.len()];
                let next_edge = self.edge(next.edge).ok_or(BodyError::MissingEdge {
                    loop_id,
                    coedge: (index + 1) % loop_.coedges.len(),
                    edge: next.edge,
                })?;
                let end = if coedge.reversed {
                    edge.start
                } else {
                    edge.end
                };
                let start = if next.reversed {
                    next_edge.end
                } else {
                    next_edge.start
                };
                if end != start {
                    return Err(BodyError::DisconnectedLoop {
                        loop_id,
                        coedge: index,
                        end,
                        next_start: start,
                    });
                }
            }
        }
        for (i, face) in self.parts.faces.iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            if face.loops.is_empty() {
                return Err(BodyError::EmptyFace { face: id });
            }
            for &loop_id in &face.loops {
                let loop_ = self
                    .boundary_loop(loop_id)
                    .ok_or(BodyError::MissingLoop { face: id, loop_id })?;
                for &coedge in &loop_.coedges {
                    let edge = &self.parts.edges[coedge.edge.index()];
                    let bound = support_gap(&edge.curve, edge.range, &face.surface);
                    if !bound.is_finite() || bound > policy.max_gap {
                        return Err(BodyError::SurfaceGap {
                            face: id,
                            edge: coedge.edge,
                            bound,
                            allowed: policy.max_gap,
                        });
                    }
                    report.max_gap = report.max_gap.max(bound);
                    report.coedges += 1;
                }
            }
        }
        Ok(report)
    }
}

/// A boundary-table, incidence or geometric-support violation.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyError {
    /// A finite nonnegative agreement tolerance was required.
    InvalidTolerance {
        /// Supplied maximum gap.
        max_gap: f64,
    },
    /// A table exceeds the local ID range.
    TooLarge {
        /// Table name.
        table: &'static str,
        /// Requested entries.
        count: usize,
    },
    /// A vertex position contains a non-finite coordinate.
    NonFiniteVertex {
        /// Offending vertex.
        vertex: VertexId,
    },
    /// An edge endpoint is outside the vertex table.
    MissingVertex {
        /// Owning edge.
        edge: EdgeId,
        /// Missing vertex.
        vertex: VertexId,
    },
    /// An edge's parameterization overflows at an endpoint.
    NonFiniteCurve {
        /// Offending edge.
        edge: EdgeId,
    },
    /// A periodic edge spans more than one period.
    RepeatedCurve {
        /// Offending edge.
        edge: EdgeId,
    },
    /// Vertex and curve endpoint do not agree within the requested tolerance.
    EndpointGap {
        /// Owning edge.
        edge: EdgeId,
        /// Endpoint vertex.
        vertex: VertexId,
        /// Measured distance.
        gap: f64,
        /// Allowed distance.
        allowed: f64,
    },
    /// A face has no outer boundary loop.
    EmptyFace {
        /// Offending face.
        face: FaceId,
    },
    /// A face names a loop outside the loop table.
    MissingLoop {
        /// Owning face.
        face: FaceId,
        /// Missing loop.
        loop_id: LoopId,
    },
    /// A loop has no edge occurrences.
    EmptyLoop {
        /// Offending loop.
        loop_id: LoopId,
    },
    /// A loop names an edge outside the edge table.
    MissingEdge {
        /// Owning loop.
        loop_id: LoopId,
        /// Occurrence index.
        coedge: usize,
        /// Missing edge.
        edge: EdgeId,
    },
    /// Consecutive occurrences do not share the same endpoint identity.
    DisconnectedLoop {
        /// Owning loop.
        loop_id: LoopId,
        /// First occurrence index.
        coedge: usize,
        /// Its end.
        end: VertexId,
        /// The next occurrence's start.
        next_start: VertexId,
    },
    /// A curve is not supported by its incident face within the requested gap.
    SurfaceGap {
        /// Incident face.
        face: FaceId,
        /// Offending edge.
        edge: EdgeId,
        /// Conservative distance bound.
        bound: f64,
        /// Allowed distance.
        allowed: f64,
    },
}
impl core::fmt::Display for BodyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "boundary validation: {self:?}")
    }
}
impl core::error::Error for BodyError {}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    core::array::from_fn(|i| a[i] - b[i])
}
fn length(v: [f64; 3]) -> f64 {
    libm::hypot(libm::hypot(v[0], v[1]), v[2])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn coordinates(frame: Placement3, p: [f64; 3]) -> [f64; 3] {
    let delta = sub(p, frame.rows.map(|r| r[3]));
    core::array::from_fn(|i| dot(delta, frame.rows.map(|r| r[i])))
}
fn support_gap(curve: &Curve, range: ParameterRange, surface: &Surface) -> f64 {
    let frame = surface.frame();
    match curve {
        Curve::Line(_) => {
            let Some(a) = curve.evaluate(range.bounds()[0]) else {
                return f64::INFINITY;
            };
            let Some(b) = curve.evaluate(range.bounds()[1]) else {
                return f64::INFINITY;
            };
            let (a, b) = (coordinates(frame, a.point), coordinates(frame, b.point));
            match surface {
                Surface::Plane(_) => a[2].abs().max(b[2].abs()),
                Surface::Cylinder(cylinder) => {
                    let dx = b[0] - a[0];
                    let dy = b[1] - a[1];
                    let delta = libm::hypot(dx, dy);
                    let t = if delta == 0.0 {
                        0.0
                    } else {
                        (-(a[0] * (dx / delta) + a[1] * (dy / delta)) / delta).clamp(0.0, 1.0)
                    };
                    let radial = |x: f64, y: f64| (libm::hypot(x, y) - cylinder.radius()).abs();
                    radial(a[0], a[1])
                        .max(radial(b[0], b[1]))
                        .max(radial(a[0] + t * dx, a[1] + t * dy))
                }
            }
        }
        Curve::Circle(circle) => {
            let cf = circle.frame();
            let center = coordinates(frame, cf.rows.map(|r| r[3]));
            let axis = |i: usize| cf.rows.map(|r| r[i]);
            let normal = frame.rows.map(|r| r[2]);
            let tilt = libm::hypot(dot(axis(0), normal), dot(axis(1), normal));
            match surface {
                Surface::Plane(_) => center[2].abs() + circle.radius() * tilt,
                Surface::Cylinder(cylinder) => {
                    libm::hypot(center[0], center[1])
                        + (circle.radius() - cylinder.radius()).abs()
                        + circle.radius() * tilt
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    const POLICY: ValidationPolicy = ValidationPolicy { max_gap: 1e-12 };

    fn cylinder_parts() -> BodyParts {
        let circle = |z| Curve::circle(Placement3::translate(0.0, 0.0, z), 2.0).unwrap();
        let full = ParameterRange::new(0.0, core::f64::consts::TAU).unwrap();
        let coedge = |edge, reversed| Coedge {
            edge: EdgeId(edge),
            reversed,
        };
        BodyParts {
            vertices: vec![[2.0, 0.0, 0.0], [2.0, 0.0, 3.0]],
            edges: vec![
                Edge {
                    curve: circle(0.0),
                    range: full,
                    start: VertexId(0),
                    end: VertexId(0),
                    role: (),
                },
                Edge {
                    curve: circle(3.0),
                    range: full,
                    start: VertexId(1),
                    end: VertexId(1),
                    role: (),
                },
                Edge {
                    curve: Curve::line([2.0, 0.0, 0.0], [0.0, 0.0, 3.0]).unwrap(),
                    range: ParameterRange::UNIT,
                    start: VertexId(0),
                    end: VertexId(1),
                    role: (),
                },
            ],
            loops: vec![
                BoundaryLoop {
                    coedges: vec![
                        coedge(0, false),
                        coedge(2, false),
                        coedge(1, true),
                        coedge(2, true),
                    ],
                },
                BoundaryLoop {
                    coedges: vec![coedge(0, true)],
                },
                BoundaryLoop {
                    coedges: vec![coedge(1, false)],
                },
            ],
            faces: vec![
                Face {
                    surface: Surface::cylinder(Placement3::IDENTITY, 2.0).unwrap(),
                    domain: ParameterDomain {
                        u: full,
                        v: ParameterRange::new(0.0, 3.0).unwrap(),
                    },
                    sense: Sense::Forward,
                    loops: vec![LoopId(0)],
                    role: (),
                },
                Face {
                    surface: Surface::plane(Placement3::IDENTITY).unwrap(),
                    domain: ParameterDomain {
                        u: ParameterRange::new(-2.0, 2.0).unwrap(),
                        v: ParameterRange::new(-2.0, 2.0).unwrap(),
                    },
                    sense: Sense::Reverse,
                    loops: vec![LoopId(1)],
                    role: (),
                },
                Face {
                    surface: Surface::plane(Placement3::translate(0.0, 0.0, 3.0)).unwrap(),
                    domain: ParameterDomain {
                        u: ParameterRange::new(-2.0, 2.0).unwrap(),
                        v: ParameterRange::new(-2.0, 2.0).unwrap(),
                    },
                    sense: Sense::Forward,
                    loops: vec![LoopId(2)],
                    role: (),
                },
            ],
        }
    }
    #[test]
    fn one_periodic_face_reuses_its_seam_edge_and_closed_circle_endpoints() {
        let body = Body::try_from_parts(cylinder_parts(), &POLICY).unwrap();
        assert!(body.is_closed());
        assert_eq!(
            (
                body.vertices().len(),
                body.edges().len(),
                body.faces().len()
            ),
            (2, 3, 3)
        );
        let wall = body
            .evaluate_support(FaceId(0), [core::f64::consts::FRAC_PI_2, 1.5])
            .unwrap();
        assert!(wall.point[0].abs() < 1e-12);
        assert_eq!([wall.point[1], wall.point[2]], [2.0, 1.5]);
        assert_eq!(
            body.evaluate_support(FaceId(1), [0.0, 0.0]).unwrap().normal,
            [0.0, 0.0, -1.0]
        );
        assert_eq!(
            body.edge_point(EdgeId(0), 0.0),
            body.edge_point(EdgeId(0), 1.0)
        );
        assert!((body.edge_length(EdgeId(0)).unwrap() - 4.0 * core::f64::consts::PI).abs() < 1e-12);
        assert_eq!(body.edge_length(EdgeId(2)), Some(3.0));
    }

    #[test]
    fn identities_and_geometric_agreement_are_checked_independently() {
        let mut parts = cylinder_parts();
        parts.vertices.push(parts.vertices[0]);
        parts.edges[0].end = VertexId(2);
        assert!(matches!(
            Body::try_from_parts(parts, &POLICY),
            Err(BodyError::DisconnectedLoop { .. })
        ));
        let mut parts = cylinder_parts();
        parts.vertices[0][0] += 0.01;
        assert!(matches!(
            Body::try_from_parts(parts, &POLICY),
            Err(BodyError::EndpointGap {
                edge: EdgeId(0),
                ..
            })
        ));
        let mut parts = cylinder_parts();
        parts.faces[0].surface = Surface::cylinder(Placement3::IDENTITY, 3.0).unwrap();
        assert!(matches!(
            Body::try_from_parts(parts, &POLICY),
            Err(BodyError::SurfaceGap {
                face: FaceId(0),
                ..
            })
        ));
    }

    #[test]
    fn endpoints_on_a_cylinder_do_not_hide_an_interior_chord_gap() {
        let mut parts = cylinder_parts();
        parts.vertices = vec![[2.0, 0.0, 0.0], [-2.0, 0.0, 0.0]];
        parts.edges = vec![Edge {
            curve: Curve::line([2.0, 0.0, 0.0], [-4.0, 0.0, 0.0]).unwrap(),
            range: ParameterRange::UNIT,
            start: VertexId(0),
            end: VertexId(1),
            role: (),
        }];
        parts.loops = vec![BoundaryLoop {
            coedges: vec![
                Coedge {
                    edge: EdgeId(0),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(0),
                    reversed: true,
                },
            ],
        }];
        parts.faces.truncate(1);
        assert!(matches!(
            Body::try_from_parts(parts, &POLICY),
            Err(BodyError::SurfaceGap { bound: 2.0, .. })
        ));
    }
}
