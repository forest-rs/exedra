// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rectangular native patches with authoritative shared boundary chains.

use super::{
    Chains, Groups, Patch, RealizationError, RealizationPolicy, VertexSource, distance, emit,
};
use crate::body::{Body, Face, FaceId, Sense};
use crate::geometry::{Curve, CurveSpan, ParameterRange, Surface, Torus, vector};
use alloc::vec::Vec;
use exedra_math::{Placement3, cross, dot, narrow, sub};

#[derive(Copy, Clone)]
struct Side {
    edge: crate::body::EdgeId,
    reverse: bool,
}
impl Side {
    fn at(self, chains: &Chains, i: usize) -> u32 {
        let chain = chains.get(self.edge);
        chain[if self.reverse { chain.len() - 1 - i } else { i }]
    }
}

#[derive(Copy, Clone)]
pub(super) struct GridPatch {
    // Bottom, right, top, left. Each runs in increasing U or V, separately
    // from the face loop's oriented traversal. Periodic sides may share an edge.
    sides: [Side; 4],
    alignment: f64,
}

#[derive(Copy, Clone)]
struct Phase {
    parameter: f64,
    middle_parameter: f64,
    point: [f64; 2],
    middle: [f64; 2],
}

struct RuledSample {
    ends: [[f64; 3]; 2],
    mid_tangents: [[f64; 3]; 2],
    mid_delta: [f64; 3],
}

/// One realization-local phase schedule and two reusable vertex rows.
#[derive(Default)]
pub(super) struct GridScratch {
    key: Option<([u64; 2], usize, bool)>,
    phases: Vec<Phase>,
    ruled: Vec<RuledSample>,
    previous: Vec<u32>,
    row: Vec<u32>,
}
impl GridScratch {
    fn prepare(&mut self, range: ParameterRange, n: usize, angular: bool) {
        let key = (range.bounds().map(f64::to_bits), n, angular);
        if self.key != Some(key) {
            self.phases.clear();
            self.phases.reserve(n + 1);
            for i in 0..=n {
                let parameter = range.at(i as f64 / n as f64).expect("grid fraction");
                let middle = if i < n {
                    0.5 * (parameter + range.at((i + 1) as f64 / n as f64).expect("grid fraction"))
                } else {
                    parameter
                };
                self.phases.push(Phase {
                    parameter,
                    middle_parameter: middle,
                    point: if angular {
                        [libm::sin(parameter), libm::cos(parameter)]
                    } else {
                        [0.0; 2]
                    },
                    middle: if angular {
                        [libm::sin(middle), libm::cos(middle)]
                    } else {
                        [0.0; 2]
                    },
                });
            }
            self.key = Some(key);
        }
        self.previous.clear();
        self.previous.reserve(n + 1);
        self.row.clear();
        self.row.reserve(n + 1);
    }
}

