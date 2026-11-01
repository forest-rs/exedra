// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Optional, budgeted shape planning over the positions a grid will emit.

use super::*;
use crate::realize::{
    Incidence, check_table_capacity,
    quality::{Rows, Work, radius_edge_ratio},
};

#[derive(Clone, Copy, Default)]
struct Score {
    maximum: f64,
    bad: u32,
    next: [u32; 2],
}
impl Score {
    fn improves(self, old: Self) -> bool {
        self.maximum < old.maximum * (1.0 - 8.0 * f64::from(f32::EPSILON))
            || (self.maximum <= old.maximum && self.bad < old.bad)
    }
}

/// Diagonals are retained from a completed scan, so emission does not repeat
/// unbudgeted shape tests. A count change invalidates the entire face schedule.
pub(in crate::realize) struct Schedule {
    counts: [u32; 2],
    diagonals: Vec<bool>,
    score: Score,
}
impl Schedule {
    pub(super) fn alternate(&self, counts: [usize; 2], u: usize, v: usize) -> bool {
        self.counts.map(|n| n as usize) == counts && self.diagonals[v * counts[0] + u]
    }
}

#[derive(Default)]
struct Scratch {
    sampler: GridScratch,
    key: Option<(FaceId, u32)>,
    previous: Vec<[f32; 3]>,
    row: Vec<[f32; 3]>,
    rims: [Vec<[f32; 3]>; 2],
    diagonals: Vec<bool>,
    counts: Vec<u32>,
}

pub(in crate::realize) fn refine_quality<F, E>(
    body: &Body<F, E>,
    patches: &[Patch],
    rows: &[Option<Rows>],
    incidence: &[Incidence],
    placement: Placement3,
    counts: &mut [u32],
    groups: &mut Groups,
    policy: &RealizationPolicy,
    work: &mut Work,
) -> Result<Vec<Option<Schedule>>, RealizationError> {
    let quality = policy.quality.expect("quality schedule");
    if !patches.iter().any(|p| matches!(p, Patch::Grid(_))) {
        return Ok(Vec::new());
    }
    if quality.max_grid_triangle_tests == 0 {
        work.exhausted = true;
        return Ok(Vec::new());
    }
    let roots: Vec<_> = (0..counts.len()).map(|i| groups.root(i)).collect();
    let mut active = alloc::vec![false; body.vertices().len()];
    for (edge, incidence) in body.edges().iter().zip(incidence) {
        if incidence.count > 0 {
            active[edge.start.index()] = true;
            active[edge.end.index()] = true;
        }
    }
    let native_vertices = active.iter().filter(|v| **v).count();
    drop(active);
    let mut scratch = Scratch {
        counts: counts.to_vec(),
        ..Default::default()
    };
    let mut schedules: Vec<Option<Schedule>> = (0..patches.len()).map(|_| None).collect();
    loop {
        let mut changed = false;
        for (i, patch) in patches.iter().enumerate() {
            let Patch::Grid(patch) = *patch else { continue };
            let face = FaceId(crate::usize_to_u32(i));
            let n = patch.intervals(counts);
            if !schedules[i].as_ref().is_some_and(|s| s.counts == n) {
                // The existing hard-budget check will diagnose an accuracy
                // schedule that already exceeds storage; do not sample it here.
                if !fits(
                    body,
                    patches,
                    rows,
                    incidence,
                    native_vertices,
                    counts,
                    policy,
                ) {
                    return Ok(schedules);
                }
                let Some(score) =
                    scratch.scan(body, face, patch, placement, counts, policy, work)?
                else {
                    return Ok(schedules);
                };
                store(&mut schedules[i], &mut scratch.diagonals, n, score);
            }
            let score = schedules[i].as_ref().expect("completed scan").score;
            if score.bad == 0 {
                continue;
            }
            let mut axes = [0, 1];
            if f64::from(score.next[1]) / f64::from(n[1])
                > f64::from(score.next[0]) / f64::from(n[0])
            {
                axes.swap(0, 1);
            }
            let mut limited = false;
            for axis in axes.into_iter().map(Some).chain(core::iter::once(None)) {
                scratch.counts.copy_from_slice(counts);
                let mut raised = false;
                for a in 0..2 {
                    if axis.is_some_and(|axis| axis != a) {
                        continue;
                    }
                    let desired = if axis.is_some() {
                        score.next[a]
                    } else {
                        n[a].saturating_mul(2)
                    };
                    if desired <= n[a] {
                        continue;
                    }
                    let next = desired.min(policy.max_edge_segments);
                    limited |= desired > next;
                    if next <= n[a] {
                        continue;
                    }
                    let root = roots[patch.sides[a].edge.index()];
                    for (count, &owner) in scratch.counts.iter_mut().zip(&roots) {
                        if owner == root {
                            *count = (*count).max(next);
                        }
                    }
                    raised = true;
                }
                if !raised {
                    continue;
                }
                if !fits(
                    body,
                    patches,
                    rows,
                    incidence,
                    native_vertices,
                    &scratch.counts,
                    policy,
                ) {
                    limited = true;
                    continue;
                }
                // Shape changes never replace the geometric error certificate.
                // Recheck it before sampling or accepting a candidate schedule.
                if let Err(error) = refine(body, patches, &mut scratch.counts, groups, policy) {
                    if matches!(error, RealizationError::SurfaceSampling { .. }) {
                        // A shape proposal may encounter a rounded parameter
                        // schedule that cannot meet geometry within the edge
                        // cap. Keep the already-certified original instead.
                        limited = true;
                        continue;
                    }
                    return Err(error);
                }
                if !fits(
                    body,
                    patches,
                    rows,
                    incidence,
                    native_vertices,
                    &scratch.counts,
                    policy,
                ) {
                    limited = true;
                    continue;
                }
                let candidate_counts = core::mem::take(&mut scratch.counts);
                let candidate = scratch.scan(
                    body,
                    face,
                    patch,
                    placement,
                    &candidate_counts,
                    policy,
                    work,
                )?;
                scratch.counts = candidate_counts;
                let Some(candidate) = candidate else {
                    return Ok(schedules);
                };
                if !candidate.improves(score) {
                    continue;
                }
                counts.copy_from_slice(&scratch.counts);
                store(
                    &mut schedules[i],
                    &mut scratch.diagonals,
                    patch.intervals(counts),
                    candidate,
                );
                changed = true;
                break;
            }
            if !changed && limited {
                work.exhausted = true;
            }
            // Synchronization can change previously visited faces. Restart in
            // native order, reusing every still-current completed scan.
            if changed {
                break;
            }
        }
        if !changed {
            return Ok(schedules);
        }
    }
}

