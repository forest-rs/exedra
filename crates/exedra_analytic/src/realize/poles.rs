// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Full-turn spherical patches and pointed conical fans. Collapsed boundaries
//! keep one physical vertex; angular schedules belong to the face, not the edge.

use super::{Chains, RealizationError, RealizationPolicy, VertexSource, distance, emit};
use crate::body::{Body, Coedge, EdgeId, Face, FaceId, Sense, VertexId};
use crate::geometry::{Curve, FRAME_TOLERANCE, ParameterRange, Surface, vector};
use alloc::vec::Vec;
use exedra_math::{Placement3, cross, dot, narrow, sub};
mod rings;
use rings::Band;

#[derive(Copy, Clone)]
struct Side {
    edge: EdgeId,
    reverse: bool,
    pole: bool,
}
impl Side {
    fn at(self, chains: &Chains, i: usize) -> u32 {
        let chain = chains.get(self.edge);
        if self.pole {
            chain[0]
        } else {
            chain[if self.reverse { chain.len() - 1 - i } else { i }]
        }
    }
}
#[derive(Copy, Clone)]
pub(super) struct PolePatch {
    sides: [Side; 4],
    minimum: [u32; 2],
    alignment: f64,
    spherical: bool,
    radius: f64,
    radial: Option<f64>,
}
#[derive(Default)]
pub(super) struct Scratch {
    key: Option<([u64; 2], u32)>,
    phases: Vec<(f64, f64, f64)>,
    previous: Vec<u32>,
    row: Vec<u32>,
}
/// Forward, face-local pole direction for one emitted corner. Resolved to a
/// mesh `CornerId` only after the mesh builder assigns its authoritative IDs.
pub(super) struct PoleSeed {
    pub(super) triangle: usize,
    pub(super) vertex: u32,
    pub(super) native_vertex: VertexId,
    pub(super) face: FaceId,
    pub(super) uv: [f64; 2],
}
fn step(range: ParameterRange, n: u32) -> f64 {
    range.span() / f64::from(n)
        + 8.0
            * f64::EPSILON
            * range
                .bounds()
                .into_iter()
                .map(f64::abs)
                .fold(range.span(), f64::max)
}
fn sin_cos(v: f64) -> (f64, f64) {
    if v == core::f64::consts::FRAC_PI_2 {
        (1.0, 0.0)
    } else if v == -core::f64::consts::FRAC_PI_2 {
        (-1.0, 0.0)
    } else {
        (libm::sin(v), libm::cos(v))
    }
}
fn world_coefficients(frame: Placement3, coeff: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [
        frame.transform_point(coeff[0]),
        vector(frame, coeff[1]),
        vector(frame, coeff[2]),
    ]
}
fn ideal<F>(face: &Face<F>, slot: usize) -> Option<([[f64; 3]; 3], ParameterRange)> {
    let (u, v) = (face.domain.u.bounds(), face.domain.v.bounds());
    let horizontal = slot.is_multiple_of(2);
    let (range, fixed) = if horizontal {
        (face.domain.u, v[usize::from(slot == 2)])
    } else {
        (face.domain.v, u[usize::from(slot == 1)])
    };
    let coefficients = match &face.surface {
        Surface::Sphere(sphere) => {
            let r = sphere.radius();
            if horizontal {
                let (sv, cv) = sin_cos(fixed);
                let (su, cu) = sin_cos(range.bounds()[0]);
                [
                    [0.0, 0.0, r * sv],
                    [r * cv * cu, r * cv * su, 0.0],
                    [-r * cv * su, r * cv * cu, 0.0],
                ]
            } else {
                let (su, cu) = sin_cos(fixed);
                let (sv, cv) = sin_cos(range.bounds()[0]);
                [
                    [0.0; 3],
                    [r * cv * cu, r * cv * su, r * sv],
                    [-r * sv * cu, -r * sv * su, r * cv],
                ]
            }
        }
        Surface::Cone(cone) => {
            if !horizontal {
                return None;
            }
            let r = cone.radius_at(fixed)?;
            let (su, cu) = sin_cos(range.bounds()[0]);
            [
                [0.0, 0.0, fixed],
                [r * cu, r * su, 0.0],
                [-r * su, r * cu, 0.0],
            ]
        }
        _ => return None,
    };
    Some((
        world_coefficients(face.surface.frame(), coefficients),
        range,
    ))
}
fn boundary_point<F>(face: &Face<F>, uv: [f64; 2]) -> Option<[f64; 3]> {
    match &face.surface {
        Surface::Cone(c) if Some(uv[1]) == c.apex_parameter() => c.apex_point(),
        _ => Some(face.surface.evaluate(uv)?.point),
    }
}

