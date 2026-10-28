// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Analytic plane intersection of strictly convex planar retained boundaries.
//!
//! Every source face must be a convex line polygon and bound the material by
//! its outward halfspace. Curved faces, holes, concavity, coplanar events and
//! cuts near existing vertices are explicit refusals. Intersections are solved
//! once per shared native edge, then reused by both incident faces. No mesh is
//! consulted. Output retains source vertices and edges, including detached
//! entries, and reports face/edge images for subsequent edits.
use crate::body::{
    Body, BodyError, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, FaceId, LoopId, Sense,
    ValidationPolicy, VertexId,
};
use crate::geometry::{Curve, GeometryError, ParameterDomain, ParameterRange, Surface};
use alloc::vec::Vec;
use exedra_math::{Placement3, add, cross, dot, scale, sub};

/// Closed material halfspace: points on the negative side of its outward plane.
#[derive(Clone, Debug, PartialEq)]
pub struct HalfSpace {
    plane: Surface,
}
impl HalfSpace {
    /// Defines retained points by `dot(point - origin, outward_normal) <= 0`.
    /// # Errors
    /// Refuses nonfinite inputs or a zero normal.
    pub fn new(origin: [f64; 3], outward_normal: [f64; 3]) -> Result<Self, GeometryError> {
        let n = norm(outward_normal);
        if !n.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if n == 0.0 {
            return Err(GeometryError::ZeroDirection);
        }
        let z = scale(outward_normal, 1.0 / n);
        let seed = if z[0].abs() < 0.8 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let y = unit(cross(z, seed));
        let x = cross(y, z);
        Ok(Self {
            plane: Surface::plane(Placement3::from_axes(x, y, z, origin))?,
        })
    }
    /// The supporting plane, with positive normal pointing out of retained material.
    #[must_use]
    pub const fn surface(&self) -> &Surface {
        &self.plane
    }
}
/// Explicit bounds for a convex planar intersection.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClipPolicy {
    /// Maximum native support disagreement and derived arithmetic allowance.
    pub geometry_gap: f64,
    /// Strict minimum plane distance from any used source vertex.
    /// Contact inside this distance is refused, never snapped or healed.
    pub vertex_clearance: f64,
}
impl Default for ClipPolicy {
    fn default() -> Self {
        Self {
            geometry_gap: 1e-9,
            vertex_clearance: 1e-6,
        }
    }
}
/// Material relation of the complete source to the clipping halfspace.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClipOutcome {
    /// Every used source vertex lies strictly in the retained halfspace.
    Unchanged,
    /// Every used source vertex lies strictly outside; the output body is empty.
    Empty,
    /// Shared source edges were split and one planar cap was inserted.
    Clipped,
}
/// Image of one source face, including removed faces.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClippedFace {
    /// Source face identity.
    pub source: FaceId,
    /// Output face identity, absent if removed.
    pub result: Option<FaceId>,
    /// Whether the surviving face's boundary changed.
    pub trimmed: bool,
}
/// One exact-form shared line/plane intersection.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EdgeSplit {
    /// Source edge identity.
    pub source: EdgeId,
    /// Authoritative output intersection vertex.
    pub vertex: VertexId,
    /// Fraction along the canonical source start-to-end direction.
    pub fraction: f64,
}
/// Operation outcome, images and conservative native validation evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipReport {
    /// Whether material was retained, removed or split.
    pub outcome: ClipOutcome,
    /// All source face images, in source order.
    pub faces: Vec<ClippedFace>,
    /// Surviving edge fragments for each source edge, in source order.
    pub edge_images: Vec<Vec<EdgeId>>,
    /// Shared intersections, in source edge order.
    pub splits: Vec<EdgeSplit>,
    /// New closing plane face, present only for a proper cut.
    pub cap: Option<FaceId>,
    /// Cap boundary edge identities in traversal order.
    pub cap_edges: Vec<EdgeId>,
    /// Conservative arithmetic allowance plus validated support disagreement.
    pub max_gap: f64,
}
/// Complete transactional planar intersection result.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipResult<F = (), E = ()> {
    /// Retained native boundary; empty only for [`ClipOutcome::Empty`].
    pub body: Body<F, E>,
    /// Explicit images, splits, cap and outcome.
    pub report: ClipReport,
}
/// Why a planar halfspace intersection could not be established.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipError {
    /// Policy values must be finite, with a nonnegative gap and positive clearance.
    InvalidPolicy,
    /// Input/output native support or incidence validation failed.
    Boundary(BodyError),
    /// Input is empty or is not an oriented closed boundary.
    OpenBoundary,
    /// Input has more than one disconnected face shell.
    DisconnectedShell,
    /// Face is curved, holed, nonconvex, degenerate or wrongly oriented.
    UnsupportedFace {
        /// Source identity.
        face: FaceId,
    },
    /// Source material violates an outward face halfspace.
    Nonconvex {
        /// Bounding source face.
        face: FaceId,
        /// Outside source vertex.
        vertex: VertexId,
        /// Positive outward distance.
        distance: f64,
    },
    /// Plane contact is too close to a used source vertex for this strict operation.
    NearVertex {
        /// Source identity.
        vertex: VertexId,
        /// Signed source-plane distance.
        distance: f64,
        /// Required absolute distance.
        required: f64,
    },
    /// Derived arithmetic or the cap cycle cannot meet the requested bounds.
    NumericLimit,
    /// Primitive construction refused derived geometry.
    Geometry(GeometryError),
}
impl core::fmt::Display for ClipError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "analytic halfspace intersection: {self:?}")
    }
}
impl core::error::Error for ClipError {}