impl GridPatch {
    pub(super) fn prepare<F, E>(body: &Body<F, E>, id: FaceId) -> Result<Self, RealizationError> {
        let face = &body.faces()[id.index()];
        if let Surface::Ruled(ruled) = &face.surface {
            let refuse = || RealizationError::UnsupportedSurfaceTrim { face: id };
            if face.domain.u != ParameterRange::UNIT
                || face.domain.v != ParameterRange::UNIT
                || face.loops.len() != 1
            {
                return Err(refuse());
            }
            let coedges = &body
                .boundary_loop(face.loops[0])
                .expect("validated loop")
                .coedges;
            if coedges.len() != 4 {
                return Err(refuse());
            }
            let mut sides = [None; 4];
            let mut alignment = 0.0_f64;
            let size = ruled
                .curves()
                .iter()
                .map(CurveSpan::magnitude)
                .fold(0.0, f64::max);
            let slack = 128.0 * crate::geometry::FRAME_TOLERANCE * size;
            for &coedge in coedges {
                let edge = &body.edges()[coedge.edge.index()];
                let (curve, edge_range) = edge.regular().ok_or_else(refuse)?;
                let mut found = None;
                for (slot, side) in sides.iter().enumerate() {
                    if side.is_some() {
                        continue;
                    }
                    let reverse = coedge.reversed ^ (slot >= 2) ^ (face.sense == Sense::Reverse);
                    let actual =
                        CurveSpan::new(curve.clone(), edge_range, reverse).map_err(|_| refuse())?;
                    let bound = ruled.boundary(slot).ok_or_else(refuse)?.alignment(&actual);
                    if bound <= slack {
                        found = Some((
                            slot,
                            Side {
                                edge: coedge.edge,
                                reverse,
                            },
                            bound,
                        ));
                        break;
                    }
                }
                let Some((slot, side, bound)) = found else {
                    return Err(refuse());
                };
                sides[slot] = Some(side);
                alignment = alignment.max(bound);
            }
            let result = Self {
                sides: sides.map(|s| s.expect("four unique slots")),
                alignment,
            };
            result.check_corners(body, id)?;
            return Ok(result);
        }
        let Surface::Torus(torus) = &face.surface else {
            unreachable!()
        };
        let refuse = || RealizationError::UnsupportedSurfaceTrim { face: id };
        if face.loops.len() != 1 {
            return Err(refuse());
        }
        let coedges = &body
            .boundary_loop(face.loops[0])
            .expect("validated loop")
            .coedges;
        if coedges.len() != 4 {
            return Err(refuse());
        }
        let mut sides = [None; 4];
        let mut alignment = 0.0_f64;
        let size = torus
            .frame()
            .rows
            .iter()
            .map(|r| r[3].abs())
            .fold(torus.major_radius() + torus.minor_radius(), f64::max);
        // Grid eligibility cannot be relaxed by a caller's support-gap policy.
        // Any admitted numerical alignment error is nevertheless reported.
        let slack = 128.0 * crate::geometry::FRAME_TOLERANCE * size;
        for &coedge in coedges {
            let mut found = None;
            for (slot, side) in sides.iter().enumerate() {
                if side.is_some() {
                    continue;
                }
                let reverse = coedge.reversed ^ (slot >= 2) ^ (face.sense == Sense::Reverse);
                let edge = &body.edges()[coedge.edge.index()];
                let (curve, edge_range) = edge.regular().ok_or_else(refuse)?;
                let Curve::Circle(circle) = &curve else {
                    return Err(refuse());
                };
                let range = if slot.is_multiple_of(2) {
                    face.domain.u
                } else {
                    face.domain.v
                };
                if range.span() > core::f64::consts::TAU
                    || (edge_range.span() - range.span()).abs() > 64.0 * f64::EPSILON * range.span()
                {
                    continue;
                }
                let phase = edge_range.bounds()[usize::from(reverse)];
                let (s, c) = (libm::sin(phase), libm::cos(phase));
                let actual = [
                    circle.frame().rows.map(|r| r[3]),
                    vector(
                        circle.frame(),
                        [circle.radius() * c, circle.radius() * s, 0.0],
                    ),
                    vector(
                        circle.frame(),
                        [-circle.radius() * s, circle.radius() * c, 0.0],
                    )
                    .map(|v| if reverse { -v } else { v }),
                ];
                let ideal = coefficients(torus, face, slot);
                // Compare the full C + A*cos(t) + B*sin(t), not a few points.
                let bound = actual
                    .into_iter()
                    .zip(ideal)
                    .map(|(a, b)| distance(a, b))
                    .sum::<f64>()
                    + circle.radius() * (edge_range.span() - range.span()).abs()
                    + 8.0
                        * f64::EPSILON
                        * circle.radius()
                        * (edge_range
                            .bounds()
                            .iter()
                            .map(|v| v.abs())
                            .fold(range.span(), f64::max));
                if bound <= slack {
                    found = Some((
                        slot,
                        Side {
                            edge: coedge.edge,
                            reverse,
                        },
                        bound,
                    ));
                    break;
                }
            }
            let Some((slot, side, bound)) = found else {
                return Err(refuse());
            };
            sides[slot] = Some(side);
            alignment = alignment.max(bound);
        }
        let sides = sides.map(|s| s.expect("four unique slots"));
        let result = Self { sides, alignment };
        result.check_corners(body, id)?;
        Ok(result)
    }

    fn check_corners<F, E>(self, body: &Body<F, E>, id: FaceId) -> Result<(), RealizationError> {
        let endpoint = |side: Side, last: bool| {
            let edge = &body.edges()[side.edge.index()];
            if side.reverse ^ last {
                edge.end
            } else {
                edge.start
            }
        };
        for (a, a_last, b, b_last) in [
            (0, false, 3, false),
            (0, true, 1, false),
            (2, false, 3, true),
            (2, true, 1, true),
        ] {
            if endpoint(self.sides[a], a_last) != endpoint(self.sides[b], b_last) {
                return Err(RealizationError::UnsupportedSurfaceTrim { face: id });
            }
        }
        Ok(())
    }