impl PolePatch {
    pub(super) fn prepare<F, E>(
        body: &Body<F, E>,
        id: FaceId,
        policy: &RealizationPolicy,
    ) -> Result<Self, RealizationError> {
        let face = &body.faces()[id.index()];
        let refuse = || RealizationError::UnsupportedSurfaceTrim { face: id };
        let spherical = matches!(face.surface, Surface::Sphere(_));
        if !spherical && !matches!(face.surface, Surface::Cone(_)) {
            return Err(refuse());
        }
        if face.loops.len() != 1 || face.domain.u.span() != core::f64::consts::TAU {
            return Err(refuse());
        }
        let coedges = &body
            .boundary_loop(face.loops[0])
            .ok_or_else(refuse)?
            .coedges;
        if coedges.len() != 4 {
            return Err(refuse());
        }
        let radius = match &face.surface {
            Surface::Sphere(s) => s.radius(),
            Surface::Cone(c) => c.radius(),
            _ => return Err(refuse()),
        };
        let mut magnitude = face
            .surface
            .frame()
            .rows
            .iter()
            .map(|r| r[3].abs())
            .fold(radius, f64::max);
        if !spherical {
            magnitude = magnitude.max(
                face.domain
                    .v
                    .bounds()
                    .into_iter()
                    .map(f64::abs)
                    .fold(0.0, f64::max),
            );
        }
        let arithmetic = (512.0 * (f64::EPSILON * magnitude + f64::from_bits(1))).next_up();
        let slack = (256.0 * FRAME_TOLERANCE * magnitude + arithmetic).next_up();
        let mut found = None;
        for start in 0..4 {
            let ordered: [Coedge; 4] = core::array::from_fn(|slot| {
                let index = if face.sense == Sense::Reverse {
                    (start + 4 - slot) % 4
                } else {
                    (start + slot) % 4
                };
                coedges[index]
            });
            if ordered[1].edge != ordered[3].edge || ordered[1].reversed == ordered[3].reversed {
                continue;
            }
            let mut sides = [None; 4];
            let mut alignment = arithmetic;
            let mut valid = true;
            for (slot, coedge) in ordered.into_iter().enumerate() {
                let edge = &body.edges()[coedge.edge.index()];
                let reverse = coedge.reversed ^ (slot >= 2) ^ (face.sense == Sense::Reverse);
                let pole = edge.is_collapsed();
                let gap = if pole {
                    if !slot.is_multiple_of(2) {
                        valid = false;
                        break;
                    }
                    let v = face.domain.v.bounds()[usize::from(slot == 2)];
                    let pole_allowed = match &face.surface {
                        Surface::Sphere(_) => v.abs() == core::f64::consts::FRAC_PI_2,
                        Surface::Cone(c) => Some(v) == c.apex_parameter(),
                        _ => false,
                    };
                    if !pole_allowed {
                        valid = false;
                        break;
                    }
                    let Some(point) = boundary_point(face, [face.domain.u.bounds()[0], v]) else {
                        valid = false;
                        break;
                    };
                    distance(body.vertices()[edge.start.index()], point) + arithmetic
                } else if !spherical && !slot.is_multiple_of(2) {
                    let Some((Curve::Line(_), _)) = edge.regular() else {
                        valid = false;
                        break;
                    };
                    let u = face.domain.u.bounds()[usize::from(slot == 1)];
                    let mut gap = 0.0_f64;
                    for (j, v) in face.domain.v.bounds().into_iter().enumerate() {
                        let Some(expected) = boundary_point(face, [u, v]) else {
                            valid = false;
                            break;
                        };
                        let Some(actual) = body.edge_point(
                            coedge.edge,
                            if reverse { 1.0 - j as f64 } else { j as f64 },
                        ) else {
                            valid = false;
                            break;
                        };
                        gap = gap.max(distance(actual, expected));
                    }
                    gap + arithmetic
                } else {
                    let Some((Curve::Circle(circle), range)) = edge.regular() else {
                        valid = false;
                        break;
                    };
                    let Some((expected, span)) = ideal(face, slot) else {
                        valid = false;
                        break;
                    };
                    let phase = range.bounds()[usize::from(reverse)];
                    let (s, c) = sin_cos(phase);
                    let actual = world_coefficients(
                        circle.frame(),
                        [
                            [0.0; 3],
                            [circle.radius() * c, circle.radius() * s, 0.0],
                            [-circle.radius() * s, circle.radius() * c, 0.0]
                                .map(|x| if reverse { -x } else { x }),
                        ],
                    );
                    actual
                        .into_iter()
                        .zip(expected)
                        .map(|(a, b)| distance(a, b))
                        .sum::<f64>()
                        + circle.radius() * (range.span() - span.span()).abs()
                        + arithmetic
                };
                if !gap.is_finite() || gap > slack {
                    valid = false;
                    break;
                }
                alignment = alignment.max(gap);
                sides[slot] = Some(Side {
                    edge: coedge.edge,
                    reverse,
                    pole,
                });
            }
            if valid {
                let sides = sides.map(|s| s.expect("four matched boundary slots"));
                if !sides[0].pole && !sides[2].pole {
                    continue;
                }
                found = Some((sides, alignment));
                break;
            }
        }
        let (sides, alignment) = found.ok_or_else(refuse)?;
        let mut patch = Self {
            sides,
            minimum: [
                policy.min_arc_segments,
                if spherical && sides[0].pole && sides[2].pole {
                    2
                } else {
                    1
                },
            ],
            alignment,
            spherical,
            radius,
            radial: None,
        };
        if spherical {
            loop {
                if patch.bound(face, patch.minimum) <= policy.tolerance * 0.75 {
                    break;
                }
                let steps = [
                    step(face.domain.u, patch.minimum[0]),
                    step(face.domain.v, patch.minimum[1]),
                ];
                let mut axis = usize::from(steps[1] > steps[0]);
                if patch.minimum[axis] >= policy.max_edge_segments {
                    axis = 1 - axis;
                }
                if patch.minimum[axis] >= policy.max_edge_segments {
                    return Err(RealizationError::SurfaceSampling {
                        face: id,
                        intervals: patch.minimum,
                        maximum: policy.max_edge_segments,
                    });
                }
                patch.minimum[axis] = patch.minimum[axis]
                    .saturating_mul(2)
                    .min(policy.max_edge_segments);
            }
        }
        Ok(patch)
    }
    pub(super) fn synchronize(self, counts: &mut [u32]) {
        let n = self.intervals(counts);
        for (slot, side) in self.sides.into_iter().enumerate() {
            if !side.pole {
                counts[side.edge.index()] = n[slot % 2];
            }
        }
    }
    fn intervals(self, counts: &[u32]) -> [u32; 2] {
        let mut result = self.minimum;
        for (slot, side) in self.sides.into_iter().enumerate() {
            if !side.pole {
                result[slot % 2] = result[slot % 2].max(counts[side.edge.index()]);
            }
        }
        result
    }
    fn ring_segments(j: u32, n: [u32; 2]) -> u32 {
        if j == 0 || j == n[1] {
            return 0;
        }
        let v =
            -core::f64::consts::FRAC_PI_2 + f64::from(j) / f64::from(n[1]) * core::f64::consts::PI;
        let target = f64::from(n[0]) * libm::cos(v).max(0.0);
        let mut segments = 4;
        while segments < n[0] && f64::from(segments) < target {
            segments *= 2;
        }
        segments
    }
    /// A both-pole, one-loop full sphere admits a degree-one radial cover.
    /// Adjacent rows share an angular origin and differ by at most a factor of
    /// two. The zipper triangulates the parameter sphere with single-cycle
    /// vertex links. Its north fan turns once; all other triangles span <PI in
    /// longitude. Certified positive natural plane offsets preserve degree one.
    /// This proof does not apply to trimmed caps or a guessed spherical family.
    pub(super) fn prepare_radial_sampling<F>(
        &mut self,
        face: &Face<F>,
        counts: &[u32],
        policy: &RealizationPolicy,
    ) -> Result<(), RealizationError> {
        self.radial = None;
        let n = self.intervals(counts);
        if !self.spherical
            || !self.sides[0].pole
            || !self.sides[2].pole
            || n[0] < 4
            || !n[0].is_power_of_two()
        {
            return Ok(());
        }
        let limit = 0.75 * policy.tolerance;
        let half = n[0] / 2;
        if half >= policy.min_arc_segments && half >= 4 {
            // Both angular boundary rows are collapsed, so no ordinary edge
            // owns this U count. Try one coarser aligned schedule against the
            // same surface-error allocation; shared meridian samples stay unchanged.
            let mut candidate = *self;
            candidate.minimum[0] = half;
            candidate.prepare_radial_candidate(face, counts, policy, limit)?;
            if candidate.radial.is_some() {
                *self = candidate;
                return Ok(());
            }
        }
        self.prepare_radial_candidate(face, counts, policy, limit)
    }