fn store(slot: &mut Option<Schedule>, scratch: &mut Vec<bool>, counts: [u32; 2], score: Score) {
    let schedule = slot.get_or_insert_with(|| Schedule {
        counts,
        diagonals: Vec::new(),
        score,
    });
    schedule.counts = counts;
    schedule.score = score;
    core::mem::swap(&mut schedule.diagonals, scratch);
}

/// Exact pre-emission cardinality for all native boundary and planned interior
/// vertices. Pole schedules are prepared on a copy under these candidate counts.
fn fits<F, E>(
    body: &Body<F, E>,
    patches: &[Patch],
    rows: &[Option<Rows>],
    incidence: &[Incidence],
    native_vertices: usize,
    counts: &[u32],
    policy: &RealizationPolicy,
) -> bool {
    let mut vertices = native_vertices;
    let mut boundary = 0_usize;
    for ((edge, &n), incidence) in body.edges().iter().zip(counts).zip(incidence) {
        if incidence.count == 0 {
            continue;
        }
        if edge.is_collapsed() {
            boundary = boundary.saturating_add(1);
        } else {
            boundary = boundary.saturating_add(n as usize).saturating_add(1);
            vertices = vertices.saturating_add(n as usize - 1);
        }
    }
    for (i, patch) in patches.iter().enumerate() {
        let interior = match *patch {
            Patch::Grid(patch) => patch.interior_count(counts),
            Patch::Pole(mut patch) => {
                if patch
                    .prepare_radial_sampling(&body.faces()[i], counts, policy)
                    .is_err()
                {
                    return false;
                }
                patch.interior_count(counts)
            }
            _ => rows
                .get(i)
                .and_then(|row| *row)
                .map_or(0, |row| row.interior_count(counts)),
        };
        vertices = vertices.saturating_add(interior);
    }
    vertices <= policy.max_vertices as usize && boundary <= policy.max_boundary_samples as usize
}

