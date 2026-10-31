// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Optional shape improvement with authoritative shared boundaries.
use super::*;
use crate::geometry::CurveSpan;
use exedra_math::{add, cross, dot, norm, scale, sub};
use exedra_triangulate::{BoundarySplits, RefineParams};

/// Optional triangle-shape improvement, independent of geometric accuracy.
///
/// Native edges are subdivided before face triangulation so neighboring faces
/// share every new boundary vertex. Planar interiors use constrained Delaunay
/// refinement; four-coedge cylindrical/conical strips gain rows along their rulings.
/// Other patch types retain their accuracy-driven sampling. Fixed boundaries,
/// input angles and budgets can prevent the requested shape: the final emitted
/// mesh is measured in [`TriangleQualityReport`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TriangleQuality {
    /// Upper bound on native boundary sample spacing in model units, using a
    /// conservative curve-speed bound. This is not a maximum triangle edge size.
    pub max_boundary_spacing: f64,
    /// Desired maximum circumradius divided by shortest triangle edge.
    /// At least one; 2.5 corresponds to a minimum angle of about 11.5 degrees.
    pub max_radius_edge_ratio: f64,
    /// Maximum generated planar interior vertices per completed attempt, across
    /// all faces. Native boundary samples and strip rows use the realization's
    /// storage budgets instead.
    pub max_steiner_points: u32,
    /// Maximum planar insertion attempts per attempt, including declined work.
    /// Initial CDT and the work inside one insertion are not counted here.
    pub max_candidate_insertions: u32,
}
impl TriangleQuality {
    /// Requests shared boundary spacing with a shape ratio of 2.5 and at most
    /// 4096 generated planar interior vertices.
    #[must_use]
    pub const fn new(max_boundary_spacing: f64) -> Self {
        Self {
            max_boundary_spacing,
            max_radius_edge_ratio: 2.5,
            max_steiner_points: 4096,
            max_candidate_insertions: 16384,
        }
    }
    /// Replaces the desired circumradius-to-shortest-edge ratio.
    #[must_use]
    pub const fn with_max_radius_edge_ratio(mut self, ratio: f64) -> Self {
        self.max_radius_edge_ratio = ratio;
        self
    }
    /// Replaces the shared planar interior-vertex budget.
    #[must_use]
    pub const fn with_max_steiner_points(mut self, maximum: u32) -> Self {
        self.max_steiner_points = maximum;
        self
    }
    /// Replaces the shared planar candidate-insertion attempt budget.
    #[must_use]
    pub const fn with_max_candidate_insertions(mut self, maximum: u32) -> Self {
        self.max_candidate_insertions = maximum;
        self
    }
    pub(super) fn valid(self) -> bool {
        self.max_boundary_spacing.is_finite()
            && self.max_boundary_spacing > 0.0
            && self.max_radius_edge_ratio.is_finite()
            && self.max_radius_edge_ratio >= 1.0
            && (self.max_radius_edge_ratio * self.max_radius_edge_ratio).is_finite()
    }
}

/// Shape evidence measured on all final f32 triangles, including unchanged
/// patch families. Accuracy achievement remains in [`RealizationReport`].
/// Refinement counters describe the retained attempt; discarded curve-retry
/// candidates are excluded.
#[derive(Clone, Debug, PartialEq)]
pub struct TriangleQualityReport {
    /// Requested circumradius-to-shortest-edge ratio.
    pub target_radius_edge_ratio: f64,
    /// Largest ratio in the final emitted mesh.
    pub max_radius_edge_ratio: f64,
    /// Final triangles whose ratio exceeds the requested target.
    pub remaining_bad_triangles: usize,
    /// Generated planar interior vertices; excludes boundary/strip vertices.
    pub planar_steiner_points: u32,
    /// Candidate planar insertions declined by fixed boundaries or predicates.
    pub declined_insertions: u32,
    /// Attempted planar insertions, including candidates that were declined.
    pub candidate_insertions: u32,
    /// Whether a planar refiner stopped with work pending at its point,
    /// candidate-insertion or representable-index limit. Other patch families
    /// may still miss the target.
    pub budget_exhausted: bool,
}
impl TriangleQualityReport {
    /// Whether every final emitted triangle meets the requested shape ratio.
    #[must_use]
    pub fn within_target(&self) -> bool {
        self.remaining_bad_triangles == 0
    }
}
#[derive(Default)]
pub(super) struct Work {
    points: u32,
    candidates: u32,
    declined: u32,
    exhausted: bool,
}