    fn prepare_radial_candidate<F>(
        &mut self,
        face: &Face<F>,
        counts: &[u32],
        policy: &RealizationPolicy,
        limit: f64,
    ) -> Result<(), RealizationError> {
        let n = self.intervals(counts);
        let budget = |required: u64| -> Result<(), RealizationError> {
            if required > u64::from(policy.max_vertices) {
                return Err(RealizationError::Budget {
                    storage: "vertices",
                    required: usize::try_from(required).map_err(|_| {
                        RealizationError::AddressSpace {
                            storage: "vertices",
                        }
                    })?,
                    maximum: policy.max_vertices,
                });
            }
            Ok(())
        };
        budget(4 * u64::from(n[1] - 1) + 2)?;
        let mut vertices = 2_u64;
        let mut lower = 0;
        let mut unit_gap = 0.0_f64;
        let latitude = |j| {
            -core::f64::consts::FRAC_PI_2 + f64::from(j) / f64::from(n[1]) * core::f64::consts::PI
        };
        // The certificate rotates the common longitude origin to zero. Bound
        // the actual range interpolation before that rotation as well as the
        // canonical TAU/PI constants, rather than assuming a zero-origin chart.
        let angular_magnitude = face
            .domain
            .u
            .bounds()
            .into_iter()
            .map(f64::abs)
            .fold(core::f64::consts::TAU, f64::max);
        let angle_error = (32.0 * f64::EPSILON * angular_magnitude).next_up();
        for j in 1..=n[1] {
            let upper = Self::ring_segments(j, n);
            if j < n[1] {
                vertices += u64::from(upper);
                budget(vertices)?;
            }
            if lower != 0
                && upper != 0
                && u64::from(lower.max(upper)) > 2 * u64::from(lower.min(upper))
            {
                return Ok(());
            }
            let shapes = if lower == 0 || upper == 0 {
                1
            } else if lower == upper {
                2
            } else {
                3
            };
            // Rotational symmetry leaves only 1–3 distinct triangles per band,
            // avoiding a duplicate whole-mesh walk during preparation.
            for triangle in Band::new(lower, upper).take(shapes) {
                let uv = triangle.map(|c| {
                    let segments = if c.upper { upper } else { lower };
                    [
                        if segments == 0 {
                            0.0
                        } else {
                            f64::from(c.index) / f64::from(segments) * core::f64::consts::TAU
                        },
                        latitude(if c.upper { j } else { j - 1 }),
                    ]
                });
                let Some(gap) = crate::geometry::unit_sphere_triangle_gap(uv, angle_error) else {
                    return Ok(());
                };
                unit_gap = unit_gap.max(gap);
            }
            lower = upper;
        }
        let triangles = 2 * (vertices - 2);
        for (storage, entries, size) in [
            ("triangles", triangles, size_of::<[u32; 3]>()),
            (
                "vertices",
                vertices,
                size_of::<(exedra_mesh::VertexId, VertexSource)>(),
            ),
        ] {
            if entries > (isize::MAX as u64) / (size as u64) {
                return Err(RealizationError::AddressSpace { storage });
            }
        }
        // The Gram-entry tolerance bounds each accepted frame's operator.
        // 1+4*tolerance encloses support and occurrence frame composition,
        // including dot-product rounding. Apply it to alignment too.
        let gap = (((unit_gap * self.radius).next_up() + self.alignment).next_up()
            * (1.0 + 4.0 * FRAME_TOLERANCE).next_up())
        .next_up();
        if gap.is_finite() && gap <= limit {
            self.radial = Some(gap);
        }
        Ok(())
    }

