// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Straight concave chains: local clearance and triangulated end-cap growth.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use exedra_math::{add, cross, dot, narrow, norm, normalize, promote, scale, sub};
use exedra_triangulate::{PolygonInput, TriParams, triangulate};

use super::{Chain, Planner, RoundError, RoundFaceSource, SelEdge, Subst, Tok};
use crate::{FaceId, VertexId};

impl Planner<'_> {
    /// Straight concave chains use the ordinary strip and end-face rewrite.
    /// Their bends and mixed-sense corners need different intersection patches
    /// and must never reach the convex miter or trihedral-corner builders.
    pub(super) fn check_concave_chains(
        &self,
        selected: &[SelEdge],
        adjacency: &BTreeMap<VertexId, Vec<usize>>,
        sweeps: &[f64],
    ) -> Result<(), RoundError> {
        for (index, edge) in selected.iter().enumerate() {
            if sweeps[index] > 0.0 {
                continue;
            }
            let unsupported = RoundError::ConcaveEdge {
                a: edge.a.index().min(edge.b.index()),
                b: edge.a.index().max(edge.b.index()),
            };
            for vertex in [edge.a, edge.b] {
                let incident = &adjacency[&vertex];
                if incident.len() == 1 {
                    continue;
                }
                let [first, second] = incident.as_slice() else {
                    return Err(unsupported);
                };
                let other_index = if *first == index { *second } else { *first };
                let other = selected[other_index];
                if sweeps[other_index] > 0.0 {
                    return Err(unsupported);
                }
                let other_vertex = |edge: SelEdge| {
                    if edge.a == vertex { edge.b } else { edge.a }
                };
                let anchor = self.position(vertex);
                let incoming = normalize(sub(anchor, self.position(other_vertex(*edge))))
                    .ok_or(unsupported)?;
                let outgoing = normalize(sub(self.position(other_vertex(other)), anchor))
                    .ok_or(unsupported)?;
                if dot(incoming, outgoing) < 1.0 - 1e-12 {
                    return Err(unsupported);
                }
                // Canonical edge orientations need not agree along the chain.
                let (left, right) = if (edge.b == vertex) == (other.a == vertex) {
                    (other.left, other.right)
                } else {
                    (other.right, other.left)
                };
                if dot(self.planes[&edge.left].normal, self.planes[&left].normal) < 1.0 - 1e-12
                    || dot(self.planes[&edge.right].normal, self.planes[&right].normal)
                        < 1.0 - 1e-12
                {
                    return Err(unsupported);
                }
            }
        }
        Ok(())
    }

    /// The triangle between the original corner and its two tangencies
    /// encloses the added fillet (and equals the added chamfer). Refuse a
    /// finish if another source face enters that swept triangle. This is
    /// intentionally conservative near an obstruction beyond the fillet arc.
    pub(super) fn check_concave_clearance(
        &self,
        chains: &[Chain],
        vertex_faces: &BTreeMap<VertexId, Vec<FaceId>>,
    ) -> Result<(), RoundError> {
        for chain in chains.iter().filter(|chain| chain.edges[0].sweep < 0.0) {
            let edge = chain.edges[0];
            let anchor = self.position(edge.a);
            let end = self.position(*chain.verts.last().expect("open chain"));
            let direction = normalize(sub(end, anchor)).expect("validated direction");
            let left = self.planes[&edge.left].normal;
            let right = self.planes[&edge.right].normal;
            let diagonal = normalize(add(left, right)).expect("validated angle");
            let tangency = self.points[chain.sections[0][0] as usize];
            let planes = [
                (left, 0.0),
                (right, 0.0),
                (direction, 0.0),
                (scale(direction, -1.0), -dot(direction, sub(end, anchor))),
                (scale(diagonal, -1.0), -dot(diagonal, sub(tangency, anchor))),
            ];
            let coordinate_scale = anchor
                .into_iter()
                .chain(end)
                .fold(self.policy.offset(), |a, b| a.max(b.abs()));
            let tolerance = 32.0 * f64::EPSILON * coordinate_scale;
            let mut incident: BTreeSet<_> = chain
                .edges
                .iter()
                .flat_map(|edge| [edge.left, edge.right])
                .collect();
            for vertex in [chain.verts.first(), chain.verts.last()]
                .into_iter()
                .flatten()
            {
                incident.extend(vertex_faces[vertex].iter().copied());
            }
            for face in self.mesh.faces().filter(|face| !incident.contains(face)) {
                let points: Vec<_> = self
                    .face_points(face)
                    .into_iter()
                    .map(|p| sub(p, anchor))
                    .collect();
                if enters_prism(&points, &planes, tolerance) {
                    return Err(RoundError::ClearanceExceeded { face: face.index() });
                }
            }
        }
        Ok(())
    }

    /// Concave caps grow into the void, so preserve their existing face fan
    /// and attach a planar fan along the added arc. This also handles caps
    /// already triangulated by a profile extrusion or Boolean cut.
    pub(super) fn build_concave_end(
        &mut self,
        chain: &Chain,
        position: usize,
        faces: &[FaceId],
    ) -> Result<(), RoundError> {
        let vertex = chain.verts[position];
        let unsupported = RoundError::UnsupportedEnd {
            vertex: vertex.index(),
        };
        let owner = *faces.first().ok_or(unsupported)?;
        let normal = self.plane(owner)?.normal;
        let anchor = self.position(vertex);
        let section = &chain.sections[position];
        let edge = chain.end_edge(position != 0);
        let direction = normalize(sub(self.position(edge.b), self.position(edge.a)))
            .expect("validated direction");
        if dot(normal, direction).abs() < 1.0 - 1e-12 {
            return Err(unsupported);
        }
        for &point in section {
            if dot(sub(self.points[point as usize], anchor), normal).abs()
                > self.policy.max_planar_deviation
            {
                return Err(unsupported);
            }
        }
        for &face in faces {
            if dot(self.plane(face)?.normal, normal) < 1.0 - 1e-12 {
                return Err(unsupported);
            }
            let boundary_point = |neighbor: VertexId,
                                  twin_direction: (VertexId, VertexId)|
             -> Result<Option<u32>, RoundError> {
                let adjacent = self
                    .half_edge_of
                    .get(&twin_direction)
                    .and_then(|&edge| self.mesh.face(edge));
                let point = if adjacent == Some(edge.left) {
                    section[0]
                } else if adjacent == Some(edge.right) {
                    *section.last().expect("section")
                } else {
                    return Ok(None);
                };
                let direction = sub(self.position(neighbor), anchor);
                let offset = sub(self.points[point as usize], anchor);
                let t = dot(offset, direction) / dot(direction, direction);
                if !(0.0..1.0).contains(&t)
                    || norm(sub(offset, scale(direction, t))) > self.policy.max_planar_deviation
                {
                    return Err(RoundError::ClearanceExceeded { face: face.index() });
                }
                Ok(Some(point))
            };
            let corners: Vec<_> = self
                .mesh
                .face_loop(face)
                .filter_map(|edge| self.mesh.to_vertex(edge))
                .collect();
            let index = corners
                .iter()
                .position(|&corner| corner == vertex)
                .expect("incident face");
            let previous = corners[(index + corners.len() - 1) % corners.len()];
            let next = corners[(index + 1) % corners.len()];
            let incoming = boundary_point(previous, (vertex, previous))?;
            let outgoing = boundary_point(next, (next, vertex))?;
            self.register(face, vertex, Subst::EndCap { incoming, outgoing })?;
        }
        for pair in section.windows(2) {
            let mut ends = [pair[0], pair[1]];
            let a = sub(self.points[ends[0] as usize], anchor);
            let b = sub(self.points[ends[1] as usize], anchor);
            if dot(cross(a, b), normal) < 0.0 {
                ends.reverse();
            }
            let emitted =
                ends.map(|point| sub(promote(narrow(self.points[point as usize])), anchor));
            if dot(cross(emitted[0], emitted[1]), normal) <= 0.0 {
                return Err(RoundError::ClearanceExceeded {
                    face: owner.index(),
                });
            }
            self.emit_face(
                alloc::vec![Tok::Old(vertex), Tok::New(ends[0]), Tok::New(ends[1])],
                RoundFaceSource::Face(owner),
            )?;
            self.stats.patch_faces += 1;
        }
        Ok(())
    }
}