    pub(super) fn join(self, groups: &mut Groups) {
        for (a, b) in [(0, 2), (1, 3)] {
            groups.join(self.sides[a].edge.index(), self.sides[b].edge.index());
        }
    }
    fn intervals(self, counts: &[u32]) -> [u32; 2] {
        [
            counts[self.sides[0].edge.index()],
            counts[self.sides[1].edge.index()],
        ]
    }
    pub(super) fn interior_count(self, counts: &[u32]) -> usize {
        let [u, v] = self.intervals(counts);
        (u as usize - 1).saturating_mul(v as usize - 1)
    }
    pub(super) fn triangle_count(self, counts: &[u32]) -> usize {
        let [u, v] = self.intervals(counts);
        (u as usize).saturating_mul(v as usize).saturating_mul(2)
    }
    pub(super) fn deviation<F>(self, face: &Face<F>, counts: &[u32]) -> f64 {
        self.bound(face, self.intervals(counts)) + self.alignment
    }
    fn bound<F>(self, face: &Face<F>, [nu, nv]: [u32; 2]) -> f64 {
        if let Surface::Ruled(ruled) = &face.surface {
            // Eligibility already checked the unit square. Bound rounded
            // normalized divisions without traversing potentially huge counts
            // that a later output-budget check will refuse before allocation.
            let step = |n: u32| {
                if n == 1 {
                    1.0
                } else {
                    1.0 / f64::from(n) + 2.0 * f64::EPSILON
                }
            };
            return ruled.triangle_bound([step(nu), step(nv)]);
        }
        let Surface::Torus(t) = &face.surface else {
            unreachable!()
        };
        let u = max_step(face.domain.u, nu);
        let v = max_step(face.domain.v, nv);
        // Two 1D interpolation bounds (sup|second derivative| * h²/8)
        // plus bilinear-to-triangle error. The mixed corner difference is at
        // most r*du*dv, and its triangle coefficient is at most one quarter.
        let interpolation =
            ((t.major_radius() + t.minor_radius()) * u * u + t.minor_radius() * v * v) / 8.0
                + t.minor_radius() * u * v / 4.0;
        let scale = t
            .frame()
            .rows
            .iter()
            .map(|r| r[3].abs())
            .fold(t.major_radius() + t.minor_radius(), f64::max);
        interpolation + 2048.0 * f64::EPSILON * scale
    }

