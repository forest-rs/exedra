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

mod cylindrical;
mod planar;
pub(crate) mod trims;
pub use cylindrical::{CylindricalBand, CylindricalBandError};
pub(crate) use trims::planar_nurbs;
pub use trims::{LoopTrims, TrimCurve};

use crate::geometry::{Curve, ParameterDomain, ParameterRange, Surface, SurfaceEvaluation};
use alloc::vec::Vec;
use exedra_curves::enclosure::{SourceRange, line_enclosure};
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

/// Geometry of a topological edge. A collapsed parameter boundary has one
/// authoritative vertex and no ordinary supporting curve or curve interval.
#[derive(Clone, Debug, PartialEq)]
pub enum EdgeGeometry {
    /// A regular bounded curve; equal endpoint IDs represent a closed edge.
    Curve {
        /// Untrimmed 3D supporting curve.
        curve: Curve,
        /// Increasing curve-parameter interval, independent of face traversal.
        range: ParameterRange,
    },
    /// A surface-coordinate boundary collapsed to the edge's sole vertex.
    /// Body validation permits a used collapsed edge only at a certified surface
    /// pole on a face-domain boundary, with explicit collapsed incidence.
    Collapsed,
}

/// One topological edge, with authoritative endpoint vertex identities.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge<E = ()> {
    /// Regular curve or explicit collapsed parameter boundary.
    pub geometry: EdgeGeometry,
    /// First authoritative endpoint; collapsed edges require the same end ID.
    pub start: VertexId,
    /// Second authoritative endpoint.
    pub end: VertexId,
    /// Caller metadata; independent of the edge's geometric definition.
    pub role: E,
}
impl<E> Edge<E> {
    /// Constructs a regular bounded edge; body validation checks its references
    /// and agreement. Equal endpoints remain an ordinary closed curve edge.
    pub fn new(
        curve: Curve,
        range: ParameterRange,
        start: VertexId,
        end: VertexId,
        role: E,
    ) -> Self {
        Self {
            geometry: EdgeGeometry::Curve { curve, range },
            start,
            end,
            role,
        }
    }
    /// Constructs a collapsed boundary from its sole authoritative vertex.
    /// No duplicate point, zero-length curve or fictitious interval is stored.
    /// Body validation still certifies the supported pole use and incidence.
    pub fn collapsed(vertex: VertexId, role: E) -> Self {
        Self {
            geometry: EdgeGeometry::Collapsed,
            start: vertex,
            end: vertex,
            role,
        }
    }
    /// Borrows a regular supporting curve and its range; collapsed boundaries
    /// explicitly return `None`.
    #[must_use]
    pub fn regular(&self) -> Option<(&Curve, ParameterRange)> {
        match &self.geometry {
            EdgeGeometry::Curve { curve, range } => Some((curve, *range)),
            EdgeGeometry::Collapsed => None,
        }
    }
    /// Mutably borrows regular geometry in an owned edge record. Collapsed
    /// boundaries return `None`. Revalidate edited [`BodyParts`] before use.
    #[must_use]
    pub fn regular_mut(&mut self) -> Option<(&mut Curve, &mut ParameterRange)> {
        match &mut self.geometry {
            EdgeGeometry::Curve { curve, range } => Some((curve, range)),
            EdgeGeometry::Collapsed => None,
        }
    }
    /// Whether this is an explicit collapsed parameter boundary, independently
    /// of closed regular curves whose endpoint IDs also happen to be equal.
    #[must_use]
    pub const fn is_collapsed(&self) -> bool {
        matches!(self.geometry, EdgeGeometry::Collapsed)
    }
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
    /// Explicit face-local parameter correspondence, indexed by boundary loop.
    /// Missing records retain support-only validation for legacy constructors.
    pub trims: Vec<LoopTrims>,
    /// Authoritative body-local vertex positions.
    pub vertices: Vec<[f64; 3]>,
    /// Shared native edges, including collapsed pole boundaries.
    pub edges: Vec<Edge<E>>,
    /// Boundary occurrence chains.
    pub loops: Vec<BoundaryLoop>,
    /// Trimmed surface uses.
    pub faces: Vec<Face<F>>,
}
impl<F, E> Default for BodyParts<F, E> {
    fn default() -> Self {
        Self {
            trims: Vec::new(),
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
    /// Maximum endpoint, curve/support and explicit trim-correspondence gap in model units.
    pub max_gap: f64,
}

/// Evidence about incidence and geometric support checks.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ValidationReport {
    /// Largest endpoint agreement evidence or conservative curve/support bound.
    /// Line endpoints use an outward original-coefficient distance upper bound;
    /// other endpoints use evaluated f64 distance. A bound may overestimate the
    /// actual geometric disagreement.
    pub max_gap: f64,
    /// Number of face-edge occurrences checked for geometric support.
    pub coedges: usize,
    /// Number of occurrences with certified explicit parameter correspondence.
    /// Support-only legacy loops do not contribute to this count.
    pub trimmed_coedges: usize,
    /// Largest conservative parameter-curve to shared-edge correspondence bound.
    pub max_trim_gap: f64,
}

/// A boundary representation independent of its construction history.
#[derive(Clone, Debug, PartialEq)]
pub struct Body<F = (), E = ()> {
    pub(crate) parts: BodyParts<F, E>,
}

impl<F, E> Default for Body<F, E> {
    fn default() -> Self {
        Self {
            parts: BodyParts::default(),
        }
    }
}

impl<F, E> Body<F, E> {
    /// Checks references, vertex-identity closure, analytic support agreement,
    /// and every supplied continuous face-local parameter correspondence.
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
    /// Changes caller roles without cloning or revalidating geometry.
    #[must_use]
    pub fn map_roles<F2, E2>(
        self,
        mut face: impl FnMut(F) -> F2,
        mut edge: impl FnMut(E) -> E2,
    ) -> Body<F2, E2> {
        let parts = self.parts;
        Body {
            parts: BodyParts {
                vertices: parts.vertices,
                edges: parts
                    .edges
                    .into_iter()
                    .map(|e| Edge {
                        geometry: e.geometry,
                        start: e.start,
                        end: e.end,
                        role: edge(e.role),
                    })
                    .collect(),
                loops: parts.loops,
                faces: parts
                    .faces
                    .into_iter()
                    .map(|f| Face {
                        surface: f.surface,
                        domain: f.domain,
                        sense: f.sense,
                        loops: f.loops,
                        role: face(f.role),
                    })
                    .collect(),
                trims: parts.trims,
            },
        }
    }
    /// Explicit parameter curves in deterministic record order.
    #[must_use]
    pub fn trims(&self) -> &[LoopTrims] {
        &self.parts.trims
    }
    /// A loop occurrence's parameter curve in canonical edge direction.
    #[must_use]
    pub fn trim_curve(&self, loop_id: LoopId, coedge: usize) -> Option<&TrimCurve> {
        self.parts
            .trims
            .iter()
            .find(|t| t.loop_id == loop_id)?
            .curves
            .get(coedge)
    }
    /// Whether every boundary loop of a face has explicit parameter curves.
    #[must_use]
    pub fn has_explicit_trims(&self, face: FaceId) -> bool {
        self.face(face)
            .is_some_and(|f| f.loops.iter().all(|id| self.trim_curve(*id, 0).is_some()))
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

    /// One-sided supporting normal at a coordinate pole, applying face sense.
    ///
    /// Uses [`Surface::pole_limit_normal`]; cone U selects its generator limit.
    /// This does not classify the face domain/trim or assign a unique apex
    /// normal. Returns `None` for an unknown face, non-pole or numeric refusal.
    #[must_use]
    pub fn pole_limit_normal(&self, face: FaceId, uv: [f64; 2]) -> Option<[f64; 3]> {
        let face = self.face(face)?;
        Some(
            face.surface
                .pole_limit_normal(uv)?
                .map(|n| face.sense.sign() * n),
        )
    }

    /// Queries an edge by normalized traversal fraction, preserving authored
    /// endpoints. A collapsed boundary returns its sole authoritative point;
    /// it has no ordinary supporting curve or curve parameter.
    /// Returns `None` for an unknown edge or a fraction outside `[0,1]`.
    #[must_use]
    pub fn edge_point(&self, edge: EdgeId, fraction: f64) -> Option<[f64; 3]> {
        let edge = self.edge(edge)?;
        if !(0.0..=1.0).contains(&fraction) {
            return None;
        }
        let Some((curve, range)) = edge.regular() else {
            return self.parts.vertices.get(edge.start.index()).copied();
        };
        let t = range.at(fraction)?;
        if fraction == 0.0 {
            return self.parts.vertices.get(edge.start.index()).copied();
        }
        if fraction == 1.0 {
            return self.parts.vertices.get(edge.end.index()).copied();
        }
        Some(curve.evaluate(t)?.point)
    }

    /// Exact length of a line or circular edge, in model units; collapsed
    /// boundaries have zero length. Ellipse lengths require an elliptic
    /// integral and NURBS lengths a quadrature, neither exact, so both
    /// return `None`.
    #[must_use]
    pub fn edge_length(&self, edge: EdgeId) -> Option<f64> {
        let edge = self.edge(edge)?;
        let Some((curve, range)) = edge.regular() else {
            return Some(0.0);
        };
        let speed = match curve {
            Curve::Line(line) => length(line.direction()),
            Curve::Circle(circle) => circle.radius(),
            Curve::Ellipse(_) | Curve::Nurbs(_) => return None,
        };
        let result = speed * range.span();
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

    /// Whether regular used edges occur once in each direction in closed loops.
    /// A certified collapsed parameter boundary occurs once; it represents no
    /// physical 3D edge to pair. Detached table entries are ignored. This tests oriented incidence;
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
        uses.iter().zip(self.edges()).all(|(uses, edge)| {
            *uses == [0, 0]
                || if edge.is_collapsed() {
                    uses[0] + uses[1] == 1
                } else {
                    *uses == [1, 1]
                }
        })
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
            trimmed_coedges: 0,
            max_trim_gap: 0.0,
        };
        for (i, p) in self.parts.vertices.iter().enumerate() {
            if !p.iter().all(|v| v.is_finite()) {
                return Err(BodyError::NonFiniteVertex {
                    vertex: VertexId(crate::usize_to_u32(i)),
                });
            }
        }
        let mut collapsed_uses: Option<Vec<usize>> = None;
        for (i, edge) in self.parts.edges.iter().enumerate() {
            let id = EdgeId(crate::usize_to_u32(i));
            let Some((curve, range)) = edge.regular() else {
                collapsed_uses.get_or_insert_with(|| alloc::vec![0;self.edges().len()]);
                if edge.start != edge.end {
                    return Err(BodyError::CollapsedEndpoints { edge: id });
                }
                if self.parts.vertices.get(edge.start.index()).is_none() {
                    return Err(BodyError::MissingVertex {
                        edge: id,
                        vertex: edge.start,
                    });
                }
                continue;
            };
            if curve.period().is_some_and(|period| range.span() > period) {
                return Err(BodyError::RepeatedCurve { edge: id });
            }
            for (vertex, t, fraction) in [
                (edge.start, range.bounds()[0], 0.0),
                (edge.end, range.bounds()[1], 1.0),
            ] {
                let point = self
                    .parts
                    .vertices
                    .get(vertex.index())
                    .ok_or(BodyError::MissingVertex { edge: id, vertex })?;
                let evaluated = curve
                    .evaluate(t)
                    .ok_or(BodyError::NonFiniteCurve { edge: id })?;
                let gap = match curve {
                    Curve::Line(line)
                        if exact_line_endpoint(
                            line.origin(),
                            line.direction(),
                            t,
                            evaluated.point,
                            *point,
                        ) =>
                    {
                        0.0
                    }
                    Curve::Line(line) => line_enclosure(
                        line.origin(),
                        line.direction(),
                        SourceRange {
                            parameters: range.bounds(),
                            reversed: false,
                        },
                        [fraction, fraction],
                    )
                    .and_then(|endpoint| endpoint.distance_bounds(*point))
                    .map_or(f64::INFINITY, |distance| distance[1]),
                    _ => length(sub(*point, evaluated.point)),
                };
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
            if let Surface::Sphere(_) = &face.surface
                && face
                    .domain
                    .v
                    .bounds()
                    .iter()
                    .any(|v| v.abs() > core::f64::consts::FRAC_PI_2)
            {
                return Err(BodyError::NonRegularSurface { face: id });
            }
            let cone_pole = if collapsed_uses.is_some() && matches!(face.surface, Surface::Cone(_))
            {
                face.loops
                    .iter()
                    .filter_map(|id| self.boundary_loop(*id))
                    .flat_map(|ring| &ring.coedges)
                    .find_map(|coedge| {
                        self.edge(coedge.edge)
                            .filter(|e| e.is_collapsed())
                            .map(|e| e.start)
                    })
            } else {
                None
            };
            if let Surface::Cone(cone) = &face.surface
                && face.domain.v.bounds().iter().any(|&v| {
                    cone.radius_at(v).is_none()
                        && !(Some(v) == cone.apex_parameter()
                            && cone_pole.is_some_and(|vertex| {
                                crate::geometry::cone_pole_gap(
                                    cone,
                                    v,
                                    self.vertices()[vertex.index()],
                                )
                                .is_some_and(|gap| gap <= policy.max_gap)
                            }))
                })
            {
                return Err(BodyError::NonRegularSurface { face: id });
            }
            if matches!(face.surface, Surface::Ruled(_))
                && face
                    .domain
                    .u
                    .bounds()
                    .into_iter()
                    .chain(face.domain.v.bounds())
                    .any(|t| !ParameterRange::UNIT.contains(t))
            {
                return Err(BodyError::NonRegularSurface { face: id });
            }
            for &loop_id in &face.loops {
                let loop_ = self
                    .boundary_loop(loop_id)
                    .ok_or(BodyError::MissingLoop { face: id, loop_id })?;
                for &coedge in &loop_.coedges {
                    let edge = &self.parts.edges[coedge.edge.index()];
                    let bound = if let Some((curve, range)) = edge.regular() {
                        if let (Surface::Cone(cone), Curve::Line(line)) = (&face.surface, curve) {
                            let poles =
                                [Some(edge.start) == cone_pole, Some(edge.end) == cone_pole];
                            if poles.into_iter().any(|p| p) {
                                crate::geometry::cone_generator_gap(cone, line, range, poles)
                            } else {
                                support_gap(curve, range, &face.surface)
                            }
                        } else {
                            support_gap(curve, range, &face.surface)
                        }
                    } else {
                        let uses = &mut collapsed_uses.as_mut().expect("collapsed edge surveyed")
                            [coedge.edge.index()];
                        *uses += 1;
                        if *uses > 1 {
                            return Err(BodyError::CollapsedIncidence {
                                edge: coedge.edge,
                                uses: *uses,
                            });
                        }
                        let point = self.parts.vertices[edge.start.index()];
                        collapsed_gap(&face.surface, face.domain, point).ok_or(
                            BodyError::UnsupportedCollapsedBoundary {
                                face: id,
                                edge: coedge.edge,
                            },
                        )?
                    };
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
        self.validate_trims(policy, &mut report)?;
        Ok(report)
    }
}

fn collapsed_gap(surface: &Surface, domain: ParameterDomain, point: [f64; 3]) -> Option<f64> {
    if domain.u.span() > core::f64::consts::TAU {
        return None;
    }
    domain
        .v
        .bounds()
        .into_iter()
        .filter_map(|v| match surface {
            Surface::Sphere(s) => crate::geometry::sphere_pole_gap(s, v, point),
            Surface::Cone(c) => crate::geometry::cone_pole_gap(c, v, point),
            _ => None,
        })
        .reduce(f64::min)
}

/// A boundary-table, incidence or geometric-support violation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BodyError {
    /// An explicit trim table does not match its unique owning loop.
    TrimStructure {
        /// Offending boundary loop.
        loop_id: LoopId,
        /// Missing owner, repeated record/owner, or curve count mismatch.
        reason: &'static str,
    },
    /// A parameter curve is invalid or leaves the finite face domain.
    TrimDomain {
        /// Owning face.
        face: FaceId,
        /// Offending boundary loop.
        loop_id: LoopId,
        /// Occurrence index.
        coedge: usize,
    },
    /// Continuous parameter-to-3D correspondence could not be certified.
    TrimCorrespondence {
        /// Owning face.
        face: FaceId,
        /// Offending boundary loop.
        loop_id: LoopId,
        /// Occurrence index.
        coedge: usize,
        /// Conservative gap bound, infinity for unsupported compositions.
        bound: f64,
        /// Requested bound.
        allowed: f64,
    },
    /// Adjacent parameter curves do not close in the same face chart.
    TrimClosure {
        /// Owning face.
        face: FaceId,
        /// Offending boundary loop.
        loop_id: LoopId,
        /// First occurrence index.
        coedge: usize,
    },
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
    /// A collapsed boundary names two distinct vertex identities.
    CollapsedEndpoints {
        /// Offending edge.
        edge: EdgeId,
    },
    /// A collapsed edge is not a certified permitted surface-pole boundary.
    UnsupportedCollapsedBoundary {
        /// Owning face.
        face: FaceId,
        /// Offending collapsed edge.
        edge: EdgeId,
    },
    /// A used collapsed edge must occur on exactly one parameter boundary.
    CollapsedIncidence {
        /// Offending edge.
        edge: EdgeId,
        /// Number of occurrences.
        uses: usize,
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
        /// Endpoint agreement evidence in model units: a conservative original-
        /// coefficient distance upper bound for lines, evaluated f64 distance
        /// for other curves. Exceeding the policy can be a conservative refusal;
        /// this value does not assert a matching geometric distance lower bound.
        gap: f64,
        /// Allowed distance.
        allowed: f64,
    },
    /// A face domain meets a singularity or leaves the certified regular region.
    NonRegularSurface {
        /// Offending face.
        face: FaceId,
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
// Preserve exact endpoint identities without treating a rounded evaluation as
// proof. These products are exact in IEEE binary arithmetic. Knuth's TwoSum
// residual then proves that adding the finite original coefficient was exact
// too. Other products use the original-source outward enclosure above.
fn exact_line_endpoint(
    origin: [f64; 3],
    direction: [f64; 3],
    parameter: f64,
    evaluated: [f64; 3],
    vertex: [f64; 3],
) -> bool {
    for k in 0..3 {
        if evaluated[k] != vertex[k] {
            return false;
        }
        let product = if parameter == 0.0 || direction[k] == 0.0 {
            0.0
        } else if parameter == 1.0 {
            direction[k]
        } else if parameter == -1.0 {
            -direction[k]
        } else if direction[k] == 1.0 {
            parameter
        } else if direction[k] == -1.0 {
            -parameter
        } else {
            return false;
        };
        let partial = evaluated[k] - origin[k];
        let tail = (origin[k] - (evaluated[k] - partial)) + (product - partial);
        if tail != 0.0 {
            return false;
        }
    }
    true
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn coordinates(frame: Placement3, p: [f64; 3]) -> [f64; 3] {
    let delta = sub(p, frame.rows.map(|r| r[3]));
    core::array::from_fn(|i| dot(delta, frame.rows.map(|r| r[i])))
}
/// The unit normal of the plane a frame's U and V axes span, on which its
/// points are evaluated: a checked frame's stored Z is only nearly normal
/// to it, which far from the origin is a measurable tilt.
fn spanned_normal(frame: Placement3) -> Option<[f64; 3]> {
    exedra_math::normalize(exedra_math::cross(
        frame.rows.map(|r| r[0]),
        frame.rows.map(|r| r[1]),
    ))
}
/// Distance from the plane a frame's U and V axes span through its origin;
/// infinite where they span none.
fn plane_distance(frame: Placement3) -> impl Fn([f64; 3]) -> f64 {
    let normal = spanned_normal(frame);
    let origin = frame.rows.map(|r| r[3]);
    move |p| normal.map_or(f64::INFINITY, |n| dot(sub(p, origin), n).abs())
}
fn support_gap(curve: &Curve, range: ParameterRange, surface: &Surface) -> f64 {
    if let Surface::Ruled(ruled) = surface {
        // Full-curve comparison against the patch's exact four boundaries.
        // This is sufficient, not complete for arbitrary interior trims.
        // A stored source span is already checked. Identical defining geometry
        // over the same range has exactly the same locus in either direction;
        // no alignment-roundoff allowance or temporary span is needed.
        if ruled
            .curves()
            .iter()
            .any(|span| span.curve() == curve && span.range() == range)
        {
            return 0.0;
        }
        let mut gap = f64::INFINITY;
        for alignment in [false, true]
            .into_iter()
            .filter_map(|reverse| {
                crate::geometry::CurveSpan::new(curve.clone(), range, reverse).ok()
            })
            .flat_map(|actual| {
                (0..4)
                    .filter_map(|side| ruled.boundary(side))
                    .map(move |ideal| ideal.alignment(&actual))
            })
        {
            gap = gap.min(alignment);
            // Alignment bounds are nonnegative. Zero is the exhaustive
            // minimum, so later boundary spans need not be constructed.
            if gap == 0.0 {
                return gap;
            }
        }
        return gap;
    }
    let frame = surface.frame();
    match curve {
        Curve::Line(_) => {
            let Some(a) = curve.evaluate(range.bounds()[0]) else {
                return f64::INFINITY;
            };
            let Some(b) = curve.evaluate(range.bounds()[1]) else {
                return f64::INFINITY;
            };
            if let Surface::Plane(_) = surface {
                let distance = plane_distance(frame);
                return distance(a.point).max(distance(b.point));
            }
            let (a, b) = (coordinates(frame, a.point), coordinates(frame, b.point));
            match surface {
                Surface::Ruled(_) | Surface::Plane(_) => unreachable!(),
                Surface::Sphere(_) => f64::INFINITY,
                Surface::Torus(t) => {
                    // Distance to a closed set is 1-Lipschitz. Each point of
                    // this segment is at most half its length from an endpoint;
                    // endpoint-only checks would miss a chord cutting the tube.
                    let gap = |p: [f64; 3]| {
                        (libm::hypot(libm::hypot(p[0], p[1]) - t.major_radius(), p[2])
                            - t.minor_radius())
                        .abs()
                    };
                    gap(a).max(gap(b)) + 0.5 * length(sub(b, a))
                }
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
                Surface::Cone(cone) => {
                    let radial = libm::hypot(a[0], a[1]);
                    if radial == 0.0 {
                        return f64::INFINITY;
                    }
                    let norm = libm::hypot(1.0, cone.slope());
                    let (sin, cos) = (cone.slope() / norm, 1.0 / norm);
                    let unit = [sin * a[0] / radial, sin * a[1] / radial, cos];
                    let origin = [
                        cone.radius() * a[0] / radial,
                        cone.radius() * a[1] / radial,
                        0.0,
                    ];
                    // Project both endpoints onto the same generator. The
                    // segment between regular projections is itself on the
                    // cone, so convexity bounds the entire original line.
                    let gap = |p: [f64; 3]| {
                        let t = dot(sub(p, origin), unit);
                        if !t.is_finite() || cone.radius() + sin * t <= 0.0 {
                            return f64::INFINITY;
                        }
                        length(sub(sub(p, origin), unit.map(|v| v * t)))
                    };
                    gap(a).max(gap(b))
                }
            }
        }
        Curve::Ellipse(ellipse) => crate::geometry::ellipse_support_gap(ellipse, surface),
        Curve::Nurbs(nurbs) => match surface {
            // The piece of curve lies in the convex hull of its controls, so
            // none strays from the plane further than the farthest control.
            Surface::Plane(_) => nurbs.piece(range).map_or(f64::INFINITY, |piece| {
                let scale = piece
                    .points()
                    .iter()
                    .flatten()
                    .map(|v| v.abs())
                    .fold(1.0, f64::max);
                let distance = plane_distance(frame);
                piece
                    .points()
                    .iter()
                    .map(|&p| distance(p))
                    .fold(0.0, f64::max)
                    + 64.0 * f64::EPSILON * scale
            }),
            _ => f64::INFINITY,
        },
        Curve::Circle(circle) => {
            let cf = circle.frame();
            let center = coordinates(frame, cf.rows.map(|r| r[3]));
            let axis = |i: usize| cf.rows.map(|r| r[i]);
            let normal = frame.rows.map(|r| r[2]);
            let tilt = libm::hypot(dot(axis(0), normal), dot(axis(1), normal));
            match surface {
                Surface::Ruled(_) => unreachable!(),
                Surface::Sphere(sphere) => crate::geometry::sphere_harmonic_gap(
                    sphere,
                    cf.rows.map(|r| r[3]),
                    cf.rows.map(|r| r[0] * circle.radius()),
                    cf.rows.map(|r| r[1] * circle.radius()),
                ),
                Surface::Torus(t) => {
                    // Bound the entire circle against either an orbit or a
                    // meridian circle, independently of its parameter trim.
                    // The factor two bounds rotation into the reference plane.
                    let rho = libm::hypot(center[0], center[1]);
                    let r = circle.radius();
                    let orbit = rho
                        + (libm::hypot(r - t.major_radius(), center[2]) - t.minor_radius()).abs()
                        + 2.0 * r * tilt;
                    let meridian = if rho == 0.0 {
                        f64::INFINITY
                    } else {
                        let radial = frame
                            .rows
                            .map(|row| (row[0] * center[0] + row[1] * center[1]) / rho);
                        let cn = axis(2);
                        let plane_tilt = libm::hypot(dot(cn, radial), dot(cn, normal));
                        libm::hypot(rho - t.major_radius(), center[2])
                            + (r - t.minor_radius()).abs()
                            + 2.0 * r * plane_tilt
                    };
                    orbit.min(meridian)
                }
                Surface::Plane(_) => spanned_normal(frame).map_or(f64::INFINITY, |n| {
                    plane_distance(frame)(cf.rows.map(|r| r[3]))
                        + circle.radius() * libm::hypot(dot(axis(0), n), dot(axis(1), n))
                }),
                Surface::Cylinder(cylinder) => {
                    libm::hypot(center[0], center[1])
                        + (circle.radius() - cylinder.radius()).abs()
                        + circle.radius() * tilt
                }
                Surface::Cone(cone) => {
                    let Some(radius) = cone.radius_at(center[2]) else {
                        return f64::INFINITY;
                    };
                    libm::hypot(center[0], center[1])
                        + (circle.radius() - radius).abs()
                        + 2.0 * circle.radius() * tilt
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloc::vec;
    const POLICY: ValidationPolicy = ValidationPolicy { max_gap: 1e-12 };

    fn line_rectangle_parts(cancelling: bool) -> BodyParts {
        use crate::geometry::CurveSpan;
        let lower = Curve::line([0., 0., 0.], [4., 0., 0.]).unwrap();
        let upper = Curve::line([0., 0., 16.], [4., 0., 0.]).unwrap();
        let surface = Surface::ruled(
            CurveSpan::new(lower.clone(), ParameterRange::UNIT, false).unwrap(),
            CurveSpan::new(upper.clone(), ParameterRange::UNIT, false).unwrap(),
        )
        .unwrap();
        let (right, range) = if cancelling {
            let direction = 2.0_f64.powi(60) + 256.;
            let min = 0.1_f64;
            (
                Curve::line([4., 0., -(direction * min)], [0., 0., direction]).unwrap(),
                ParameterRange::new(min, min.next_up()).unwrap(),
            )
        } else {
            (
                Curve::line([4., 0., 0.], [0., 0., 16.]).unwrap(),
                ParameterRange::UNIT,
            )
        };
        BodyParts {
            vertices: vec![[0., 0., 0.], [4., 0., 0.], [4., 0., 16.], [0., 0., 16.]],
            edges: vec![
                Edge::new(lower, ParameterRange::UNIT, VertexId(0), VertexId(1), ()),
                Edge::new(right, range, VertexId(1), VertexId(2), ()),
                Edge::new(upper, ParameterRange::UNIT, VertexId(3), VertexId(2), ()),
                Edge::new(
                    Curve::line([0., 0., 0.], [0., 0., 16.]).unwrap(),
                    ParameterRange::UNIT,
                    VertexId(0),
                    VertexId(3),
                    (),
                ),
            ],
            loops: vec![BoundaryLoop {
                coedges: (0..4)
                    .map(|i| Coedge {
                        edge: EdgeId(i),
                        reversed: i >= 2,
                    })
                    .collect(),
            }],
            faces: vec![Face {
                surface,
                domain: ParameterDomain {
                    u: ParameterRange::UNIT,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
            trims: vec![],
        }
    }

    #[test]
    fn public_line_endpoint_admission_refuses_hidden_coefficient_cancellation() {
        let parts = line_rectangle_parts(true);
        let (curve, range) = parts.edges[1].regular().unwrap();
        // The rounded evaluator aliases the authoritative endpoints. Exact
        // original-f64 coefficients instead give Z about -6.4 and 9.6.
        assert_eq!(
            curve.evaluate(range.bounds()[0]).unwrap().point,
            parts.vertices[1]
        );
        assert_eq!(
            curve.evaluate(range.bounds()[1]).unwrap().point,
            parts.vertices[2]
        );
        assert!(
            matches!(Body::try_from_parts(parts,&ValidationPolicy{max_gap:1e-9}),Err(BodyError::EndpointGap{edge:EdgeId(1),gap,..}) if gap>6.)
        );
        let exact = Body::try_from_parts(
            line_rectangle_parts(false),
            &ValidationPolicy { max_gap: 0. },
        )
        .unwrap();
        assert_eq!(
            exact
                .validate(&ValidationPolicy { max_gap: 0. })
                .unwrap()
                .max_gap,
            0.
        );
    }

    #[test]
    fn public_realization_never_reports_false_tolerance_for_a_cancelling_line() {
        use crate::realize::{RealizationError, RealizationPolicy};
        // A caller can explicitly admit a loose input gap, but realization must
        // revalidate against its own policy and retain the actual upper bound.
        let body = Body::try_from_parts(
            line_rectangle_parts(true),
            &ValidationPolicy { max_gap: 100. },
        )
        .unwrap();
        let input = body.validate(&ValidationPolicy { max_gap: 100. }).unwrap();
        assert!(input.max_gap > 6.);
        assert!(matches!(
            body.realize(Placement3::IDENTITY, &RealizationPolicy::default()),
            Err(RealizationError::Boundary(BodyError::EndpointGap {
                edge: EdgeId(1),
                ..
            }))
        ));
        let loose = body
            .realize(
                Placement3::IDENTITY,
                &RealizationPolicy {
                    geometry_gap: 100.,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!loose.report.within_tolerance());
        assert!(loose.report.geometry_gap >= input.max_gap);
        assert!(loose.report.max_deviation >= 2. * input.max_gap);
    }

    #[test]
    fn rounded_unit_addition_is_not_an_exact_endpoint_identity() {
        let parts = BodyParts::<(), ()> {
            vertices: vec![[1e16, 0., 0.], [1e16, 0., 0.]],
            edges: vec![Edge::new(
                Curve::line([1e16, 0., 0.], [1., 0., 0.]).unwrap(),
                ParameterRange::UNIT,
                VertexId(0),
                VertexId(1),
                (),
            )],
            ..Default::default()
        };
        assert!(
            matches!(Body::try_from_parts(parts,&ValidationPolicy{max_gap:0.}),Err(BodyError::EndpointGap{vertex:VertexId(1),gap,..}) if gap>=1.)
        );
    }

    fn exhaustive_ruled_gap(curve: &Curve, range: ParameterRange, surface: &Surface) -> f64 {
        let Surface::Ruled(ruled) = surface else {
            panic!("ruled surface required");
        };
        let boundaries: [_; 4] = core::array::from_fn(|side| ruled.boundary(side));
        let mut gap = f64::INFINITY;
        for reverse in [false, true] {
            if let Ok(actual) = crate::geometry::CurveSpan::new(curve.clone(), range, reverse) {
                for ideal in boundaries.iter().flatten() {
                    let alignment = ideal.alignment(&actual);
                    assert!(alignment >= 0.0);
                    gap = gap.min(alignment);
                }
            }
        }
        gap
    }

    fn ruled_support_fixture(rational: bool, reversed: bool) -> Surface {
        use crate::geometry::{CurveSpan, NurbsCurve};
        let (curve, range) = if rational {
            (
                Curve::nurbs(
                    NurbsCurve::new(
                        4,
                        vec![2.0, 2.0, 2.0, 2.0, 5.0, 5.0, 5.0, 5.0],
                        vec![[0.0; 3], [1.0, 2.0, 0.0], [3.0, 2.0, 0.0], [4.0, 0.0, 0.0]],
                        Some(vec![1.0, 0.7, 1.4, 1.0]),
                    )
                    .unwrap(),
                )
                .unwrap(),
                ParameterRange::new(2.0, 5.0).unwrap(),
            )
        } else {
            (
                Curve::line([0.0; 3], [4.0, 0.0, 0.0]).unwrap(),
                ParameterRange::UNIT,
            )
        };
        let upper = curve
            .transformed(Placement3::translate(0.0, 0.0, 2.0))
            .unwrap();
        Surface::ruled(
            CurveSpan::new(curve, range, reversed).unwrap(),
            CurveSpan::new(upper, range, reversed).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn ruled_support_lazy_minimum_matches_exhaustive_boundary_search() {
        for rational in [false, true] {
            for reversed in [false, true] {
                let surface = ruled_support_fixture(rational, reversed);
                let Surface::Ruled(ruled) = &surface else {
                    unreachable!()
                };
                for side in 0..4 {
                    let ideal = ruled.boundary(side).unwrap();
                    let curve = ideal.curve();
                    let range = ideal.range();
                    let exhaustive = exhaustive_ruled_gap(curve, range, &surface);
                    assert_eq!(support_gap(curve, range, &surface), 0.0);
                    assert!(exhaustive >= 0.0);

                    // These are not source identities: every candidate must
                    // still contribute to the same conservative minimum.
                    let shifted = curve
                        .transformed(Placement3::translate(0.125, 0.0, 0.0))
                        .unwrap();
                    let exhaustive = exhaustive_ruled_gap(&shifted, range, &surface);
                    assert!(exhaustive > 0.0);
                    assert_eq!(
                        support_gap(&shifted, range, &surface).to_bits(),
                        exhaustive.to_bits()
                    );
                }
                let last = ruled.boundary(3).unwrap();
                let Curve::Line(line) = last.curve() else {
                    unreachable!()
                };
                let backward = Curve::line(
                    core::array::from_fn(|k| {
                        line.origin()[k] + line.direction()[k] + if k == 0 { 0.125 } else { 0.0 }
                    }),
                    line.direction().map(|v| -v),
                )
                .unwrap();
                assert_eq!(
                    exhaustive_ruled_gap(&backward, ParameterRange::UNIT, &surface),
                    0.125
                );
                assert_eq!(
                    support_gap(&backward, ParameterRange::UNIT, &surface),
                    0.125
                );

                // Finite checked inputs can still have an unrepresentable
                // normalized span. Both searches must retain the refusal.
                let huge = ParameterRange::new(0.0, 1e300).unwrap();
                let line = Curve::line([0.0; 3], [1e100, 0.0, 0.0]).unwrap();
                assert_eq!(exhaustive_ruled_gap(&line, huge, &surface), f64::INFINITY);
                assert_eq!(support_gap(&line, huge, &surface), f64::INFINITY);
            }
        }
    }

    #[test]
    fn ruled_source_identity_proves_zero_beyond_alignment_roundoff_guards() {
        use crate::geometry::CurveSpan;
        let rational = ruled_support_fixture(true, false);
        let circle = Curve::circle(Placement3::IDENTITY, 2.0).unwrap();
        let range = ParameterRange::new(0.0, core::f64::consts::TAU).unwrap();
        let harmonic = Surface::ruled(
            CurveSpan::new(circle.clone(), range, true).unwrap(),
            CurveSpan::new(
                circle
                    .transformed(Placement3::translate(0.0, 0.0, 2.0))
                    .unwrap(),
                range,
                true,
            )
            .unwrap(),
        )
        .unwrap();
        for surface in [&rational, &harmonic] {
            let Surface::Ruled(ruled) = surface else {
                unreachable!()
            };
            for span in ruled.curves() {
                // This is a stronger exact-locus proof, not equality with the
                // old comparison's positive arithmetic allowance.
                assert!(exhaustive_ruled_gap(span.curve(), span.range(), surface) > 0.0);
                assert_eq!(support_gap(span.curve(), span.range(), surface), 0.0);
                let bounds = span.range().bounds();
                let partial =
                    ParameterRange::new(bounds[0], (bounds[0] + bounds[1]) / 2.0).unwrap();
                assert_eq!(
                    support_gap(span.curve(), partial, surface).to_bits(),
                    exhaustive_ruled_gap(span.curve(), partial, surface).to_bits(),
                    "different ranges must use the full comparison",
                );
            }
        }

        let Surface::Ruled(ruled) = &rational else {
            unreachable!()
        };
        let boundaries: [_; 4] = core::array::from_fn(|side| ruled.boundary(side).unwrap());
        let vertices = [
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 0.0, 2.0],
            [0.0, 0.0, 2.0],
        ];
        let endpoints = [(0, 1), (1, 2), (3, 2), (0, 3)];
        let parts = BodyParts {
            vertices: vertices.into(),
            edges: boundaries
                .into_iter()
                .zip(endpoints)
                .map(|(span, (start, end))| Edge {
                    geometry: EdgeGeometry::Curve {
                        curve: span.curve().clone(),
                        range: span.range(),
                    },
                    start: VertexId(start),
                    end: VertexId(end),
                    role: (),
                })
                .collect(),
            loops: vec![BoundaryLoop {
                coedges: (0..4)
                    .map(|i| Coedge {
                        edge: EdgeId(i),
                        reversed: i >= 2,
                    })
                    .collect(),
            }],
            faces: vec![Face {
                surface: rational,
                domain: ParameterDomain {
                    u: ParameterRange::UNIT,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
            trims: vec![],
        };
        let zero = ValidationPolicy { max_gap: 0.0 };
        let body = Body::try_from_parts(parts, &zero).unwrap();
        assert_eq!(body.validate(&zero).unwrap().max_gap, 0.0);
    }

    #[test]
    fn torus_support_bounds_whole_curves_including_chords_and_oblique_circles() {
        let surface = Surface::torus(Placement3::IDENTITY, 10.0, 2.0).unwrap();
        let meridian = Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [10.0, 0.0, 0.0],
        );
        for frame in [
            Placement3::IDENTITY,
            meridian,
            Placement3::rotate_y_then_translate(0.13, 10.0, 0.0, 0.2),
        ] {
            let curve = Curve::circle(
                frame,
                if frame == Placement3::IDENTITY {
                    12.0
                } else {
                    2.0
                },
            )
            .unwrap();
            let bound = support_gap(&curve, ParameterRange::UNIT, &surface);
            for i in 0..128 {
                let p = curve
                    .evaluate(f64::from(i) * core::f64::consts::TAU / 128.0)
                    .unwrap()
                    .point;
                let gap = (libm::hypot(libm::hypot(p[0], p[1]) - 10.0, p[2]) - 2.0).abs();
                assert!(gap <= bound + 1e-12, "whole-circle bound {gap} <= {bound}");
            }
        }
        let chord = Curve::line([12.0, 0.0, 0.0], [-24.0, 0.0, 0.0]).unwrap();
        assert!(
            support_gap(&chord, ParameterRange::UNIT, &surface) > 7.0,
            "surface endpoints do not validate the chord interior"
        );
    }

    #[test]
    fn plane_support_is_measured_from_the_plane_its_axes_span() {
        // An accepted frame is only nearly orthonormal: its stored normal
        // misses the plane its axes span by 5e-13 radians, which a megametre
        // out is 5e-7 from the plane points are evaluated on.
        let surface = Surface::plane(Placement3::from_axes(
            [1.0, 0.0, 5e-13],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0; 3],
        ))
        .unwrap();
        let far = [1e6, 0.0, 0.0];
        let unit = ParameterRange::UNIT;
        let nurbs =
            Curve::nurbs(crate::geometry::NurbsCurve::line(far, [1e6, 1.0, 0.0]).unwrap()).unwrap();
        let line = Curve::line(far, [0.0, 1.0, 0.0]).unwrap();
        let circle = Curve::circle(Placement3::translate(1e6, 0.0, 0.0), 1.0).unwrap();
        for curve in [nurbs, line, circle] {
            let gap = support_gap(&curve, unit, &surface);
            assert!((4.9e-7..1e-6).contains(&gap), "{curve:?}: {gap}");
        }
    }

    pub(crate) fn cylinder_parts() -> BodyParts {
        let circle = |z| Curve::circle(Placement3::translate(0.0, 0.0, z), 2.0).unwrap();
        let full = ParameterRange::new(0.0, core::f64::consts::TAU).unwrap();
        let coedge = |edge, reversed| Coedge {
            edge: EdgeId(edge),
            reversed,
        };
        BodyParts {
            trims: Vec::new(),
            vertices: vec![[2.0, 0.0, 0.0], [2.0, 0.0, 3.0]],
            edges: vec![
                Edge {
                    geometry: EdgeGeometry::Curve {
                        curve: circle(0.0),
                        range: full,
                    },
                    start: VertexId(0),
                    end: VertexId(0),
                    role: (),
                },
                Edge {
                    geometry: EdgeGeometry::Curve {
                        curve: circle(3.0),
                        range: full,
                    },
                    start: VertexId(1),
                    end: VertexId(1),
                    role: (),
                },
                Edge {
                    geometry: EdgeGeometry::Curve {
                        curve: Curve::line([2.0, 0.0, 0.0], [0.0, 0.0, 3.0]).unwrap(),
                        range: ParameterRange::UNIT,
                    },
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
    fn cone_support_checks_complete_generators_circles_and_apex_domains() {
        let surface = Surface::cone(Placement3::IDENTITY, 2.0, 0.5).unwrap();
        let generator = Curve::line([2.0, 0.0, 0.0], [1.0, 0.0, 2.0]).unwrap();
        assert!(support_gap(&generator, ParameterRange::UNIT, &surface) < 1e-12);
        let ring = Curve::circle(Placement3::translate(0.0, 0.0, 2.0), 3.0).unwrap();
        assert!(support_gap(&ring, ParameterRange::UNIT, &surface) < 1e-12);
        let chord = Curve::line([2.0, 0.0, 0.0], [-4.0, 0.0, 0.0]).unwrap();
        assert!(support_gap(&chord, ParameterRange::UNIT, &surface) > 1.0);
        let mut parts = cylinder_parts();
        parts.faces[0].surface = Surface::cone(Placement3::IDENTITY, 2.0, -1.0).unwrap();
        assert!(matches!(
            Body::try_from_parts(parts, &POLICY),
            Err(BodyError::NonRegularSurface { face: FaceId(0) })
        ));
    }

    #[test]
    fn endpoints_on_a_cylinder_do_not_hide_an_interior_chord_gap() {
        let mut parts = cylinder_parts();
        parts.vertices = vec![[2.0, 0.0, 0.0], [-2.0, 0.0, 0.0]];
        parts.edges = vec![Edge {
            geometry: EdgeGeometry::Curve {
                curve: Curve::line([2.0, 0.0, 0.0], [-4.0, 0.0, 0.0]).unwrap(),
                range: ParameterRange::UNIT,
            },
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

#[cfg(test)]
mod pole_tests;