    pub(super) fn interior_count(self, counts: &[u32]) -> usize {
        let [u, v] = self.intervals(counts);
        if self.radial.is_some() {
            (1..v)
                .map(|j| Self::ring_segments(j, [u, v]) as usize - 1)
                .sum()
        } else {
            (u as usize - 1).saturating_mul(v as usize - 1)
        }
    }
    pub(super) fn pole_corner_count(self, counts: &[u32]) -> usize {
        let n = self.intervals(counts);
        if self.radial.is_some() {
            (Self::ring_segments(1, n) as usize)
                .saturating_add(Self::ring_segments(n[1] - 1, n) as usize)
        } else {
            (n[0] as usize)
                .saturating_mul(usize::from(self.sides[0].pole) + usize::from(self.sides[2].pole))
        }
    }
    pub(super) fn triangle_count(self, counts: &[u32]) -> usize {
        let [u, v] = self.intervals(counts);
        if self.radial.is_some() {
            return 2
                * (1..v)
                    .map(|j| Self::ring_segments(j, [u, v]) as usize)
                    .sum::<usize>();
        }
        let bands = (v as usize)
            .saturating_mul(2)
            .saturating_sub(usize::from(self.sides[0].pole) + usize::from(self.sides[2].pole));
        (u as usize).saturating_mul(bands)
    }
    fn bound<F>(self, face: &Face<F>, n: [u32; 2]) -> f64 {
        let u = step(face.domain.u, n[0]);
        let operator = (1.0 + 4.0 * FRAME_TOLERANCE).next_up();
        let interpolation = if self.spherical {
            let v = step(face.domain.v, n[1]);
            self.radius * (u + v) * (u + v) / 8.0
        } else {
            let s = libm::sin(u * 0.25);
            2.0 * self.radius * s * s
        };
        (((interpolation.next_up() + self.alignment).next_up()) * operator).next_up()
    }
    pub(super) fn deviation<F>(self, face: &Face<F>, counts: &[u32]) -> f64 {
        self.radial
            .unwrap_or_else(|| self.bound(face, self.intervals(counts)))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one bounded realization emission context"
    )]
    pub(super) fn emit<F, E>(
        self,
        body: &Body<F, E>,
        id: FaceId,
        placement: Placement3,
        counts: &[u32],
        chains: &Chains,
        scratch: &mut Scratch,
        points: &mut Vec<[f32; 3]>,
        sources: &mut Vec<VertexSource>,
        displacement: &mut f64,
        region: u32,
        triangles: &mut Vec<[u32; 3]>,
        regions: &mut Vec<u32>,
        faces: &mut Vec<FaceId>,
        poles: &mut Vec<PoleSeed>,
    ) -> Result<(), RealizationError> {
        let face = &body.faces()[id.index()];
        let [nu, nv] = self.intervals(counts);
        if self.sides[0].pole && self.sides[2].pole {
            if points[self.sides[0].at(chains, 0) as usize]
                == points[self.sides[2].at(chains, 0) as usize]
            {
                return Err(RealizationError::DegenerateFace { face: id });
            }
        } else {
            let (pole, rim) = if self.sides[0].pole {
                (self.sides[0], self.sides[2])
            } else {
                (self.sides[2], self.sides[0])
            };
            let Some((Curve::Circle(circle), _)) = body.edges()[rim.edge.index()].regular() else {
                return Err(RealizationError::UnsupportedSurfaceTrim { face: id });
            };
            let center = narrow(placement.transform_point(circle.frame().rows.map(|r| r[3])));
            if points[pole.at(chains, 0) as usize] == center {
                return Err(RealizationError::DegenerateFace { face: id });
            }
        }

        let key = (face.domain.u.bounds().map(f64::to_bits), nu);
        if scratch.key != Some(key) {
            scratch.phases.clear();
            scratch.phases.reserve(nu as usize + 1);
            for i in 0..=nu {
                let u = face
                    .domain
                    .u
                    .at(f64::from(i) / f64::from(nu))
                    .expect("grid fraction");
                let (s, c) = sin_cos(u);
                scratch.phases.push((u, s, c));
            }
            scratch.key = Some(key);
        }
        scratch.previous.clear();
        scratch.previous.reserve(nu as usize + 1);
        scratch.row.clear();
        scratch.row.reserve(nu as usize + 1);
        for j in 0..=nv {
            scratch.row.clear();
            let v = face
                .domain
                .v
                .at(f64::from(j) / f64::from(nv))
                .expect("grid fraction");
            let (sv, cv) = sin_cos(v);
            let row_segments = if self.radial.is_some() {
                Self::ring_segments(j, [nu, nv])
            } else {
                nu
            };
            let phase_stride = nu.checked_div(row_segments).unwrap_or(0) as usize;
            for i in 0..=row_segments as usize {
                let phase = i * phase_stride;
                let (u, su, cu) = scratch.phases[phase];
                let index = if j == 0 {
                    self.sides[0].at(chains, i)
                } else if j == nv {
                    self.sides[2].at(chains, i)
                } else if i == 0 {
                    self.sides[3].at(chains, j as usize)
                } else if i == row_segments as usize {
                    self.sides[1].at(chains, j as usize)
                } else {
                    let local = match &face.surface {
                        Surface::Sphere(s) => s.frame().transform_point([
                            s.radius() * cv * cu,
                            s.radius() * cv * su,
                            s.radius() * sv,
                        ]),
                        Surface::Cone(c) => c.frame().transform_point([
                            c.radius_at(v)
                                .ok_or(RealizationError::DegenerateFace { face: id })?
                                * cu,
                            c.radius_at(v)
                                .ok_or(RealizationError::DegenerateFace { face: id })?
                                * su,
                            v,
                        ]),
                        _ => return Err(RealizationError::UnsupportedSurfaceTrim { face: id }),
                    };
                    let exact = placement.transform_point(local);
                    let p = narrow(exact);
                    if p.iter().any(|v| !v.is_finite()) {
                        return Err(RealizationError::NonFiniteFaceSample {
                            face: id,
                            uv: [u, v],
                        });
                    }
                    *displacement = displacement.max(
                        distance(exact, p.map(f64::from))
                            + super::placement_roundoff(placement, local),
                    );
                    let index = crate::usize_to_u32(points.len());
                    points.push(p);
                    sources.push(VertexSource::Face {
                        face: id,
                        uv: [u, v],
                    });
                    index
                };
                scratch.row.push(index);
            }
            if j > 0 {
                let previous_v = face
                    .domain
                    .v
                    .at(f64::from(j - 1) / f64::from(nv))
                    .expect("grid fraction");
                let middle_v = 0.5 * (previous_v + v);
                let lower_segments = if j == 1 && self.sides[0].pole {
                    0
                } else if self.radial.is_some() {
                    Self::ring_segments(j - 1, [nu, nv])
                } else {
                    nu
                };
                let upper_segments = if j == nv && self.sides[2].pole {
                    0
                } else {
                    row_segments
                };
                let mut previous_normal = None;
                for corners in Band::new(lower_segments, upper_segments) {
                    let mut min_u = f64::INFINITY;
                    let mut max_u = f64::NEG_INFINITY;
                    for corner in corners {
                        let segments = if corner.upper {
                            upper_segments
                        } else {
                            lower_segments
                        };
                        if let Some(stride) = nu.checked_div(segments) {
                            let parameter =
                                scratch.phases[corner.index as usize * stride as usize].0;
                            min_u = min_u.min(parameter);
                            max_u = max_u.max(parameter);
                        }
                    }
                    let middle_u = 0.5 * (min_u + max_u);
                    let normal = if let Some((old_u, normal)) = previous_normal
                        && old_u == middle_u
                    {
                        normal
                    } else {
                        let normal = body
                            .evaluate_support(id, [middle_u, middle_v])
                            .ok_or(RealizationError::UnsupportedSurfaceTrim { face: id })?
                            .normal;
                        let normal = vector(placement, normal);
                        previous_normal = Some((middle_u, normal));
                        normal
                    };
                    let mut triangle = corners.map(|corner| {
                        if corner.upper {
                            scratch.row[corner.index as usize]
                        } else {
                            scratch.previous[corner.index as usize]
                        }
                    });
                    if face.sense == Sense::Reverse {
                        triangle.swap(1, 2);
                    }
                    let p = triangle.map(|v| points[v as usize].map(f64::from));
                    if dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), normal) <= 0.0 {
                        return Err(RealizationError::DegenerateFace { face: id });
                    }
                    for (side, vv) in [
                        (self.sides[0], face.domain.v.bounds()[0]),
                        (self.sides[2], face.domain.v.bounds()[1]),
                    ] {
                        if side.pole {
                            let vertex = body.edges()[side.edge.index()].start;
                            let mesh_vertex = side.at(chains, 0);
                            if triangle.contains(&mesh_vertex) {
                                poles.push(PoleSeed {
                                    triangle: triangles.len(),
                                    vertex: mesh_vertex,
                                    native_vertex: vertex,
                                    face: id,
                                    uv: [middle_u, vv],
                                });
                            }
                        }
                    }
                    emit(triangles, triangle, id, region, points, regions, faces)?;
                }
            }
            core::mem::swap(&mut scratch.previous, &mut scratch.row);
        }
        Ok(())
    }
}