    pub(super) fn emit<F, E>(
        self,
        body: &Body<F, E>,
        id: FaceId,
        placement: Placement3,
        counts: &[u32],
        chains: &Chains,
        scratch: &mut GridScratch,
        points: &mut Vec<[f32; 3]>,
        sources: &mut Vec<VertexSource>,
        displacement: &mut f64,
        region: u32,
        triangles: &mut Vec<[u32; 3]>,
        regions: &mut Vec<u32>,
        faces: &mut Vec<FaceId>,
    ) -> Result<(), RealizationError> {
        let face = &body.faces()[id.index()];
        let torus = if let Surface::Torus(torus) = &face.surface {
            Some(torus)
        } else {
            None
        };
        let [nu, nv] = self.intervals(counts).map(|n| n as usize);
        scratch.prepare(face.domain.u, nu, torus.is_some());
        if let Surface::Ruled(ruled) = &face.surface {
            scratch.ruled.clear();
            scratch.ruled.reserve(nu + 1);
            for phase in &scratch.phases {
                let ends = [
                    ruled.curves()[0]
                        .evaluate(phase.parameter)
                        .expect("bounded grid")
                        .point,
                    ruled.curves()[1]
                        .evaluate(phase.parameter)
                        .expect("bounded grid")
                        .point,
                ];
                let mid = [
                    ruled.curves()[0]
                        .evaluate(phase.middle_parameter)
                        .expect("bounded grid"),
                    ruled.curves()[1]
                        .evaluate(phase.middle_parameter)
                        .expect("bounded grid"),
                ];
                scratch.ruled.push(RuledSample {
                    ends,
                    mid_tangents: mid.map(|e| e.first),
                    mid_delta: sub(mid[1].point, mid[0].point),
                });
            }
        }
        let GridScratch {
            phases,
            ruled,
            previous,
            row,
            ..
        } = scratch;
        for v in 0..=nv {
            row.clear();
            let vv = face
                .domain
                .v
                .at(v as f64 / nv as f64)
                .expect("grid fraction");
            let (sv, cv) = if torus.is_some() {
                (libm::sin(vv), libm::cos(vv))
            } else {
                (0.0, 0.0)
            };
            for (u, phase) in phases.iter().enumerate() {
                let vertex = if v == 0 {
                    self.sides[0].at(chains, u)
                } else if v == nv {
                    self.sides[2].at(chains, u)
                } else if u == 0 {
                    self.sides[3].at(chains, v)
                } else if u == nu {
                    self.sides[1].at(chains, v)
                } else {
                    let uv = [phase.parameter, vv];
                    let exact = if let Some(torus) = torus {
                        let [su, cu] = phase.point;
                        let radial = torus.major_radius() + torus.minor_radius() * cv;
                        torus.frame().transform_point([
                            radial * cu,
                            radial * su,
                            torus.minor_radius() * sv,
                        ])
                    } else {
                        exedra_math::lerp(ruled[u].ends[0], ruled[u].ends[1], vv)
                    };
                    let exact = placement.transform_point(exact);
                    let point = narrow(exact);
                    if point.iter().any(|p| !p.is_finite()) {
                        return Err(RealizationError::DegenerateFace { face: id });
                    }
                    *displacement = displacement.max(distance(exact, point.map(f64::from)));
                    let vertex = crate::usize_to_u32(points.len());
                    points.push(point);
                    sources.push(VertexSource::Face { face: id, uv });
                    vertex
                };
                row.push(vertex);
            }
            if v > 0 {
                let mid_v = 0.5
                    * (vv
                        + face
                            .domain
                            .v
                            .at((v - 1) as f64 / nv as f64)
                            .expect("grid fraction"));
                let (sv, cv) = if torus.is_some() {
                    (libm::sin(mid_v), libm::cos(mid_v))
                } else {
                    (0.0, 0.0)
                };
                for u in 0..nu {
                    let normal = if let Some(torus) = torus {
                        let [su, cu] = phases[u].middle;
                        vector(torus.frame(), [cv * cu, cv * su, sv])
                    } else {
                        exedra_math::normalize(cross(
                            exedra_math::lerp(
                                ruled[u].mid_tangents[0],
                                ruled[u].mid_tangents[1],
                                mid_v,
                            ),
                            ruled[u].mid_delta,
                        ))
                        .ok_or(RealizationError::DegenerateFace { face: id })?
                    }
                    .map(|v| v * face.sense.sign());
                    let normal = vector(placement, normal);
                    for mut tri in [
                        [previous[u], previous[u + 1], row[u + 1]],
                        [previous[u], row[u + 1], row[u]],
                    ] {
                        if face.sense == Sense::Reverse {
                            tri.swap(1, 2);
                        }
                        let p = tri.map(|j| points[j as usize].map(f64::from));
                        if dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), normal) <= 0.0 {
                            return Err(RealizationError::DegenerateFace { face: id });
                        }
                        emit(triangles, tri, id, region, points, regions, faces)?;
                    }
                }
            }
            core::mem::swap(row, previous);
        }
        Ok(())
    }
}