/// Intersects a convex planar body with a halfspace, using default new roles.
/// # Errors
/// Refuses unsupported support/trim shapes, nonconvex input and near contact.
pub fn intersect_half_space<F: Clone + Default, E: Clone + Default>(
    body: &Body<F, E>,
    halfspace: &HalfSpace,
    policy: &ClipPolicy,
) -> Result<ClipResult<F, E>, ClipError> {
    intersect_half_space_with_roles(body, halfspace, policy, F::default(), E::default())
}
/// Intersects a strictly convex planar solid with one material halfspace.
///
/// The input is not mutated. Existing vertices/edges retain IDs, even when
/// detached. Source faces are compacted; [`ClipReport::faces`] is authoritative
/// correspondence. New cap entities receive explicit caller metadata. Complete
/// removal and no-op results are explicit, distinct successful outcomes.
/// # Errors
/// Refuses curved or holed faces, nonconvexity, contact/coplanarity, arithmetic
/// uncertainty above policy, and nonclosed input/output boundaries.
pub fn intersect_half_space_with_roles<F: Clone, E: Clone>(
    body: &Body<F, E>,
    halfspace: &HalfSpace,
    policy: &ClipPolicy,
    face_role: F,
    edge_role: E,
) -> Result<ClipResult<F, E>, ClipError> {
    if !policy.geometry_gap.is_finite()
        || policy.geometry_gap < 0.0
        || !policy.vertex_clearance.is_finite()
        || policy.vertex_clearance <= 0.0
    {
        return Err(ClipError::InvalidPolicy);
    }
    let validation = ValidationPolicy {
        max_gap: policy.geometry_gap,
    };
    body.validate(&validation).map_err(ClipError::Boundary)?;
    if !body.is_closed() {
        return Err(ClipError::OpenBoundary);
    }
    let frame = halfspace.plane.frame();
    let origin = col(frame, 3);
    let normal = col(frame, 2);
    let magnitude = body
        .vertices()
        .iter()
        .flatten()
        .copied()
        .chain(origin)
        .map(f64::abs)
        .fold(0.0, f64::max);
    let arithmetic = 512.0 * f64::EPSILON * magnitude;
    if !arithmetic.is_finite() || arithmetic > policy.geometry_gap {
        return Err(ClipError::NumericLimit);
    }
    let mut used = alloc::vec![false;body.vertices().len()];
    let mut used_edges = alloc::vec![false;body.edges().len()];
    let mut polygons: Vec<Vec<VertexId>> = Vec::with_capacity(body.faces().len());
    for (index, face) in body.faces().iter().enumerate() {
        let id = FaceId(crate::usize_to_u32(index));
        let Surface::Plane(plane) = &face.surface else {
            return Err(ClipError::UnsupportedFace { face: id });
        };
        if face.loops.len() != 1 {
            return Err(ClipError::UnsupportedFace { face: id });
        }
        let loop_ = &body.loops()[face.loops[0].index()];
        if loop_.coedges.len() < 3 {
            return Err(ClipError::UnsupportedFace { face: id });
        }
        let mut vertices = Vec::with_capacity(loop_.coedges.len());
        for co in &loop_.coedges {
            used_edges[co.edge.index()] = true;
            if !matches!(
                body.edges()[co.edge.index()].regular(),
                Some((Curve::Line(_), _))
            ) {
                return Err(ClipError::UnsupportedFace { face: id });
            }
            let v = body.coedge_vertices(*co).ok_or(ClipError::NumericLimit)?[0];
            used[v.index()] = true;
            vertices.push(v);
        }
        let outward = scale(col(plane.frame(), 2), face.sense.sign());
        for i in 0..vertices.len() {
            let a = body.vertices()[vertices[i].index()];
            let b = body.vertices()[vertices[(i + 1) % vertices.len()].index()];
            let ab = sub(b, a);
            for (j, &vertex) in vertices.iter().enumerate() {
                if j == i || j == (i + 1) % vertices.len() {
                    continue;
                }
                let ap = sub(body.vertices()[vertex.index()], a);
                let side = dot(cross(ab, ap), outward);
                let uncertainty = arithmetic * (norm(ab) + norm(ap));
                if !side.is_finite() || side <= uncertainty {
                    return Err(ClipError::UnsupportedFace { face: id });
                }
            }
        }
        polygons.push(vertices);
    }
    // Incidence closure alone permits multiple shells; this convex operation
    // needs one connected boundary component.
    let mut reached = alloc::vec![false;body.faces().len()];
    reached[0] = true;
    loop {
        let mut progress = false;
        for i in 0..body.faces().len() {
            if !reached[i] {
                continue;
            }
            let edges = &body.loops()[body.faces()[i].loops[0].index()].coedges;
            for (j, face) in body.faces().iter().enumerate() {
                if reached[j] {
                    continue;
                }
                if body.loops()[face.loops[0].index()]
                    .coedges
                    .iter()
                    .any(|co| edges.iter().any(|other| other.edge == co.edge))
                {
                    reached[j] = true;
                    progress = true;
                }
            }
        }
        if !progress {
            break;
        }
    }
    if reached.iter().any(|&r| !r) {
        return Err(ClipError::DisconnectedShell);
    }
    // Whole-polyhedron convexity is a finite vertex/plane test, not surface sampling.
    for (index, face) in body.faces().iter().enumerate() {
        let f = face.surface.frame();
        let n = scale(col(f, 2), face.sense.sign());
        for (i, &point) in body.vertices().iter().enumerate() {
            if !used[i] {
                continue;
            }
            let distance = dot(sub(point, col(f, 3)), n);
            if distance > policy.geometry_gap - arithmetic {
                return Err(ClipError::Nonconvex {
                    face: FaceId(crate::usize_to_u32(index)),
                    vertex: VertexId(crate::usize_to_u32(i)),
                    distance,
                });
            }
        }
    }
    let distances = body
        .vertices()
        .iter()
        .map(|&p| dot(sub(p, origin), normal))
        .collect::<Vec<_>>();
    let mut inside = 0;
    let mut outside = 0;
    for (i, (&d, &is_used)) in distances.iter().zip(&used).enumerate() {
        if !is_used {
            continue;
        }
        if !d.is_finite() {
            return Err(ClipError::NumericLimit);
        }
        if d.abs() <= policy.vertex_clearance + arithmetic {
            return Err(ClipError::NearVertex {
                vertex: VertexId(crate::usize_to_u32(i)),
                distance: d,
                required: policy.vertex_clearance + arithmetic,
            });
        }
        if d < 0.0 {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    if inside == 0 || outside == 0 {
        let outcome = if inside == 0 {
            ClipOutcome::Empty
        } else {
            ClipOutcome::Unchanged
        };
        let kept = inside != 0;
        return Ok(ClipResult {
            body: if kept { body.clone() } else { Body::default() },
            report: ClipReport {
                outcome,
                faces: body
                    .faces()
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let id = FaceId(crate::usize_to_u32(i));
                        ClippedFace {
                            source: id,
                            result: kept.then_some(id),
                            trimmed: false,
                        }
                    })
                    .collect(),
                edge_images: body
                    .edges()
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        if kept && used_edges[i] {
                            alloc::vec![EdgeId(crate::usize_to_u32(i))]
                        } else {
                            Vec::new()
                        }
                    })
                    .collect(),
                splits: Vec::new(),
                cap: None,
                cap_edges: Vec::new(),
                max_gap: arithmetic,
            },
        });
    }
    let mut parts = BodyParts {
        trims: Vec::new(),
        vertices: body.vertices().to_vec(),
        edges: body.edges().to_vec(),
        loops: Vec::new(),
        faces: Vec::new(),
    };
    let mut splits = Vec::new();
    let mut intersections = alloc::vec![None;body.edges().len()];
    for (index, edge) in body.edges().iter().enumerate() {
        if !used_edges[index] {
            continue;
        }
        let a = distances[edge.start.index()];
        let b = distances[edge.end.index()];
        if (a < 0.0) == (b < 0.0) {
            continue;
        }
        let fraction = a / (a - b);
        if !fraction.is_finite() || fraction <= 0.0 || fraction >= 1.0 {
            return Err(ClipError::NumericLimit);
        }
        let point = add(
            body.vertices()[edge.start.index()],
            scale(
                sub(
                    body.vertices()[edge.end.index()],
                    body.vertices()[edge.start.index()],
                ),
                fraction,
            ),
        );
        let gap = dot(sub(point, origin), normal).abs() + arithmetic;
        if !gap.is_finite() || gap > policy.geometry_gap {
            return Err(ClipError::NumericLimit);
        }
        let vertex = VertexId(crate::usize_to_u32(parts.vertices.len()));
        parts.vertices.push(point);
        intersections[index] = Some(vertex);
        splits.push(EdgeSplit {
            source: EdgeId(crate::usize_to_u32(index)),
            vertex,
            fraction,
        });
    }
    let mut faces = Vec::new();
    let mut edge_images = alloc::vec![Vec::new();body.edges().len()];
    let mut cap_segments = Vec::new();
    for (index, polygon) in polygons.iter().enumerate() {
        let source_face = &body.faces()[index];
        let source_loop = &body.loops()[source_face.loops[0].index()];
        let mut clipped = Vec::new();
        let mut trimmed = false;
        for i in 0..polygon.len() {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            let ai = distances[a.index()] < 0.0;
            let bi = distances[b.index()] < 0.0;
            if ai {
                clipped.push(a);
            }
            if ai != bi {
                clipped.push(
                    intersections[source_loop.coedges[i].edge.index()]
                        .ok_or(ClipError::NumericLimit)?,
                );
            }
            if !ai {
                trimmed = true;
            }
        }
        let source = FaceId(crate::usize_to_u32(index));
        if clipped.is_empty() {
            faces.push(ClippedFace {
                source,
                result: None,
                trimmed: false,
            });
            continue;
        }
        if clipped.len() < 3 {
            return Err(ClipError::NumericLimit);
        }
        let mut coedges = Vec::new();
        for i in 0..clipped.len() {
            let a = clipped[i];
            let b = clipped[(i + 1) % clipped.len()];
            let source_edge = source_loop.coedges.iter().find_map(|co| {
                let e = &body.edges()[co.edge.index()];
                let mid = intersections[co.edge.index()];
                let belongs = |v| v == e.start || v == e.end || Some(v) == mid;
                if belongs(a) && belongs(b) {
                    Some(co.edge)
                } else {
                    None
                }
            });
            let role = source_edge.map_or_else(
                || edge_role.clone(),
                |e| body.edges()[e.index()].role.clone(),
            );
            let co = segment(&mut parts, a, b, role)?;
            coedges.push(co);
            if let Some(e) = source_edge {
                if !edge_images[e.index()].contains(&co.edge) {
                    edge_images[e.index()].push(co.edge);
                }
            } else {
                cap_segments.push((b, a));
            }
        }
        let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
        parts.loops.push(BoundaryLoop { coedges });
        let result = FaceId(crate::usize_to_u32(parts.faces.len()));
        let mut output_face = source_face.clone();
        output_face.loops = alloc::vec![loop_id];
        parts.faces.push(output_face);
        faces.push(ClippedFace {
            source,
            result: Some(result),
            trimmed,
        });
    }
    if cap_segments.len() < 3 {
        return Err(ClipError::NumericLimit);
    }
    let mut cap_vertices = Vec::with_capacity(cap_segments.len());
    let start = cap_segments[0].0;
    let mut current = start;
    while !cap_segments.is_empty() {
        let Some(index) = cap_segments.iter().position(|s| s.0 == current) else {
            return Err(ClipError::NumericLimit);
        };
        let (a, b) = cap_segments.remove(index);
        cap_vertices.push(a);
        current = b;
    }
    if current != start {
        return Err(ClipError::NumericLimit);
    }
    let mut coedges = Vec::new();
    let mut cap_edges = Vec::new();
    let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    for i in 0..cap_vertices.len() {
        let a = cap_vertices[i];
        let b = cap_vertices[(i + 1) % cap_vertices.len()];
        let co = segment(&mut parts, a, b, edge_role.clone())?;
        cap_edges.push(co.edge);
        coedges.push(co);
        let delta = sub(parts.vertices[a.index()], origin);
        for (j, axis) in [col(frame, 0), col(frame, 1)].into_iter().enumerate() {
            let uv = dot(delta, axis);
            bounds[0][j] = bounds[0][j].min(uv);
            bounds[1][j] = bounds[1][j].max(uv);
        }
    }
    let loop_id = LoopId(crate::usize_to_u32(parts.loops.len()));
    parts.loops.push(BoundaryLoop { coedges });
    let cap = FaceId(crate::usize_to_u32(parts.faces.len()));
    parts.faces.push(Face {
        surface: halfspace.plane.clone(),
        domain: ParameterDomain {
            u: ParameterRange::new(
                (bounds[0][0] - policy.geometry_gap).next_down(),
                (bounds[1][0] + policy.geometry_gap).next_up(),
            )
            .map_err(ClipError::Geometry)?,
            v: ParameterRange::new(
                (bounds[0][1] - policy.geometry_gap).next_down(),
                (bounds[1][1] + policy.geometry_gap).next_up(),
            )
            .map_err(ClipError::Geometry)?,
        },
        sense: Sense::Forward,
        loops: alloc::vec![loop_id],
        role: face_role,
    });
    crate::operation_trims::append_all_planar(&mut parts).map_err(ClipError::Geometry)?;
    let body = Body { parts };
    if !body.is_closed() {
        return Err(ClipError::OpenBoundary);
    }
    let checked = body.validate(&validation).map_err(ClipError::Boundary)?;
    Ok(ClipResult {
        body,
        report: ClipReport {
            outcome: ClipOutcome::Clipped,
            faces,
            edge_images,
            splits,
            cap: Some(cap),
            cap_edges,
            max_gap: arithmetic.max(checked.max_gap),
        },
    })
}
fn segment<F, E>(
    parts: &mut BodyParts<F, E>,
    a: VertexId,
    b: VertexId,
    role: E,
) -> Result<Coedge, ClipError> {
    if a == b {
        return Err(ClipError::NumericLimit);
    }
    if let Some((i, e)) = parts
        .edges
        .iter()
        .enumerate()
        .find(|(_, e)| (e.start == a && e.end == b) || (e.start == b && e.end == a))
    {
        return Ok(Coedge {
            edge: EdgeId(crate::usize_to_u32(i)),
            reversed: e.start != a,
        });
    }
    let start = parts.vertices[a.index()];
    let end = parts.vertices[b.index()];
    let edge = EdgeId(crate::usize_to_u32(parts.edges.len()));
    parts.edges.push(Edge::new(
        Curve::line(start, sub(end, start)).map_err(ClipError::Geometry)?,
        ParameterRange::UNIT,
        a,
        b,
        role,
    ));
    Ok(Coedge {
        edge,
        reversed: false,
    })
}
fn col(frame: Placement3, i: usize) -> [f64; 3] {
    frame.rows.map(|r| r[i])
}
fn norm(v: [f64; 3]) -> f64 {
    libm::hypot(libm::hypot(v[0], v[1]), v[2])
}
fn unit(v: [f64; 3]) -> [f64; 3] {
    scale(v, 1.0 / norm(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extrusion::Extrusion;
    use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    fn stock() -> Extrusion {
        let outline = Loop::new(alloc::vec![
            Segment::line([10.0, -8.0]),
            Segment::line([10.0, 8.0]),
            Segment::line([-10.0, 8.0]),
            Segment::line([-10.0, -8.0])
        ])
        .unwrap();
        Extrusion::new(
            Profile::new(outline, alloc::vec![], &ProfilePolicy::new(1e-9)).unwrap(),
            Placement3::IDENTITY,
            6.0,
        )
        .unwrap()
    }
    fn volume<F, E>(body: &Body<F, E>) -> f64 {
        let mut volume = 0.0;
        for face in body.faces() {
            let loop_ = &body.loops()[face.loops[0].index()];
            let vertices = loop_
                .coedges
                .iter()
                .map(|c| body.vertices()[body.coedge_vertices(*c).unwrap()[0].index()])
                .collect::<Vec<_>>();
            for i in 1..vertices.len() - 1 {
                volume += dot(vertices[0], cross(vertices[i], vertices[i + 1])) / 6.0;
            }
        }
        volume
    }
    #[test]
    fn axis_and_oblique_cuts_share_splits_preserve_roles_and_close() {
        let source = stock();
        for (point, normal, expected, intersections) in [
            ([3.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1248.0, 4),
            ([0.0; 3], [1.0, 1.0, 0.0], 960.0, 4),
            ([0.0, 0.0, 3.0], [1.0, 1.0, 1.0], 960.0, 6),
        ] {
            let original = source.body().clone();
            let result = intersect_half_space_with_roles(
                source.body(),
                &HalfSpace::new(point, normal).unwrap(),
                &ClipPolicy::default(),
                source.body().faces()[0].role,
                source.body().edges()[0].role,
            )
            .unwrap();
            assert_eq!(source.body(), &original);
            assert_eq!(result.report.outcome, ClipOutcome::Clipped);
            assert!(result.body.is_closed());
            assert!((0..result.body.faces().len()).all(|i| {
                result
                    .body
                    .has_explicit_trims(FaceId(crate::usize_to_u32(i)))
            }));
            assert_eq!(result.report.splits.len(), intersections);
            assert_eq!(result.report.cap_edges.len(), intersections);
            assert!((volume(&result.body) - expected).abs() < 1e-9);
            let realized = result
                .body
                .realize(
                    Placement3::IDENTITY,
                    &crate::realize::RealizationPolicy::default(),
                )
                .unwrap();
            assert!(realized.mesh.validate_deep().is_empty());
            assert!(realized.mesh.boundary_loops().unwrap().is_empty());
            assert!(realized.report.within_tolerance());
            for split in &result.report.splits {
                assert!(split.fraction > 0.0 && split.fraction < 1.0);
            }
        }
    }
    #[test]
    fn repeated_clipping_is_constructor_independent() {
        let source = stock();
        let mut body = source.body().clone();
        for (point, normal) in [
            ([3.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            ([0.0, 2.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, 4.0], [0.0, 0.0, 1.0]),
        ] {
            body = intersect_half_space_with_roles(
                &body,
                &HalfSpace::new(point, normal).unwrap(),
                &ClipPolicy::default(),
                source.body().faces()[0].role,
                source.body().edges()[0].role,
            )
            .unwrap()
            .body;
        }
        assert!(body.is_closed());
        assert!((volume(&body) - 520.0).abs() < 1e-9);
    }
    #[test]
    fn unsupported_concavity_curves_and_invalid_policy_are_refused() {
        let source = stock();
        let plane = HalfSpace::new([3.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
        assert!(matches!(
            intersect_half_space_with_roles(
                source.body(),
                &plane,
                &ClipPolicy {
                    geometry_gap: f64::NAN,
                    ..Default::default()
                },
                source.body().faces()[0].role,
                source.body().edges()[0].role
            ),
            Err(ClipError::InvalidPolicy)
        ));
        let outline = Loop::new(alloc::vec![
            Segment::line([10.0, -8.0]),
            Segment::line([10.0, 0.0]),
            Segment::line([0.0, 0.0]),
            Segment::line([0.0, 8.0]),
            Segment::line([-10.0, 8.0]),
            Segment::line([-10.0, -8.0])
        ])
        .unwrap();
        let concave = Extrusion::new(
            Profile::new(outline, alloc::vec![], &ProfilePolicy::new(1e-9)).unwrap(),
            Placement3::IDENTITY,
            6.0,
        )
        .unwrap();
        assert!(matches!(
            intersect_half_space_with_roles(
                concave.body(),
                &plane,
                &ClipPolicy::default(),
                concave.body().faces()[0].role,
                concave.body().edges()[0].role
            ),
            Err(ClipError::UnsupportedFace { .. }) | Err(ClipError::Nonconvex { .. })
        ));
        let circle = Loop::new(alloc::vec![
            Segment::arc([-5.0, 0.0], 1.0),
            Segment::arc([5.0, 0.0], 1.0)
        ])
        .unwrap();
        let curved = Extrusion::new(
            Profile::new(circle, alloc::vec![], &ProfilePolicy::new(1e-9)).unwrap(),
            Placement3::IDENTITY,
            6.0,
        )
        .unwrap();
        assert!(matches!(
            intersect_half_space_with_roles(
                curved.body(),
                &plane,
                &ClipPolicy::default(),
                curved.body().faces()[0].role,
                curved.body().edges()[0].role
            ),
            Err(ClipError::UnsupportedFace { .. })
        ));
    }
    #[test]
    fn empty_noop_and_tangent_outcomes_are_explicit() {
        let source = stock();
        let apply = |x| {
            intersect_half_space_with_roles(
                source.body(),
                &HalfSpace::new([x, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap(),
                &ClipPolicy::default(),
                source.body().faces()[0].role,
                source.body().edges()[0].role,
            )
        };
        let kept = apply(20.0).unwrap();
        assert_eq!(kept.report.outcome, ClipOutcome::Unchanged);
        assert_eq!(&kept.body, source.body());
        assert!(kept.report.cap.is_none());
        let removed = apply(-20.0).unwrap();
        assert_eq!(removed.report.outcome, ClipOutcome::Empty);
        assert!(removed.body.faces().is_empty());
        assert!(removed.report.faces.iter().all(|f| f.result.is_none()));
        assert!(matches!(apply(10.0), Err(ClipError::NearVertex { .. })));
        assert!(matches!(
            apply(10.0 - 1e-8),
            Err(ClipError::NearVertex { .. })
        ));
        assert!(HalfSpace::new([0.0; 3], [0.0; 3]).is_err());
    }
}