pub(super) fn report(
    q: TriangleQuality,
    points: &[[f32; 3]],
    triangles: &[[u32; 3]],
    work: Work,
) -> TriangleQualityReport {
    let mut maximum = 0.0_f64;
    let mut bad = 0;
    for tri in triangles {
        let p = tri.map(|i| points[i as usize].map(f64::from));
        let a = sub(p[1], p[0]);
        let b = sub(p[2], p[0]);
        let c = sub(p[2], p[1]);
        let lengths = [norm(a), norm(b), norm(c)];
        let shortest = lengths.into_iter().fold(f64::INFINITY, f64::min);
        let ratio = lengths[0] * lengths[1] * lengths[2] / (2.0 * norm(cross(a, b)) * shortest);
        maximum = maximum.max(ratio);
        bad += usize::from(ratio > q.max_radius_edge_ratio);
    }
    TriangleQualityReport {
        target_radius_edge_ratio: q.max_radius_edge_ratio,
        max_radius_edge_ratio: maximum,
        remaining_bad_triangles: bad,
        planar_steiner_points: work.points,
        declined_insertions: work.declined,
        candidate_insertions: work.candidates,
        budget_exhausted: work.exhausted,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "ceiling is checked against the u32 limit before conversion"
)]
fn count(length: f64, spacing: f64, edge: EdgeId, maximum: u32) -> Result<u32, RealizationError> {
    let required = libm::ceil(length / spacing).max(1.0);
    if !required.is_finite() || required > f64::from(maximum) {
        return Err(RealizationError::Sampling {
            edge,
            error: CircularSamplingError::ToleranceBudgetExceeded {
                required: if required >= f64::from(u32::MAX) {
                    u32::MAX
                } else {
                    required as u32
                },
                maximum,
            },
        });
    }
    Ok(required as u32)
}

/// A four-coedge strip's two normalized ruling chains. Angular rims are in
/// the Strip itself; periodic seams deliberately reuse the same native edge.
#[derive(Copy, Clone)]
pub(super) struct Rows {
    rim: EdgeId,
    sides: [Coedge; 2],
}
impl Rows {
    fn intervals(self, counts: &[u32]) -> (usize, usize) {
        (
            counts[self.rim.index()] as usize,
            counts[self.sides[0].edge.index()] as usize,
        )
    }
    pub(super) fn interior_count(self, counts: &[u32]) -> usize {
        let (u, v) = self.intervals(counts);
        u.saturating_sub(1).saturating_mul(v.saturating_sub(1))
    }
    pub(super) fn triangle_count(self, counts: &[u32]) -> usize {
        let (u, v) = self.intervals(counts);
        u.saturating_mul(v).saturating_mul(2)
    }
}

pub(super) fn prepare<F, E>(
    body: &Body<F, E>,
    patches: &[Patch],
    counts: &mut [u32],
    groups: &mut Groups,
    policy: &RealizationPolicy,
) -> Result<Vec<Option<Rows>>, RealizationError> {
    let Some(q) = policy.quality else {
        return Ok(Vec::new());
    };
    let mut rows = alloc::vec![None; patches.len()];
    for (i, edge) in body.edges().iter().enumerate() {
        if counts[i] == 0 {
            continue;
        }
        let (curve, range) = edge.regular().expect("active regular edge");
        let speed = CurveSpan::new(curve.clone(), range, false)
            .map_err(RealizationError::Frame)?
            .speed();
        counts[i] = counts[i].max(count(
            speed,
            q.max_boundary_spacing,
            EdgeId(crate::usize_to_u32(i)),
            policy.max_edge_segments,
        )?);
    }
    groups.synchronize(counts);
    for (i, patch) in patches.iter().enumerate() {
        let Patch::Strip(p) = *patch else {
            continue;
        };
        let face = &body.faces()[i];
        if p.variable_axial_rulings || face.loops.len() != 1 {
            continue;
        }
        let uses = &body
            .boundary_loop(face.loops[0])
            .expect("validated")
            .coedges;
        if uses.len() != 4 {
            continue;
        }
        let Some(ai) = uses.iter().position(|&c| c == p.a) else {
            continue;
        };
        let Some(bi) = uses.iter().position(|&c| c == p.b) else {
            continue;
        };
        if (ai + 2) % 4 != bi {
            continue;
        }
        let left = uses[(bi + 1) % 4];
        let right = uses[(ai + 1) % 4];
        let sides = [
            Coedge {
                edge: left.edge,
                reversed: !left.reversed,
            },
            right,
        ];
        let n = counts[p.a.edge.index()];
        let a0 = body.edge_point(p.a.edge, 0.0).expect("validated");
        let a1 = body
            .edge_point(p.a.edge, 1.0 / f64::from(n))
            .expect("validated");
        let chord = distance(a0, a1) * p.radius_scale;
        let edge = &body.edges()[left.edge.index()];
        let length = distance(
            body.vertices()[edge.start.index()],
            body.vertices()[edge.end.index()],
        );
        let aspect = libm::sqrt(4.0 * q.max_radius_edge_ratio * q.max_radius_edge_ratio - 1.0);
        let target = q.max_boundary_spacing.min(chord * aspect);
        let needed = count(length, target, left.edge, policy.max_edge_segments)?;
        counts[left.edge.index()] = counts[left.edge.index()].max(needed);
        counts[right.edge.index()] = counts[right.edge.index()].max(needed);
        groups.join(left.edge.index(), right.edge.index());
        rows[i] = Some(Rows {
            rim: p.a.edge,
            sides,
        });
    }
    groups.synchronize(counts);
    Ok(rows)
}