pub(super) fn refine<F, E>(
    body: &Body<F, E>,
    patches: &[Patch],
    counts: &mut [u32],
    groups: &mut Groups,
    policy: &RealizationPolicy,
) -> Result<(), RealizationError> {
    loop {
        let mut changed = false;
        for (i, patch) in patches.iter().enumerate() {
            let Patch::Grid(patch) = patch else {
                continue;
            };
            let face = &body.faces()[i];
            let n = patch.intervals(counts);
            if patch.bound(face, n) <= policy.tolerance * 0.75 {
                continue;
            }
            let mut best = None;
            let preferred = if let Surface::Ruled(r) = &face.surface {
                Some(r.refinement_axis(n, policy.tolerance * 0.75))
            } else {
                None
            };
            for axis in 0..2 {
                if n[axis] == policy.max_edge_segments {
                    continue;
                }
                let mut next = n;
                next[axis] = n[axis].saturating_mul(2).min(policy.max_edge_segments);
                let bound = patch.bound(face, next);
                if best.is_none_or(|(old, _, b)| {
                    if let Some(preferred) = preferred {
                        axis == preferred || (old != preferred && bound < b)
                    } else {
                        bound < b
                    }
                }) {
                    best = Some((axis, next[axis], bound));
                }
            }
            let Some((axis, mut next, candidate_bound)) = best else {
                return Err(RealizationError::SurfaceSampling {
                    face: FaceId(crate::usize_to_u32(i)),
                    intervals: n,
                    maximum: policy.max_edge_segments,
                });
            };
            if matches!(face.surface, Surface::Ruled(_))
                && candidate_bound <= policy.tolerance * 0.75
            {
                // Doubling finds a bracket cheaply. Resolve the final count
                // inside it so a barely failing wall does not allocate twice
                // as many shared rows as its conservative bound requires.
                let mut lo = n[axis];
                let mut hi = next;
                while hi - lo > 1 {
                    let mid = lo + (hi - lo) / 2;
                    let mut probe = n;
                    probe[axis] = mid;
                    if patch.bound(face, probe) <= policy.tolerance * 0.75 {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                next = hi;
            }
            counts[patch.sides[axis].edge.index()] = next;
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        groups.synchronize(counts);
    }
}

fn max_step(range: ParameterRange, n: u32) -> f64 {
    let mut previous = range.bounds()[0];
    let mut step = 0.0_f64;
    for i in 1..=n {
        let next = range
            .at(f64::from(i) / f64::from(n))
            .expect("grid fraction");
        step = step.max(next - previous);
        previous = next;
    }
    step
}

// Coefficients of the exact side curve in a phase measured from the lower
// parameter endpoint. This also rejects mismatched angular origins and winding.
fn coefficients<F>(t: &Torus, face: &Face<F>, slot: usize) -> [[f64; 3]; 3] {
    let (u, v) = if slot.is_multiple_of(2) {
        (
            face.domain.u.bounds()[0],
            face.domain.v.bounds()[usize::from(slot == 2)],
        )
    } else {
        (
            face.domain.u.bounds()[usize::from(slot == 1)],
            face.domain.v.bounds()[0],
        )
    };
    let (su, cu, sv, cv) = (libm::sin(u), libm::cos(u), libm::sin(v), libm::cos(v));
    let r = t.minor_radius();
    let major = t.major_radius();
    let (c, a, b) = if slot.is_multiple_of(2) {
        let rho = major + r * cv;
        (
            [0.0, 0.0, r * sv],
            [rho * cu, rho * su, 0.0],
            [-rho * su, rho * cu, 0.0],
        )
    } else {
        (
            [major * cu, major * su, 0.0],
            [r * cv * cu, r * cv * su, r * sv],
            [-r * sv * cu, -r * sv * su, r * cv],
        )
    };
    [
        t.frame().transform_point(c),
        vector(t.frame(), a),
        vector(t.frame(), b),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{
        BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, LoopId, ValidationPolicy, VertexId,
    };
    use crate::geometry::ParameterDomain;
    use alloc::vec;

    #[test]
    fn independent_ruled_patches_share_full_boundary_curves_and_refuse_incompatible_domains() {
        let p = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 1.0],
            [0.0, 2.0, 0.0],
        ];
        for reverse in [false, true] {
            let mut edges = Vec::new();
            for (a, b) in [(0, 1), (1, 2), (3, 2), (0, 3)] {
                edges.push(Edge {
                    geometry: crate::body::EdgeGeometry::Curve {
                        curve: Curve::line(p[a], sub(p[b], p[a])).unwrap(),
                        range: ParameterRange::UNIT,
                    },
                    start: VertexId(u32::try_from(a).unwrap()),
                    end: VertexId(u32::try_from(b).unwrap()),
                    role: (),
                });
            }
            let surface = Surface::ruled(
                CurveSpan::new(
                    edges[0].regular().unwrap().0.clone(),
                    ParameterRange::UNIT,
                    false,
                )
                .unwrap(),
                CurveSpan::new(
                    edges[2].regular().unwrap().0.clone(),
                    ParameterRange::UNIT,
                    false,
                )
                .unwrap(),
            )
            .unwrap();
            let mut coedges: Vec<_> = (0..4)
                .map(|i| Coedge {
                    edge: EdgeId(i),
                    reversed: i >= 2,
                })
                .collect();
            if reverse {
                coedges.reverse();
                for c in &mut coedges {
                    c.reversed = !c.reversed;
                }
            }
            let body = Body::try_from_parts(
                BodyParts {
                    trims: Vec::new(),
                    vertices: p.to_vec(),
                    edges,
                    loops: vec![BoundaryLoop { coedges }],
                    faces: vec![Face {
                        surface,
                        domain: ParameterDomain {
                            u: ParameterRange::UNIT,
                            v: ParameterRange::UNIT,
                        },
                        sense: if reverse {
                            Sense::Reverse
                        } else {
                            Sense::Forward
                        },
                        loops: vec![LoopId(0)],
                        role: (),
                    }],
                },
                &ValidationPolicy { max_gap: 1e-9 },
            )
            .unwrap();
            let output = body
                .realize(
                    Placement3::IDENTITY,
                    &RealizationPolicy {
                        tolerance: 0.005,
                        ..RealizationPolicy::default()
                    },
                )
                .unwrap();
            assert!(output.report.within_tolerance());
            assert!(output.mesh.validate_deep().is_empty());
            assert_eq!(output.mesh.boundary_loops().unwrap().len(), 1);
            for &(vertex, source) in &output.mesh_vertices {
                if let VertexSource::Face { face, uv } = source {
                    let exact = body.evaluate_support(face, uv).unwrap().point;
                    assert!(
                        distance(
                            exact,
                            output.mesh.vertex_position(vertex).unwrap().map(f64::from)
                        ) < 1e-6
                    );
                }
            }
            let mut parts = body.into_parts();
            parts.faces[0].domain.u = ParameterRange::new(0.0, 0.9).unwrap();
            let altered = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1.0 }).unwrap();
            assert!(matches!(
                altered.realize(
                    Placement3::IDENTITY,
                    &RealizationPolicy {
                        geometry_gap: 1.0,
                        ..RealizationPolicy::default()
                    }
                ),
                Err(RealizationError::UnsupportedSurfaceTrim { .. })
            ));
        }
    }

    #[test]
    fn a_single_torus_face_shares_both_periodic_seams_without_construction_history() {
        let range = ParameterRange::new(0.0, core::f64::consts::TAU).unwrap();
        for reverse in [false, true] {
            let meridian = Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0],
                [10.0, 0.0, 0.0],
            );
            let mut coedges = vec![
                Coedge {
                    edge: EdgeId(0),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(1),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(0),
                    reversed: true,
                },
                Coedge {
                    edge: EdgeId(1),
                    reversed: true,
                },
            ];
            if reverse {
                coedges.reverse();
                for c in &mut coedges {
                    c.reversed = !c.reversed;
                }
            }
            let body = Body::try_from_parts(
                BodyParts {
                    trims: Vec::new(),
                    vertices: vec![[12.0, 0.0, 0.0]],
                    edges: vec![
                        Edge {
                            geometry: crate::body::EdgeGeometry::Curve {
                                curve: Curve::circle(Placement3::IDENTITY, 12.0).unwrap(),
                                range,
                            },
                            start: VertexId(0),
                            end: VertexId(0),
                            role: (),
                        },
                        Edge {
                            geometry: crate::body::EdgeGeometry::Curve {
                                curve: Curve::circle(meridian, 2.0).unwrap(),
                                range,
                            },
                            start: VertexId(0),
                            end: VertexId(0),
                            role: (),
                        },
                    ],
                    loops: vec![BoundaryLoop { coedges }],
                    faces: vec![Face {
                        surface: Surface::torus(Placement3::IDENTITY, 10.0, 2.0).unwrap(),
                        domain: ParameterDomain { u: range, v: range },
                        sense: if reverse {
                            Sense::Reverse
                        } else {
                            Sense::Forward
                        },
                        loops: vec![LoopId(0)],
                        role: (),
                    }],
                },
                &ValidationPolicy { max_gap: 1e-9 },
            )
            .unwrap();
            assert!(body.is_closed());
            let output = body
                .realize(
                    Placement3::IDENTITY,
                    &RealizationPolicy {
                        tolerance: 0.1,
                        ..RealizationPolicy::default()
                    },
                )
                .unwrap();
            assert!(output.report.within_tolerance(), "{:?}", output.report);
            assert!(output.mesh.validate_deep().is_empty());
            assert!(output.mesh.boundary_loops().unwrap().is_empty());
            let [u, v] =
                core::array::from_fn::<_, 2, _>(|i| output.report.edge_segments[i] as usize);
            assert_eq!(
                output.mesh.vertices().count(),
                u * v,
                "periodic geometry has no duplicate seam vertices"
            );
            assert_eq!(output.mesh.faces().count(), 2 * u * v);
        }
    }
}
