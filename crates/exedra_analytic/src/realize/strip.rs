// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Subdivided boundary rulings without subdividing every strip panel.

use super::{Body, Chains, Coedge, FaceId, RealizationError, VertexSource, distance, emit};
use crate::body::LoopId;
use alloc::vec::Vec;
use exedra_math::{Placement3, add, cross, dot, norm, scale, sub};

#[derive(Clone, Copy)]
struct Side {
    ring: LoopId,
    first: usize,
    count: usize,
    reverse: bool,
}
impl Side {
    fn coedge<F, E>(self, body: &Body<F, E>, i: usize) -> Coedge {
        let ring = &body
            .boundary_loop(self.ring)
            .expect("validated loop")
            .coedges;
        ring[(self.first + i) % ring.len()]
    }
}
/// Ruling at the beginning/end of oriented rim traversal, respectively.
#[derive(Clone, Copy)]
pub(super) struct Sides {
    ring: LoopId,
    a: u32,
    b: u32,
}
#[derive(Clone, Copy)]
pub(super) struct Sample {
    pub vertex: u32,
    pub fraction: f64,
}
impl Sides {
    fn records<F, E>(self, body: &Body<F, E>) -> [Side; 2] {
        let len = body
            .boundary_loop(self.ring)
            .expect("validated loop")
            .coedges
            .len();
        let (a, b) = (self.a as usize, self.b as usize);
        [
            Side {
                ring: self.ring,
                first: (b + 1) % len,
                count: (a + len - b - 1) % len,
                reverse: true,
            },
            Side {
                ring: self.ring,
                first: (a + 1) % len,
                count: (b + len - a - 1) % len,
                reverse: false,
            },
        ]
    }
    pub(super) fn prepare<F, E>(
        body: &Body<F, E>,
        id: FaceId,
        a: Coedge,
        b: Coedge,
        slack: f64,
    ) -> Result<Option<Self>, RealizationError> {
        let face = &body.faces()[id.index()];
        if face.loops.len() != 1 {
            return Ok(None);
        }
        let ring = face.loops[0];
        let uses = &body.boundary_loop(ring).expect("validated loop").coedges;
        if uses.len() == 4 {
            return Ok(None);
        }
        let refuse = || RealizationError::UnsupportedSurfaceTrim { face: id };
        let ai = uses.iter().position(|&c| c == a).ok_or_else(refuse)?;
        let bi = uses.iter().position(|&c| c == b).ok_or_else(refuse)?;
        let sides = Self {
            ring,
            a: u32::try_from(ai).map_err(|_| refuse())?,
            b: u32::try_from(bi).map_err(|_| refuse())?,
        };
        for side in sides.records(body) {
            if side.count == 0 {
                return Err(refuse());
            }
            let first = side.coedge(body, 0);
            let last = side.coedge(body, side.count - 1);
            let start = body.coedge_vertices(first).ok_or_else(refuse)?[0];
            let end = body.coedge_vertices(last).ok_or_else(refuse)?[1];
            let lo = body.vertices()[start.index()];
            let delta = sub(body.vertices()[end.index()], lo);
            let length = norm(delta);
            if !length.is_finite() || length == 0.0 {
                return Err(refuse());
            }
            let unit = scale(delta, 1.0 / length);
            let mut previous = 0.0;
            for j in 0..side.count {
                let coedge = side.coedge(body, j);
                let edge = body.edge(coedge.edge).ok_or_else(refuse)?;
                if !matches!(edge.regular(), Some((crate::geometry::Curve::Line(_), _))) {
                    return Err(refuse());
                }
                let vertex = body.coedge_vertices(coedge).ok_or_else(refuse)?[1];
                let point = body.vertices()[vertex.index()];
                let fraction = if j + 1 == side.count {
                    1.0
                } else {
                    dot(sub(point, lo), unit) / length
                };
                let gap = distance(point, add(lo, scale(delta, fraction)));
                if !fraction.is_finite()
                    || fraction <= previous
                    || fraction > 1.0
                    || !gap.is_finite()
                    || gap > slack
                {
                    return Err(refuse());
                }
                previous = fraction;
            }
        }
        Ok(Some(sides))
    }
    pub(super) fn extra_samples<F, E>(self, body: &Body<F, E>, counts: &[u32]) -> usize {
        self.records(body)
            .into_iter()
            .map(|side| {
                (0..side.count)
                    .map(|i| counts[side.coedge(body, i).edge.index()] as usize)
                    .sum::<usize>()
                    .saturating_sub(1)
            })
            .sum()
    }
    pub(super) fn samples<F, E>(
        self,
        body: &Body<F, E>,
        id: FaceId,
        chains: &Chains,
        sources: &[VertexSource],
        emitted: &[[f32; 3]],
        placement: Placement3,
        left: [u32; 2],
        right: [u32; 2],
        residual: &mut f64,
    ) -> Result<[Vec<Sample>; 2], RealizationError> {
        let mut result = [Vec::new(), Vec::new()];
        let mut extra_gap = 0.0_f64;
        for (i, side) in self.records(body).into_iter().enumerate() {
            let capacity = (0..side.count)
                .map(|j| chains.get(side.coedge(body, j).edge).len() - 1)
                .sum::<usize>()
                .saturating_add(1);
            result[i].reserve(capacity);
            let mut last = None;
            for j in 0..side.count {
                let c = side.coedge(body, j);
                let chain = chains.get(c.edge);
                for k in 0..chain.len() - 1 {
                    let vertex = chain[if c.reversed { chain.len() - 1 - k } else { k }];
                    result[i].push(Sample {
                        vertex,
                        fraction: 0.0,
                    });
                }
                last = Some(chain[if c.reversed { 0 } else { chain.len() - 1 }]);
            }
            result[i].push(Sample {
                vertex: last.expect("nonempty side chain"),
                fraction: 1.0,
            });
            if side.reverse {
                result[i].reverse();
            }
            let endpoints = if i == 0 { left } else { right };
            let native = |index: u32| match sources[index as usize] {
                VertexSource::Vertex(v) => body.vertices()[v.index()],
                VertexSource::Edge { edge, parameter } => body
                    .edge_point(edge, parameter)
                    .expect("sampled regular edge"),
                VertexSource::Face { .. } => unreachable!("boundary chain source"),
            };
            let lo = native(endpoints[0]);
            let delta = sub(native(endpoints[1]), lo);
            let length = norm(delta);
            let unit = scale(delta, 1.0 / length);
            let n = result[i].len();
            let placed_lo = emitted[endpoints[0] as usize].map(f64::from);
            let placed_delta = sub(emitted[endpoints[1] as usize].map(f64::from), placed_lo);
            for (j, sample) in result[i].iter_mut().enumerate() {
                sample.fraction = if j == 0 {
                    0.0
                } else if j + 1 == n {
                    1.0
                } else {
                    dot(sub(native(sample.vertex), lo), unit) / length
                };
                if !sample.fraction.is_finite() || !(0.0..=1.0).contains(&sample.fraction) {
                    return Err(RealizationError::DegenerateFace { face: id });
                }
                let expected = add(placed_lo, scale(placed_delta, sample.fraction));
                let gap = distance(emitted[sample.vertex as usize].map(f64::from), expected);
                if !gap.is_finite() {
                    return Err(RealizationError::DegenerateFace { face: id });
                }
                extra_gap = extra_gap
                    .max(gap + super::placement_roundoff(placement, native(sample.vertex)));
            }
        }
        *residual += extra_gap;
        Ok(result)
    }
}