pub(super) fn planar<F, E>(
    body: &Body<F, E>,
    id: FaceId,
    placement: Placement3,
    frame: Placement3,
    input: &PolygonInput<'_>,
    indices: &mut Vec<u32>,
    emitted: &mut Vec<[f32; 3]>,
    sources: &mut Vec<VertexSource>,
    displacement: &mut f64,
    q: TriangleQuality,
    max_vertices: u32,
    work: &mut Work,
) -> Result<Vec<[u32; 3]>, RealizationError> {
    let room = (max_vertices as usize)
        .saturating_sub(emitted.len())
        .min(q.max_steiner_points.saturating_sub(work.points) as usize);
    let params = RefineParams::new(q.max_radius_edge_ratio)
        .with_max_steiner_points(crate::usize_to_u32(room))
        .with_max_candidate_insertions(q.max_candidate_insertions.saturating_sub(work.candidates))
        .with_boundary_splits(BoundarySplits::Forbidden);
    let refined = exedra_triangulate::refine(input, &params)
        .map_err(|error| RealizationError::Arrangement { face: id, error })?;
    work.points += refined.stats.steiner_points;
    work.candidates += refined.stats.candidate_insertions;
    work.declined = work
        .declined
        .saturating_add(refined.stats.declined_insertions);
    work.exhausted |= refined.stats.budget_exhausted;
    let local_frame = body.planar_frame(id, [0.0; 2]).expect("planar support");
    for &[x, y] in &refined.points[indices.len()..] {
        let local = local_frame.transform_point([x, y, 0.0]);
        let uv = body.faces()[id.index()]
            .surface
            .parameters(local)
            .ok_or(RealizationError::DegenerateFace { face: id })?;
        // The projected polygon uses placed emitted boundary points. Map its
        // chart back through the unplaced support, rather than retaining a
        // world-space coordinate as native UV.
        let placed = frame.transform_point([x, y, 0.0]);
        let p = narrow(placed);
        if !p.iter().all(|v| v.is_finite()) {
            return Err(RealizationError::NonFiniteFaceSample { face: id, uv });
        }
        *displacement = displacement
            .max(distance(p.map(f64::from), placed) + placement_roundoff(placement, local));
        indices.push(crate::usize_to_u32(emitted.len()));
        emitted.push(p);
        sources.push(VertexSource::Face { face: id, uv });
    }
    Ok(refined.triangles)
}