impl Scratch {
    fn scan<F, E>(
        &mut self,
        body: &Body<F, E>,
        id: FaceId,
        patch: GridPatch,
        placement: Placement3,
        counts: &[u32],
        policy: &RealizationPolicy,
        work: &mut Work,
    ) -> Result<Option<Score>, RealizationError> {
        let settings = policy.quality.expect("quality schedule");
        let n = patch.intervals(counts);
        let tests = u64::from(n[0]) * u64::from(n[1]) * 4;
        if tests
            > u64::from(
                settings
                    .max_grid_triangle_tests
                    .saturating_sub(work.grid_triangle_tests),
            )
        {
            work.exhausted = true;
            return Ok(None);
        }
        let [nu, nv] = n.map(|n| n as usize);
        check_table_capacity::<Phase>("grid quality phases", nu + 1)?;
        check_table_capacity::<RuledSample>("grid quality source samples", nu + 1)?;
        check_table_capacity::<[f32; 3]>("grid quality rows", nu + 1)?;
        check_table_capacity::<bool>("grid quality diagonals", nu * nv)?;
        let face = &body.faces()[id.index()];
        if self.key != Some((id, n[0])) {
            self.sampler.prepare_face(face, nu);
            for (rim, side) in self.rims.iter_mut().zip([patch.sides[0], patch.sides[2]]) {
                rim.clear();
                rim.reserve(nu + 1);
                for u in 0..=nu {
                    let index = if side.reverse { nu - u } else { u };
                    let exact = body
                        .edge_point(side.edge, index as f64 / nu as f64)
                        .expect("prepared boundary");
                    let point = narrow(placement.transform_point(exact));
                    if !point.iter().all(|v| v.is_finite()) {
                        return Err(RealizationError::DegenerateFace { face: id });
                    }
                    rim.push(point);
                }
            }
            self.key = Some((id, n[0]));
        }
        self.previous.clear();
        self.previous.reserve(nu + 1);
        self.row.clear();
        self.row.reserve(nu + 1);
        self.diagonals.clear();
        self.diagonals.reserve(nu * nv);
        let mut score = Score {
            next: n,
            ..Default::default()
        };
        for v in 0..=nv {
            self.row.clear();
            let vv = face
                .domain
                .v
                .at(v as f64 / nv as f64)
                .expect("grid fraction");
            let svcv = if matches!(face.surface, Surface::Torus(_)) {
                [libm::sin(vv), libm::cos(vv)]
            } else {
                [0.0; 2]
            };
            for u in 0..=nu {
                let p = if v == 0 {
                    self.rims[0][u]
                } else if v == nv {
                    self.rims[1][u]
                } else {
                    let exact = if u == 0 || u == nu {
                        let side = patch.sides[if u == 0 { 3 } else { 1 }];
                        let index = if side.reverse { nv - v } else { v };
                        body.edge_point(side.edge, index as f64 / nv as f64)
                            .expect("prepared boundary")
                    } else {
                        sample_point(
                            face,
                            &self.sampler.phases[u],
                            &self.sampler.ruled,
                            u,
                            vv,
                            svcv,
                        )
                    };
                    narrow(placement.transform_point(exact))
                };
                if !p.iter().all(|v| v.is_finite()) {
                    return Err(RealizationError::DegenerateFace { face: id });
                }
                self.row.push(p);
            }
            if v > 0 {
                let previous = face
                    .domain
                    .v
                    .at((v - 1) as f64 / nv as f64)
                    .expect("grid fraction");
                let middle = 0.5 * (previous + vv);
                let svcv = if matches!(face.surface, Surface::Torus(_)) {
                    [libm::sin(middle), libm::cos(middle)]
                } else {
                    [0.0; 2]
                };
                for u in 0..nu {
                    let p = [
                        self.previous[u],
                        self.previous[u + 1],
                        self.row[u + 1],
                        self.row[u],
                    ];
                    let normal = sample_normal(
                        face,
                        &self.sampler.phases[u],
                        &self.sampler.ruled,
                        u,
                        middle,
                        svcv,
                    )
                    .map(|n| vector(placement, n))
                    .ok_or(RealizationError::DegenerateFace { face: id })?;
                    let (alternate, ratios) = diagonal(p, normal, face.sense == Sense::Reverse);
                    work.grid_triangle_tests += 4;
                    self.diagonals.push(alternate);
                    let maximum = ratios[0].max(ratios[1]);
                    score.maximum = score.maximum.max(maximum);
                    score.bad += u32::from(ratios[0] > settings.max_radius_edge_ratio)
                        + u32::from(ratios[1] > settings.max_radius_edge_ratio);
                    if maximum > settings.max_radius_edge_ratio {
                        let p = p.map(|p| p.map(f64::from));
                        let lengths = [
                            distance(p[0], p[1]).max(distance(p[2], p[3])),
                            distance(p[0], p[3]).max(distance(p[1], p[2])),
                        ];
                        let axis = usize::from(lengths[1] > lengths[0]);
                        if lengths[axis] > lengths[1 - axis] * (1.0 + 1e-6) {
                            let aspect = libm::sqrt(
                                4.0 * settings.max_radius_edge_ratio
                                    * settings.max_radius_edge_ratio
                                    - 1.0,
                            );
                            let required = libm::ceil(
                                f64::from(n[axis]) * lengths[axis] / (lengths[1 - axis] * aspect),
                            );
                            let next = interval_count(required);
                            score.next[axis] =
                                score.next[axis].max(next.max(n[axis].saturating_add(1)));
                        }
                    }
                }
            }
            core::mem::swap(&mut self.row, &mut self.previous);
        }
        Ok(Some(score))
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "finite nonnegative ceiling is checked against u32 before conversion"
)]
fn interval_count(required: f64) -> u32 {
    if required.is_finite() && required >= 0.0 && required < f64::from(u32::MAX) {
        required as u32
    } else {
        u32::MAX
    }
}