pub(super) fn limit_normal<F>(
    face: &Face<F>,
    uv: [f64; 2],
    placement: Placement3,
) -> Option<[f64; 3]> {
    let normal = face
        .surface
        .pole_limit_normal(uv)?
        .map(|n| n * face.sense.sign());
    crate::geometry::transformed_normal(placement, normal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revolve::Revolution;
    use alloc::vec;
    use core::f64::consts::TAU;
    use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};
    fn sphere(r: f64) -> Profile {
        Profile::new(
            Loop::new(vec![Segment::line([0.0, -r]), Segment::arc([0.0, r], 1.0)]).unwrap(),
            vec![],
            &ProfilePolicy::new(r * 1e-9),
        )
        .unwrap()
    }
    fn cone(r: f64, h: f64, _extra: bool) -> Profile {
        Profile::new(
            Loop::new(vec![
                Segment::line([r, 0.0]),
                Segment::line([0.0, h]),
                Segment::line([0.0, 0.0]),
            ])
            .unwrap(),
            vec![],
            &ProfilePolicy::new(r.max(h) * 1e-9),
        )
        .unwrap()
    }

    #[test]
    fn pole_realizations_are_closed_oriented_welded_and_bounded() {
        for source in [sphere(2.0), cone(2.0, 3.0, false)] {
            let solid = Revolution::new(source, Placement3::IDENTITY, TAU).unwrap();
            let mut previous_error = f64::INFINITY;
            for tolerance in [0.1, 0.025, 0.00625] {
                let mesh = solid
                    .realize(&RealizationPolicy {
                        tolerance,
                        ..RealizationPolicy::default()
                    })
                    .unwrap();
                assert!(mesh.mesh.validate_deep().is_empty());
                assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
                assert!(mesh.report.within_tolerance(), "{:?}", mesh.report);
                assert!(mesh.report.max_deviation < previous_error);
                previous_error = mesh.report.max_deviation;
                let mut volume = 0.0;
                for face in mesh.mesh.faces() {
                    let p: Vec<_> = mesh
                        .mesh
                        .face_loop(face)
                        .map(|corner| {
                            mesh.mesh
                                .vertex_position(mesh.mesh.to_vertex(corner).unwrap())
                                .unwrap()
                                .map(f64::from)
                        })
                        .collect();
                    assert_eq!(p.len(), 3);
                    volume += dot(p[0], cross(p[1], p[2])) / 6.0;
                }
                assert!(volume > 0.0);
                assert!((solid.volume() - volume).abs() < solid.volume() * tolerance);
                for (i, edge) in solid.body().edges().iter().enumerate() {
                    if edge.is_collapsed() {
                        assert_eq!(mesh.report.edge_segments[i], 0);
                        assert_eq!(
                            mesh.edge_chains
                                .get(EdgeId(u32::try_from(i).unwrap()))
                                .unwrap()
                                .len(),
                            1
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn pole_corner_normals_and_charts_use_forward_sector_parameters() {
        for source in [sphere(2.0), cone(2.0, 3.0, false)] {
            let solid = Revolution::new(source, Placement3::IDENTITY, TAU).unwrap();
            let policy = RealizationPolicy {
                tolerance: 0.025,
                ..RealizationPolicy::default()
            };
            let plain = solid.realize(&policy).unwrap();
            let mut calls = 0;
            let charted = solid
                .realize_with_uvs(
                    &policy,
                    |id, _| id.0,
                    |_, _, points| -> Result<_, core::convert::Infallible> {
                        calls += 1;
                        Ok(points.map(|p| {
                            let p = narrow([p[0], p[1], 0.0]);
                            [p[0], p[1]]
                        }))
                    },
                )
                .unwrap();
            assert_eq!(calls, charted.mesh.faces().count());
            assert_eq!(charted.report, plain.report);
            assert!(!charted.pole_corners.is_empty());
            assert_eq!(charted.pole_corners.len(), plain.pole_corners.len());
            let normals = charted
                .mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                .unwrap();
            let uvs = charted
                .mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_UV)
                .unwrap();
            let mut min_u = f64::INFINITY;
            let mut max_u = f64::NEG_INFINITY;
            for record in charted.pole_corners.as_slice() {
                assert_eq!(charted.pole_corners.get(record.corner), Some(record));
                let vertex = charted.mesh.to_vertex(record.corner).unwrap();
                let source = charted
                    .mesh_vertices
                    .iter()
                    .find(|p| p.0 == vertex)
                    .unwrap()
                    .1;
                assert_eq!(source, VertexSource::Vertex(record.vertex));
                let n = normals.get(record.corner.as_id()).unwrap().map(f64::from);
                assert!((dot(n, n) - 1.0).abs() < 1e-6);
                let expected = limit_normal(
                    &solid.body().faces()[record.face.index()],
                    record.uv,
                    Placement3::IDENTITY,
                )
                .unwrap();
                assert!(dot(n, expected) > 1.0 - 1e-6);
                let uv = uvs.get(record.corner.as_id()).unwrap();
                assert!((f64::from(uv[0]) - record.uv[0]).abs() < 1e-6);
                min_u = min_u.min(record.uv[0]);
                max_u = max_u.max(record.uv[0]);
            }
            assert!(
                max_u - min_u > 5.0,
                "a welded pole needs distinct chart-sector coordinates"
            );
        }
    }

    #[test]
    fn pole_error_bounds_cover_independent_chord_witnesses_across_scales_and_datums() {
        for scale in [1e-6, 1.0, 1e6] {
            for spherical in [true, false] {
                let radius = 2.0 * scale;
                let height = 3.0 * scale;
                let center = [1000.0 * scale, -2000.0 * scale, 3000.0 * scale];
                let source = if spherical {
                    sphere(radius)
                } else {
                    cone(radius, height, false)
                };
                let solid = Revolution::new(
                    source,
                    Placement3::translate(center[0], center[1], center[2]),
                    TAU,
                )
                .unwrap();
                let mesh = solid
                    .realize(&RealizationPolicy {
                        tolerance: 0.025 * scale,
                        geometry_gap: 1e-9 * scale,
                        ..Default::default()
                    })
                    .unwrap();
                assert!(mesh.report.within_tolerance(), "{:?}", mesh.report);
                let mut witnessed = 0.0_f64;
                for &(face, native) in &mesh.mesh_faces {
                    let points: Vec<_> = mesh
                        .mesh
                        .face_loop(face)
                        .map(|corner| {
                            let p = mesh
                                .mesh
                                .vertex_position(mesh.mesh.to_vertex(corner).unwrap())
                                .unwrap()
                                .map(f64::from);
                            sub(p, center)
                        })
                        .collect();
                    for weights in [
                        [0.5, 0.5, 0.0],
                        [0.0, 0.5, 0.5],
                        [0.5, 0.0, 0.5],
                        [1.0 / 3.0; 3],
                    ] {
                        let point = core::array::from_fn::<_, 3, _>(|k| {
                            (0..3).map(|i| weights[i] * points[i][k]).sum::<f64>()
                        });
                        let gap = if spherical {
                            (libm::hypot(libm::hypot(point[0], point[1]), point[2]) - radius).abs()
                        } else if matches!(
                            solid.body().faces()[native.index()].surface,
                            Surface::Plane(_)
                        ) {
                            point[1].abs()
                        } else {
                            (libm::hypot(point[0], point[2]) - radius + radius / height * point[1])
                                .abs()
                                / libm::hypot(1.0, radius / height)
                        };
                        witnessed = witnessed.max(gap);
                    }
                }
                assert!(witnessed > 0.0);
                assert!(
                    witnessed <= mesh.report.max_deviation,
                    "independent chord witness {witnessed} exceeds {:?}",
                    mesh.report
                );
            }
        }
    }

    #[test]
    fn radial_cover_keeps_shifted_parameter_origin_and_reversed_face_sense() {
        let source = Revolution::new(sphere(2.0), Placement3::IDENTITY, TAU).unwrap();
        let policy = RealizationPolicy {
            tolerance: 0.025,
            ..Default::default()
        };
        let ordinary = source.realize(&policy).unwrap();
        for origin in [-core::f64::consts::PI, 1.5, -6.25] {
            for reversed in [false, true] {
                let mut parts = source.body().clone().into_parts();
                let Surface::Sphere(sphere) = &parts.faces[0].surface else {
                    unreachable!()
                };
                let frame =
                    sphere.frame() * Placement3::rotate_z_then_translate(-origin, 0.0, 0.0, 0.0);
                parts.faces[0].surface = Surface::sphere(frame, 2.0).unwrap();
                parts.faces[0].domain.u = ParameterRange::new(origin, origin + TAU).unwrap();
                if reversed {
                    parts.faces[0].sense = Sense::Reverse;
                    for ring in &mut parts.loops {
                        ring.coedges.reverse();
                        for edge in &mut ring.coedges {
                            edge.reversed = !edge.reversed;
                        }
                    }
                }
                let body =
                    Body::try_from_parts(parts, &crate::body::ValidationPolicy { max_gap: 1e-9 })
                        .unwrap();
                let result = body
                    .realize(Placement3::IDENTITY, &policy)
                    .unwrap_or_else(|error| {
                        panic!("origin={origin}, reversed={reversed}: {error:?}")
                    });
                assert_eq!(result.mesh_faces.len(), ordinary.mesh_faces.len());
                assert!(result.mesh.validate_deep().is_empty());
                assert!(result.mesh.boundary_loops().unwrap().is_empty());
                assert!(result.report.within_tolerance());
                let mut volume = 0.0;
                for face in result.mesh.faces() {
                    let p: Vec<_> = result
                        .mesh
                        .face_loop(face)
                        .map(|c| {
                            result
                                .mesh
                                .vertex_position(result.mesh.to_vertex(c).unwrap())
                                .unwrap()
                                .map(f64::from)
                        })
                        .collect();
                    volume += dot(p[0], cross(p[1], p[2])) / 6.0;
                }
                assert_eq!(
                    volume > 0.0,
                    !reversed,
                    "FaceSense is applied after the natural radial-cover proof"
                );
            }
        }
    }

    #[test]
    fn spherical_and_conical_cavities_keep_inward_walls_and_closed_solids() {
        for spherical in [true, false] {
            let side = if spherical {
                Segment::arc_centered([0.0, 2.0], [0.0, 4.0], true)
            } else {
                Segment::line([0.0, 2.0])
            };
            let profile = Profile::new(
                Loop::new(vec![
                    Segment::line([4.0, 0.0]),
                    Segment::line([4.0, 4.0]),
                    Segment::line([2.0, 4.0]),
                    side,
                    Segment::line([0.0, 0.0]),
                ])
                .unwrap(),
                vec![],
                &ProfilePolicy::new(1e-9),
            )
            .unwrap();
            let source = Revolution::new(profile, Placement3::IDENTITY, TAU).unwrap();
            let wall = source
                .body()
                .faces()
                .iter()
                .find(|face| matches!(face.surface, Surface::Sphere(_) | Surface::Cone(_)))
                .unwrap();
            assert_eq!(wall.sense, Sense::Reverse);
            let mut calls = 0;
            let result = source
                .realize_with_uvs(
                    &RealizationPolicy {
                        tolerance: 0.025,
                        ..Default::default()
                    },
                    |id, _| id.0,
                    |_, _, p| -> Result<_, core::convert::Infallible> {
                        calls += 1;
                        Ok(p.map(|v| {
                            let n = narrow([v[0], v[1], 0.0]);
                            [n[0], n[1]]
                        }))
                    },
                )
                .unwrap();
            assert_eq!(calls, result.mesh_faces.len());
            assert!(result.mesh.validate_deep().is_empty());
            assert!(result.mesh.boundary_loops().unwrap().is_empty());
            assert!(result.report.within_tolerance());
            let removed = if spherical { 16.0 / 3.0 } else { 8.0 / 3.0 };
            let expected = (64.0 - removed) * core::f64::consts::PI;
            assert!((source.volume() - expected).abs() < 1e-11);
            for pole in result.pole_corners.as_slice() {
                let n = source.body().pole_limit_normal(pole.face, pole.uv).unwrap();
                assert!(
                    n[1] > 0.5,
                    "the lower cavity pole normal points toward the empty region above it"
                );
            }
        }
    }

    #[test]
    fn wasm32_rectangular_fallback_refuses_saturated_counts_before_allocation() {
        let model = Revolution::new(sphere(1.0), Placement3::IDENTITY, TAU).unwrap();
        for policy in [
            RealizationPolicy {
                tolerance: 3e-9,
                max_edge_segments: 100001,
                max_vertices: u32::MAX,
                max_boundary_samples: u32::MAX,
                ..Default::default()
            },
            RealizationPolicy {
                min_arc_segments: 3_000_000_001,
                max_edge_segments: u32::MAX,
                max_vertices: u32::MAX,
                max_boundary_samples: u32::MAX,
                ..Default::default()
            },
        ] {
            let error = model.realize(&policy).unwrap_err();
            #[cfg(target_pointer_width = "32")]
            assert!(
                matches!(error, RealizationError::AddressSpace { .. }),
                "{error:?}"
            );
            #[cfg(target_pointer_width = "64")]
            assert!(
                matches!(error,RealizationError::Budget{storage:"vertices",required,..} if required>u32::MAX as usize),
                "{error:?}"
            );
        }
    }

    #[test]
    fn radial_schedule_obeys_native_vertex_budget_and_ordinary_fallback() {
        let model = Revolution::new(sphere(20.0), Placement3::IDENTITY, TAU).unwrap();
        let policy = RealizationPolicy {
            tolerance: 0.025,
            ..Default::default()
        };
        let mesh = model.realize(&policy).unwrap();
        let exact = u32::try_from(mesh.mesh_vertices.len()).unwrap();
        assert!(
            exact < 14000,
            "radial rings must remove the uniform schedule's excess vertices"
        );
        for face in mesh.mesh.faces() {
            let points: Vec<_> = mesh
                .mesh
                .face_loop(face)
                .map(|corner| {
                    mesh.mesh
                        .vertex_position(mesh.mesh.to_vertex(corner).unwrap())
                        .unwrap()
                        .map(f64::from)
                })
                .collect();
            for i in 0..3 {
                let a = sub(points[(i + 1) % 3], points[i]);
                let b = sub(points[(i + 2) % 3], points[i]);
                let c = cross(a, b);
                let sine = libm::hypot(libm::hypot(c[0], c[1]), c[2]);
                assert!(
                    libm::atan2(sine, dot(a, b)) > core::f64::consts::PI / 18.0,
                    "independent emitted-triangle angles must avoid uniform pole needles"
                );
            }
        }
        let bounded = model
            .realize(&RealizationPolicy {
                max_vertices: exact,
                ..policy
            })
            .unwrap();
        assert_eq!(mesh.report, bounded.report);
        assert_eq!(mesh.mesh_vertices, bounded.mesh_vertices);
        assert!(
            matches!(model.realize(&RealizationPolicy{max_vertices:exact-1,..policy}),Err(RealizationError::Budget{storage:"vertices",required,maximum}) if required>maximum as usize)
        );
        // A non-power-of-two authored minimum keeps the rectangular path rather
        // than changing caller sampling constraints to enable this strategy.
        let irregular = model
            .realize(&RealizationPolicy {
                min_arc_segments: 5,
                ..policy
            })
            .unwrap();
        assert!(irregular.mesh.validate_deep().is_empty());
        assert!(irregular.report.within_tolerance());
        assert!(irregular.mesh.boundary_loops().unwrap().is_empty());
    }

    #[test]
    fn emitted_poles_refuse_flattening_and_nonfinite_precision() {
        for source in [sphere(2.0), cone(2.0, 3.0, false)] {
            let model = Revolution::new(source, Placement3::translate(0.0, 1e8, 0.0), TAU).unwrap();
            assert!(
                matches!(
                    model.realize(&RealizationPolicy {
                        tolerance: 2.0,
                        ..RealizationPolicy::default()
                    }),
                    Err(RealizationError::DegenerateFace { .. })
                ),
                "opposing surfaces must not become a doubled planar mesh"
            );
        }
        let model =
            Revolution::new(sphere(1e38), Placement3::translate(0.0, 0.0, -2.5e38), TAU).unwrap();
        assert!(matches!(
            model.realize(&RealizationPolicy {
                tolerance: 1e37,
                geometry_gap: 1e29,
                ..Default::default()
            }),
            Err(RealizationError::NonFiniteFaceSample { .. })
        ));
        let model = Revolution::new(sphere(1e100), Placement3::IDENTITY, TAU).unwrap();
        assert!(matches!(
            model.realize(&RealizationPolicy {
                tolerance: 1e100,
                geometry_gap: 1e90,
                ..RealizationPolicy::default()
            }),
            Err(RealizationError::NonFiniteVertex { .. })
        ));
        let model = Revolution::new(sphere(1e-100), Placement3::IDENTITY, TAU).unwrap();
        assert!(matches!(
            model.realize(&RealizationPolicy {
                tolerance: 1e-101,
                geometry_gap: 1e-109,
                ..RealizationPolicy::default()
            }),
            Err(RealizationError::DegenerateFace { .. } | RealizationError::CollapsedEdge { .. })
        ));
    }
}