pub(super) fn emit_panel(
    left: &[Sample],
    right: &[Sample],
    id: FaceId,
    region: u32,
    normal: [f64; 3],
    emitted: &[[f32; 3]],
    triangles: &mut Vec<[u32; 3]>,
    regions: &mut Vec<u32>,
    faces: &mut Vec<FaceId>,
) -> Result<(), RealizationError> {
    let (mut l, mut r) = (0, 0);
    while l + 1 < left.len() || r + 1 < right.len() {
        let advance_left = r + 1 == right.len()
            || (l + 1 < left.len() && left[l + 1].fraction < right[r + 1].fraction);
        let triangle = if advance_left {
            let t = [left[l].vertex, right[r].vertex, left[l + 1].vertex];
            l += 1;
            t
        } else {
            let t = [left[l].vertex, right[r].vertex, right[r + 1].vertex];
            r += 1;
            t
        };
        let p = triangle.map(|j| emitted[j as usize].map(f64::from));
        if dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), normal) <= 0.0 {
            return Err(RealizationError::DegenerateFace { face: id });
        }
        emit(triangles, triangle, id, region, emitted, regions, faces)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{Edge, TrimCurve, ValidationPolicy, VertexId};
    use crate::geometry::ParameterRange;
    use crate::realize::RealizationPolicy;
    #[test]
    fn four_edge_seam_chain_keeps_every_split_vertex_with_only_six_extra_triangles() {
        let original = crate::body::trims::tests::cylinder_parts();
        let baseline = Body::try_from_parts(original.clone(), &ValidationPolicy { max_gap: 1e-10 })
            .unwrap()
            .realize(Placement3::IDENTITY, &RealizationPolicy::default())
            .unwrap();
        let mut p = original;
        let curve = p.edges[2].regular().unwrap().0.clone();
        p.edges.truncate(2);
        let tau = core::f64::consts::TAU;
        let mut right = Vec::new();
        let mut left = Vec::new();
        for j in 0..4_u32 {
            let (a, b) = (f64::from(j) / 4.0, f64::from(j + 1) / 4.0);
            let start = if j == 0 { VertexId(0) } else { VertexId(j + 1) };
            let end = if j == 3 {
                VertexId(1)
            } else {
                let id = VertexId(crate::usize_to_u32(p.vertices.len()));
                p.vertices.push([2.0, 0.0, 3.0 * b]);
                id
            };
            p.edges.push(Edge::new(
                curve.clone(),
                ParameterRange::new(a, b).unwrap(),
                start,
                end,
                (),
            ));
            right.push(TrimCurve::Line {
                start: [tau, 3.0 * a],
                end: [tau, 3.0 * b],
            });
            left.push(TrimCurve::Line {
                start: [0.0, 3.0 * a],
                end: [0.0, 3.0 * b],
            });
        }
        let mut uses = alloc::vec![Coedge {
            edge: crate::body::EdgeId(0),
            reversed: false
        }];
        uses.extend((2..6).map(|i| Coedge {
            edge: crate::body::EdgeId(i),
            reversed: false,
        }));
        uses.push(Coedge {
            edge: crate::body::EdgeId(1),
            reversed: true,
        });
        uses.extend((2..6).rev().map(|i| Coedge {
            edge: crate::body::EdgeId(i),
            reversed: true,
        }));
        p.loops[0].coedges = uses;
        let mut trims = alloc::vec![TrimCurve::Line {
            start: [0.0, 0.0],
            end: [tau, 0.0]
        }];
        trims.extend(right);
        trims.push(TrimCurve::Line {
            start: [0.0, 3.0],
            end: [tau, 3.0],
        });
        trims.extend(left.into_iter().rev());
        p.trims[0].curves = trims;
        let body = Body::try_from_parts(p, &ValidationPolicy { max_gap: 1e-10 }).unwrap();
        let output = body
            .realize(Placement3::IDENTITY, &RealizationPolicy::default())
            .unwrap();
        assert_eq!(output.mesh_faces.len(), baseline.mesh_faces.len() + 6);
        assert_eq!(
            output.report.edge_segments[0],
            baseline.report.edge_segments[0]
        );
        assert_eq!(output.report.work.trim_refinement_batches, 0);
        assert!(output.report.within_tolerance());
        assert!(output.mesh.validate_deep().is_empty());
        assert!(output.mesh.boundary_loops().unwrap().is_empty());
        for i in 2..5 {
            assert!(
                output
                    .mesh_vertices
                    .iter()
                    .any(|(_, s)| *s == VertexSource::Vertex(VertexId(i)))
            );
        }
    }
}