fn diagonal(points: [[f32; 3]; 4], normal: [f64; 3], reverse: bool) -> (bool, [f64; 2]) {
    let points = points.map(|p| p.map(f64::from));
    let ratios = |alternate| {
        cell_triangles(alternate).map(|mut t| {
            if reverse {
                t.swap(1, 2);
            }
            let p = t.map(|i| points[i]);
            if dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), normal) > 0.0 {
                radius_edge_ratio(p)
            } else {
                f64::INFINITY
            }
        })
    };
    let original = ratios(false);
    let alternate = ratios(true);
    if alternate[0].max(alternate[1]) < original[0].max(original[1]) {
        (true, alternate)
    } else {
        (false, original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{
        BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, LoopId, ValidationPolicy, VertexId,
    };
    use crate::geometry::ParameterDomain;
    use crate::realize::{TriangleQuality, VertexSource};

    fn flat_grids(quads: &[[[f64; 3]; 4]]) -> Body {
        let mut parts = BodyParts {
            vertices: Vec::new(),
            edges: Vec::new(),
            loops: Vec::new(),
            faces: Vec::new(),
            trims: Vec::new(),
        };
        for (i, p) in quads.iter().enumerate() {
            let base = crate::usize_to_u32(parts.vertices.len());
            let edge_base = crate::usize_to_u32(parts.edges.len());
            parts.vertices.extend(p);
            for (a, b) in [(0, 1), (1, 2), (3, 2), (0, 3)] {
                parts.edges.push(Edge::new(
                    Curve::line(p[a], sub(p[b], p[a])).unwrap(),
                    ParameterRange::UNIT,
                    VertexId(base + crate::usize_to_u32(a)),
                    VertexId(base + crate::usize_to_u32(b)),
                    (),
                ));
            }
            let span = |side: usize| {
                CurveSpan::new(
                    parts.edges[edge_base as usize + side]
                        .regular()
                        .unwrap()
                        .0
                        .clone(),
                    ParameterRange::UNIT,
                    false,
                )
                .unwrap()
            };
            let surface = Surface::ruled(span(0), span(2)).unwrap();
            parts.loops.push(BoundaryLoop {
                coedges: (0..4)
                    .map(|j| Coedge {
                        edge: EdgeId(edge_base + j),
                        reversed: j >= 2,
                    })
                    .collect(),
            });
            parts.faces.push(Face {
                surface,
                domain: ParameterDomain {
                    u: ParameterRange::UNIT,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Forward,
                loops: alloc::vec![LoopId(crate::usize_to_u32(i))],
                role: (),
            });
        }
        Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
    }

    fn wall_shape(output: &crate::realize::RealizedBody) -> (usize, f64) {
        let mut bad = 0;
        let mut maximum = 0.0_f64;
        for &(face, native) in &output.mesh_faces {
            if native != FaceId(2) {
                continue;
            }
            let p: Vec<_> = output
                .mesh
                .face_loop(face)
                .map(|c| {
                    output
                        .mesh
                        .vertex_position(output.mesh.to_vertex(c).unwrap())
                        .unwrap()
                        .map(f64::from)
                })
                .collect();
            // Independent circumradius/min-edge formula on emitted vertices.
            let sides = [
                distance(p[0], p[1]),
                distance(p[1], p[2]),
                distance(p[2], p[0]),
            ];
            let area2 = exedra_math::norm(cross(sub(p[1], p[0]), sub(p[2], p[0])));
            let ratio = sides[0] * sides[1] * sides[2]
                / (2.0 * area2 * sides.into_iter().fold(f64::INFINITY, f64::min));
            maximum = maximum.max(ratio);
            bad += usize::from(ratio > 2.5);
        }
        (bad, maximum)
    }

    #[test]
    fn dense_spline_boundaries_gain_shared_ruling_rows_without_fake_curvature() {
        let body = crate::nurbs_tests::prism();
        let placement = Placement3::rotate_y_then_translate(0.37, 3.0, -2.0, 4.0);
        let mut policy = RealizationPolicy {
            min_arc_segments: 256,
            quality: Some(TriangleQuality::new(0.1).with_max_grid_triangle_tests(0)),
            ..Default::default()
        };
        let original = body.realize(placement, &policy).unwrap();
        assert!(
            wall_shape(&original).0 > 0,
            "accuracy-only grids leave long spline-wall triangles"
        );
        policy.quality = Some(TriangleQuality::new(0.1));
        let improved = body.realize(placement, &policy).unwrap();
        let shape = wall_shape(&improved);
        assert_eq!(shape.0, 0, "requested emitted wall quality: {shape:?}");
        assert!(improved.report.edge_segments[4] > original.report.edge_segments[4]);
        assert_eq!(
            improved.report.edge_segments[4],
            improved.report.edge_segments[5]
        );
        assert!(improved.report.within_tolerance());
        assert!(
            improved.report.max_surface_deviation <= original.report.max_surface_deviation + 1e-12,
            "quality does not add fictitious curvature error"
        );
        assert!(improved.mesh.boundary_loops().unwrap().is_empty());
        assert!(improved.mesh.validate_deep().is_empty());
        let report = improved.report.quality.as_ref().unwrap();
        assert!(report.grid_triangle_tests > 0 && report.grid_triangle_tests <= 1 << 20);
        for &(vertex, source) in &improved.mesh_vertices {
            if let VertexSource::Face { face, uv } = source {
                let expected =
                    placement.transform_point(body.evaluate_support(face, uv).unwrap().point);
                let actual = improved
                    .mesh
                    .vertex_position(vertex)
                    .unwrap()
                    .map(f64::from);
                assert!(
                    distance(expected, actual) <= improved.report.max_vertex_displacement + 1e-10,
                    "native supporting parameters reconstruct every generated vertex"
                );
            }
        }
        for edge in [EdgeId(4), EdgeId(5)] {
            let chain = improved.edge_chains.get(edge).unwrap();
            assert_eq!(
                chain.len(),
                improved.report.edge_segments[edge.index()] as usize + 1
            );
        }
    }

    #[test]
    fn zero_and_partial_grid_work_budgets_retain_closed_honest_meshes() {
        let body = crate::nurbs_tests::prism();
        for budget in [0, 1, 16, 65536] {
            let policy = RealizationPolicy {
                min_arc_segments: 256,
                quality: Some(TriangleQuality::new(0.1).with_max_grid_triangle_tests(budget)),
                ..Default::default()
            };
            let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
            let report = output.report.quality.as_ref().unwrap();
            assert!(
                report.grid_triangle_tests <= budget,
                "global planning work is capped"
            );
            assert!(output.report.within_tolerance());
            assert!(output.mesh.validate_deep().is_empty());
            assert!(output.mesh.boundary_loops().unwrap().is_empty());
            if budget < 16 {
                assert_eq!(report.grid_triangle_tests, 0);
                assert!(report.budget_exhausted);
                assert!(wall_shape(&output).0 > 0);
            }
        }
    }

    #[test]
    fn grid_shape_limit_is_soft_while_output_storage_limits_stay_hard() {
        let body = crate::nurbs_tests::prism();
        let mut policy = RealizationPolicy {
            min_arc_segments: 256,
            quality: Some(TriangleQuality::new(0.1)),
            max_edge_segments: 256,
            ..Default::default()
        };
        let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
        assert!(output.report.quality.as_ref().unwrap().grid_triangle_tests <= 1 << 20);
        policy.max_vertices = 8;
        assert!(matches!(
            body.realize(Placement3::IDENTITY, &policy),
            Err(RealizationError::Budget {
                storage: "vertices",
                ..
            })
        ));
    }

    #[test]
    fn impossible_cell_angle_does_not_trigger_unbounded_uniform_refinement() {
        let points = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.9848077, 0.17364818, 0.0],
            [0.9848077, 0.17364818, 0.0],
        ];
        let (_, ratios) = diagonal(points, [0.0, 0.0, 1.0], false);
        assert!(
            ratios.into_iter().any(|r| r > 2.5),
            "a ten-degree corner fixes an impossible angle target"
        );
        let body = flat_grids(&[points.map(|p| p.map(f64::from))]);
        let policy = RealizationPolicy {
            quality: Some(TriangleQuality::new(2.0)),
            ..Default::default()
        };
        let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
        let report = output.report.quality.unwrap();
        assert!(report.remaining_bad_triangles > 0);
        assert!(
            !report.budget_exhausted,
            "input limitation is distinct from a planning budget"
        );
        assert!(
            report.grid_triangle_tests <= 20,
            "one unchanged uniform-subdivision probe suffices"
        );
        assert_eq!(output.mesh.faces().count(), 2);
    }

    #[test]
    fn candidate_storage_is_checked_before_sampling() {
        let body = flat_grids(&[[
            [0.0, 0.0, 0.0],
            [0.02, 0.0, 0.0],
            [0.02, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]]);
        for (max_vertices, max_boundary_samples) in [(4, 100), (100, 8)] {
            let policy = RealizationPolicy {
                quality: Some(TriangleQuality::new(2.0)),
                max_vertices,
                max_boundary_samples,
                ..Default::default()
            };
            let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
            let report = output.report.quality.unwrap();
            assert_eq!(
                report.grid_triangle_tests, 4,
                "over-budget candidates are not sampled"
            );
            assert!(report.budget_exhausted);
            assert!(report.remaining_bad_triangles > 0);
            assert_eq!(output.mesh.vertices().count(), 4);
        }
    }

    #[test]
    fn grid_work_budget_is_shared_across_faces_and_includes_candidates() {
        let body = flat_grids(&[
            [
                [0.0, 0.0, 0.0],
                [0.02, 0.0, 0.0],
                [0.02, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            [
                [2.0, 0.0, 0.0],
                [2.02, 0.0, 0.0],
                [2.02, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
        ]);
        let policy = RealizationPolicy {
            quality: Some(TriangleQuality::new(2.0).with_max_grid_triangle_tests(52)),
            ..Default::default()
        };
        let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
        let report = output.report.quality.unwrap();
        assert_eq!(
            report.grid_triangle_tests, 52,
            "initial/candidate scans consume one shared budget"
        );
        assert!(report.budget_exhausted);
        assert!(report.remaining_bad_triangles > 0);
        assert!(output.report.edge_segments[1] > output.report.edge_segments[5]);
        assert!(output.mesh.validate_deep().is_empty());
    }

    #[test]
    fn default_and_zero_budget_keep_the_original_diagonal() {
        let body = flat_grids(&[[
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ]]);
        let keys = |output: &crate::realize::RealizedBody| {
            let mut keys: Vec<_> = output
                .mesh
                .faces()
                .map(|face| {
                    let mut vertices: Vec<_> = output
                        .mesh
                        .face_loop(face)
                        .map(|c| output.mesh.to_vertex(c).unwrap().index())
                        .collect();
                    vertices.sort_unstable();
                    vertices
                })
                .collect();
            keys.sort_unstable();
            keys
        };
        let default = body
            .realize(Placement3::IDENTITY, &RealizationPolicy::default())
            .unwrap();
        assert_eq!(default.report.edge_segments, [1, 1, 1, 1]);
        assert_eq!(
            keys(&default),
            alloc::vec![alloc::vec![0, 1, 2], alloc::vec![0, 2, 3]]
        );
        for budget in [0, 4] {
            let policy = RealizationPolicy {
                quality: Some(TriangleQuality::new(100.0).with_max_grid_triangle_tests(budget)),
                ..Default::default()
            };
            let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
            assert_eq!(output.mesh_vertices, default.mesh_vertices);
            assert_eq!(output.report.edge_segments, default.report.edge_segments);
            assert_eq!(
                output.report.max_surface_deviation,
                default.report.max_surface_deviation
            );
            assert_eq!(
                output.report.quality.as_ref().unwrap().grid_triangle_tests,
                budget
            );
            if budget == 0 {
                assert_eq!(keys(&output), keys(&default));
            } else {
                assert_eq!(
                    keys(&output),
                    alloc::vec![alloc::vec![0, 1, 3], alloc::vec![1, 2, 3]],
                    "quality chooses the better diagonal without extra samples"
                );
            }
        }
    }

    #[test]
    fn quality_does_not_accept_f32_collapsed_grid_boundaries() {
        let body = crate::nurbs_tests::prism();
        let policy = RealizationPolicy {
            quality: Some(TriangleQuality::new(0.1)),
            ..Default::default()
        };
        assert!(
            matches!(
                body.realize(Placement3::translate(1e15, 1e15, 1e15), &policy),
                Err(RealizationError::CollapsedEdge { .. })
                    | Err(RealizationError::DegenerateFace { .. })
            ),
            "f32 collapse remains an explicit numeric refusal"
        );
    }

    #[test]
    fn admitted_native_endpoint_offsets_are_used_by_probe_and_emission() {
        let mut parts = flat_grids(&[[
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ]])
        .into_parts();
        parts.vertices[1][1] = 3e-8;
        let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-6 }).unwrap();
        let policy = RealizationPolicy {
            geometry_gap: 1e-6,
            quality: Some(TriangleQuality::new(100.0).with_max_grid_triangle_tests(4)),
            ..Default::default()
        };
        let patch = GridPatch::prepare(&body, FaceId(0)).unwrap();
        let mut scratch = Scratch::default();
        let mut work = Work::default();
        let planned = scratch
            .scan(
                &body,
                FaceId(0),
                patch,
                Placement3::IDENTITY,
                &[1, 1, 1, 1],
                &policy,
                &mut work,
            )
            .unwrap()
            .unwrap();
        let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
        assert_eq!(
            output
                .mesh
                .vertex_position(output.mesh_vertices[1].0)
                .unwrap()[1],
            3e-8_f32,
            "the admitted native corner stays authoritative"
        );
        assert_eq!(
            planned.maximum,
            output
                .report
                .quality
                .as_ref()
                .unwrap()
                .max_radius_edge_ratio,
            "the planner measures the exact positions later emitted"
        );
    }
}