#[derive(Default)]
pub(super) struct Scratch {
    previous: Vec<u32>,
    row: Vec<u32>,
    ends: Vec<[[f64; 3]; 2]>,
}
impl Rows {
    pub(super) fn emit<F, E>(
        self,
        body: &Body<F, E>,
        id: FaceId,
        patch: Strip,
        placement: Placement3,
        counts: &[u32],
        max_vertices: u32,
        chains: &Chains,
        scratch: &mut Scratch,
        emitted: &mut Vec<[f32; 3]>,
        sources: &mut Vec<VertexSource>,
        displacement: &mut f64,
        region: u32,
        triangles: &mut Vec<[u32; 3]>,
        regions: &mut Vec<u32>,
        faces: &mut Vec<FaceId>,
    ) -> Result<(), RealizationError> {
        let required = emitted.len().saturating_add(self.interior_count(counts));
        if required > max_vertices as usize {
            return Err(RealizationError::Budget {
                storage: "vertices",
                required,
                maximum: max_vertices,
            });
        }
        let (nu, nv) = self.intervals(counts);
        let a = chains.get(patch.a.edge);
        let b = chains.get(patch.b.edge);
        let at = |j: usize| a[if patch.a.reversed { nu - j } else { j }];
        let bt = |j: usize| b[if patch.b.reversed { j } else { nu - j }];
        scratch.previous.clear();
        scratch.previous.extend((0..=nu).map(at));
        scratch.ends.clear();
        scratch.ends.reserve(nu + 1);
        for j in 0..=nu {
            let f = j as f64 / nu as f64;
            scratch.ends.push([
                body.edge_point(patch.a.edge, if patch.a.reversed { 1.0 - f } else { f })
                    .expect("edge sample"),
                body.edge_point(patch.b.edge, if patch.b.reversed { f } else { 1.0 - f })
                    .expect("edge sample"),
            ]);
        }
        for k in 1..=nv {
            scratch.row.clear();
            scratch.row.reserve(nu + 1);
            for j in 0..=nu {
                let vertex = if k == nv {
                    bt(j)
                } else if j == 0 || j == nu {
                    let side = self.sides[usize::from(j == nu)];
                    let chain = chains.get(side.edge);
                    chain[if side.reversed { nv - k } else { k }]
                } else {
                    let [lo, hi] = scratch.ends[j];
                    let local = add(lo, scale(sub(hi, lo), k as f64 / nv as f64));
                    let record = &body.faces()[id.index()];
                    let mut uv = record
                        .surface
                        .parameters(local)
                        .ok_or(RealizationError::DegenerateFace { face: id })?;
                    // Inverse supports return principal angles. Retain the
                    // face's authored turn instead, so interior correspondence
                    // stays in the same chart as its shared boundary chains.
                    for ((value, period), range) in uv
                        .iter_mut()
                        .zip(record.surface.periods())
                        .zip([record.domain.u, record.domain.v])
                    {
                        if let Some(period) = period {
                            *value += period * libm::ceil((range.bounds()[0] - *value) / period);
                        }
                    }
                    let exact = placement.transform_point(local);
                    let p = narrow(exact);
                    if !p.iter().all(|v| v.is_finite()) {
                        return Err(RealizationError::NonFiniteFaceSample { face: id, uv });
                    }
                    *displacement = displacement.max(
                        distance(p.map(f64::from), exact) + placement_roundoff(placement, local),
                    );
                    let index = crate::usize_to_u32(emitted.len());
                    emitted.push(p);
                    sources.push(VertexSource::Face { face: id, uv });
                    index
                };
                scratch.row.push(vertex);
            }
            for j in 0..nu {
                let uv = body.faces()[id.index()]
                    .surface
                    .parameters(scratch.ends[j][0])
                    .ok_or(RealizationError::DegenerateFace { face: id })?;
                let normal = crate::geometry::vector(
                    placement,
                    body.evaluate_support(id, uv)
                        .ok_or(RealizationError::DegenerateFace { face: id })?
                        .normal,
                );
                let c = [
                    scratch.previous[j],
                    scratch.previous[j + 1],
                    scratch.row[j + 1],
                    scratch.row[j],
                ];
                for tri in [[c[0], c[1], c[2]], [c[0], c[2], c[3]]] {
                    let p = tri.map(|i| emitted[i as usize].map(f64::from));
                    if dot(cross(sub(p[1], p[0]), sub(p[2], p[0])), normal) <= 0.0 {
                        return Err(RealizationError::DegenerateFace { face: id });
                    }
                    emit(triangles, tri, id, region, emitted, regions, faces)?;
                }
            }
            core::mem::swap(&mut scratch.previous, &mut scratch.row);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{ValidationPolicy, tests::cylinder_parts};

    #[test]
    fn periodic_rows_are_closed_and_keep_native_coordinates_after_placement() {
        let body: Body =
            Body::try_from_parts(cylinder_parts(), &ValidationPolicy { max_gap: 1e-12 }).unwrap();
        let placement = Placement3::rotate_y_then_translate(0.37, 3.0, -2.0, 4.0);
        let policy = RealizationPolicy {
            quality: Some(TriangleQuality::new(0.3)),
            ..Default::default()
        };
        let out = body.realize(placement, &policy).unwrap();
        assert!(out.mesh.boundary_loops().unwrap().is_empty());
        assert!(out.report.within_tolerance());
        assert!(out.report.quality.as_ref().unwrap().within_target());
        assert!(
            out.report.edge_segments[2] > 1,
            "shared periodic seam gains rows"
        );
        let mut interiors = 0;
        for &(vertex, source) in &out.mesh_vertices {
            if let VertexSource::Face { face, uv } = source {
                interiors += 1;
                let record = &body.faces()[face.index()];
                for (value, range) in uv.into_iter().zip([record.domain.u, record.domain.v]) {
                    let [lo, hi] = range.bounds();
                    assert!(
                        value >= lo - 1e-12 && value <= hi + 1e-12,
                        "interior UV remains in the bounded native face domain"
                    );
                }
                let expected =
                    placement.transform_point(body.evaluate_support(face, uv).unwrap().point);
                let actual = out.mesh.vertex_position(vertex).unwrap().map(f64::from);
                assert!(distance(actual, expected) <= out.report.max_vertex_displacement + 1e-12);
            }
        }
        assert!(interiors > 0);
        assert!(out.report.quality.as_ref().unwrap().planar_steiner_points > 0);
    }

    #[test]
    fn zero_point_and_candidate_budgets_keep_a_valid_honest_output() {
        let body: Body =
            Body::try_from_parts(cylinder_parts(), &ValidationPolicy { max_gap: 1e-12 }).unwrap();
        let policy = RealizationPolicy {
            quality: Some(
                TriangleQuality::new(0.3)
                    .with_max_steiner_points(0)
                    .with_max_candidate_insertions(0),
            ),
            ..Default::default()
        };
        let out = body.realize(Placement3::IDENTITY, &policy).unwrap();
        let report = out.report.quality.unwrap();
        assert_eq!(report.planar_steiner_points, 0);
        assert_eq!(report.candidate_insertions, 0);
        assert!(report.budget_exhausted);
        assert!(!report.within_target());
        assert!(out.mesh.boundary_loops().unwrap().is_empty());
    }

    #[test]
    fn planar_work_budgets_are_shared_across_both_caps() {
        let body: Body =
            Body::try_from_parts(cylinder_parts(), &ValidationPolicy { max_gap: 1e-12 }).unwrap();
        for limit in [1, 7] {
            let policy = RealizationPolicy {
                quality: Some(
                    TriangleQuality::new(0.3)
                        .with_max_steiner_points(limit)
                        .with_max_candidate_insertions(limit),
                ),
                ..Default::default()
            };
            let out = body.realize(Placement3::IDENTITY, &policy).unwrap();
            let report = out.report.quality.unwrap();
            assert!(report.planar_steiner_points <= limit);
            assert!(report.candidate_insertions <= limit);
            assert!(report.budget_exhausted);
            assert!(out.mesh.boundary_loops().unwrap().is_empty());
        }
    }

    #[test]
    fn quality_storage_and_edge_limits_refuse_before_emission() {
        let body: Body =
            Body::try_from_parts(cylinder_parts(), &ValidationPolicy { max_gap: 1e-12 }).unwrap();
        let mut policy = RealizationPolicy {
            quality: Some(TriangleQuality::new(0.01)),
            max_vertices: 8,
            ..Default::default()
        };
        assert!(matches!(
            body.realize(Placement3::IDENTITY, &policy),
            Err(RealizationError::Budget {
                storage: "vertices",
                ..
            })
        ));
        policy.max_edge_segments = 4;
        assert!(matches!(
            body.realize(Placement3::IDENTITY, &policy),
            Err(RealizationError::Sampling { .. })
        ));
        policy.quality = Some(TriangleQuality::new(f64::NAN));
        assert_eq!(
            body.realize(Placement3::IDENTITY, &policy).unwrap_err(),
            RealizationError::InvalidPolicy
        );
    }
}