/// Whether a face enters the strict interior of a convex prism. Boundary
/// contact is expected at the source flanks and end caps. Clipping against a
/// small inward offset keeps roundoff on those planes from inventing overlap.
pub(super) fn enters_prism(
    points: &[[f64; 3]],
    planes: &[([f64; 3], f64); 5],
    tolerance: f64,
) -> bool {
    let mut polygon = points.to_vec();
    let mut clipped = Vec::new();
    for &(normal, distance) in planes {
        if polygon.is_empty() {
            return false;
        }
        clipped.clear();
        for index in 0..polygon.len() {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            let da = dot(normal, a) - distance - tolerance;
            let db = dot(normal, b) - distance - tolerance;
            if da > 0.0 {
                clipped.push(a);
            }
            if (da > 0.0) != (db > 0.0) {
                clipped.push(add(a, scale(sub(b, a), da / (da - db))));
            }
        }
        core::mem::swap(&mut polygon, &mut clipped);
    }
    !polygon.is_empty()
}

/// A concave addition can make an end loop cross itself without reversing
/// its Newell normal. Use the existing polygon validator before committing it.
pub(super) fn simple_polygon(points: &[[f64; 3]], normal: [f64; 3]) -> bool {
    let mut axis = 0;
    for candidate in 1..3 {
        if normal[candidate].abs() > normal[axis].abs() {
            axis = candidate;
        }
    }
    let mut axes = [(axis + 1) % 3, (axis + 2) % 3];
    if normal[axis] < 0.0 {
        axes.swap(0, 1);
    }
    let projected: Vec<_> = points.iter().map(|p| [p[axes[0]], p[axes[1]]]).collect();
    triangulate(
        &PolygonInput {
            outer: &projected,
            holes: &[],
        },
        &TriParams::ear_clip(),
    )
    .is_ok()
}
