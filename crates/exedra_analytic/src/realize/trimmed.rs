// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit chart polygons and conforming, boundary-preserving refinement.

use super::{
    Body, Chains, FaceId, RealizationError, RealizationPolicy, VertexSource, distance, emit,
    placement_roundoff,
};
use crate::body::{Face, Sense};
use crate::geometry::Surface;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};
use exedra_math::{Placement3, cross, dot, narrow, sub};
use exedra_triangulate::{PolygonInput, TriParams, triangulate_preserving_boundary};

#[derive(Default)]
pub(super) struct Work {
    pub batches: usize,
    pub triangle_tests: usize,
    pub vertices: usize,
    pub normal_scratch_bytes: usize,
}

pub(super) struct TriangleNormals {
    pub triangle: u32,
    pub normals: [[f32; 3]; 3],
}

/// Uniform bounds on the norms of `S_uu`, `S_uv`, `S_vv` throughout a face domain.
fn hessian<F>(face: &Face<F>) -> [f64; 3] {
    let bound = match &face.surface {
        Surface::Plane(_) => [0.0; 3],
        Surface::Cylinder(c) => [c.radius(), 0.0, 0.0],
        Surface::Cone(c) => [
            face.domain
                .v
                .bounds()
                .into_iter()
                .filter_map(|v| c.radius_at(v))
                .fold(0.0, f64::max),
            c.slope().abs(),
            0.0,
        ],
        Surface::Sphere(s) => [s.radius(); 3],
        Surface::Torus(t) => [
            t.major_radius() + t.minor_radius(),
            t.minor_radius(),
            t.minor_radius(),
        ],
        Surface::Ruled(r) => {
            let [a, b] = r.curves();
            [
                a.second_bound().max(b.second_bound()),
                a.speed() + b.speed(),
                0.0,
            ]
        }
    };
    bound.map(|v| {
        if v == 0.0 {
            0.0
        } else {
            (v * (1.0 + 16.0 * crate::geometry::FRAME_TOLERANCE)).next_up()
        }
    })
}

fn deviation(points: &[[f64; 2]], triangle: [u32; 3], h: [f64; 3]) -> f64 {
    let p = triangle.map(|i| points[i as usize]);
    let width: [f64; 2] = core::array::from_fn(|axis| {
        p.iter().map(|v| v[axis]).fold(f64::NEG_INFINITY, f64::max)
            - p.iter().map(|v| v[axis]).fold(f64::INFINITY, f64::min)
    });
    // Taylor's remainder at any point of the parameter triangle, followed by
    // barycentric interpolation. The affine terms cancel exactly.
    let error = 0.5
        * (h[0] * width[0] * width[0]
            + 2.0 * h[1] * width[0] * width[1]
            + h[2] * width[1] * width[1]);
    if error == 0.0 { 0.0 } else { error.next_up() }
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "finite positive segment counts are checked against the u32 policy limit before conversion"
)]
pub(super) fn prepare_counts<F, E>(
    body: &Body<F, E>,
    patches: &[super::Patch],
    counts: &mut [u32],
    policy: &RealizationPolicy,
) -> Result<(), RealizationError> {
    for (i, patch) in patches.iter().enumerate() {
        if !matches!(patch, super::Patch::Trimmed) {
            continue;
        }
        let id = FaceId(crate::usize_to_u32(i));
        let face = &body.faces()[i];
        if [face.domain.u, face.domain.v]
            .into_iter()
            .zip(face.surface.periods())
            .any(|(range, period)| {
                period.is_some_and(|p| range.span() > p * (1.0 + 64.0 * f64::EPSILON))
            })
        {
            return Err(RealizationError::UnsupportedSurfaceTrim { face: id });
        }
        let h = hessian(face);
        for &loop_id in &face.loops {
            for (j, coedge) in body
                .boundary_loop(loop_id)
                .expect("validated loop")
                .coedges
                .iter()
                .enumerate()
            {
                let d = body
                    .trim_curve(loop_id, j)
                    .expect("explicit trim")
                    .derivative_bounds();
                let metric = h[0] * d[0] * d[0] + 2.0 * h[1] * d[0] * d[1] + h[2] * d[1] * d[1];
                let required = libm::ceil(libm::sqrt(16.0 * metric / policy.tolerance)).max(1.0);
                if !required.is_finite() || required > f64::from(policy.max_edge_segments) {
                    return Err(RealizationError::SurfaceSampling {
                        face: id,
                        intervals: [
                            if required > f64::from(u32::MAX) {
                                u32::MAX
                            } else {
                                required as u32
                            },
                            0,
                        ],
                        maximum: policy.max_edge_segments,
                    });
                }
                counts[coedge.edge.index()] = counts[coedge.edge.index()].max(required as u32);
            }
        }
    }
    Ok(())
}
fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}
fn uv_average(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    core::array::from_fn(|i| 0.5 * a[i] + 0.5 * b[i])
}

pub(super) fn emit_face<F, E>(
    body: &Body<F, E>,
    id: FaceId,
    placement: Placement3,
    policy: &RealizationPolicy,
    chains: &Chains,
    emitted: &mut Vec<[f32; 3]>,
    sources: &mut Vec<VertexSource>,
    displacement: &mut f64,
    region: u32,
    triangles: &mut Vec<[u32; 3]>,
    regions: &mut Vec<u32>,
    faces: &mut Vec<FaceId>,
    normals: &mut Vec<TriangleNormals>,
    work: &mut Work,
) -> Result<f64, RealizationError> {
    let face = &body.faces()[id.index()];
    let initial_vertices = emitted.len();
    let mut rings = Vec::with_capacity(face.loops.len());
    let mut indices = Vec::new();
    for &loop_id in &face.loops {
        let mut points = Vec::new();
        let mut mapped = Vec::new();
        for (occurrence, coedge) in body
            .boundary_loop(loop_id)
            .expect("validated loop")
            .coedges
            .iter()
            .enumerate()
        {
            let chain = chains.get(coedge.edge);
            let trim = body
                .trim_curve(loop_id, occurrence)
                .expect("complete explicit trims");
            for k in 0..chain.len() - 1 {
                let j = if coedge.reversed {
                    chain.len() - 1 - k
                } else {
                    k
                };
                let uv = trim
                    .evaluate(j as f64 / (chain.len() - 1) as f64)
                    .ok_or(RealizationError::UnsupportedSurfaceTrim { face: id })?;
                points.push(uv);
                mapped.push(chain[j]);
            }
        }
        // The triangulator always consumes forward-chart CCW outers/CW holes.
        if face.sense == Sense::Reverse {
            points.reverse();
            mapped.reverse();
        }
        indices.extend(mapped);
        rings.push(points);
    }
    let holes: Vec<_> = rings.iter().skip(1).map(Vec::as_slice).collect();
    let result = triangulate_preserving_boundary(
        &PolygonInput {
            outer: &rings[0],
            holes: &holes,
        },
        &TriParams::constrained_delaunay(),
    )
    .map_err(|error| RealizationError::Arrangement { face: id, error })?;
    let mut points: Vec<_> = rings.into_iter().flatten().collect();
    let mut cover = result.triangles;
    let h = hessian(face);
    let target = policy.tolerance * 0.25;
    // All chart boundary samples stay authoritative. Only interior edges split;
    // every incident triangle sees the same split in a deterministic batch.
    // This preserves shared native chains, seam copies and hole boundaries.
    let mut bad = Vec::new();
    loop {
        work.triangle_tests += cover.len();
        bad.clear();
        bad.extend(
            cover
                .iter()
                .enumerate()
                .filter_map(|(i, &t)| (deviation(&points, t, h) > target).then_some(i)),
        );
        if bad.is_empty() {
            break;
        }
        work.batches += 1;
        let mut incidence = BTreeMap::<(u32, u32), u8>::new();
        for t in &cover {
            for i in 0..3 {
                *incidence.entry(edge(t[i], t[(i + 1) % 3])).or_default() += 1;
            }
        }
        let mut split = BTreeSet::new();
        let mut centers = BTreeSet::new();
        for &i in &bad {
            let t = cover[i];
            let candidate = (0..3)
                .map(|k| edge(t[k], t[(k + 1) % 3]))
                .filter(|e| incidence[e] == 2)
                .max_by(|a, b| {
                    let metric = |e: &(u32, u32)| {
                        let d: [f64; 2] = core::array::from_fn(|axis| {
                            points[e.0 as usize][axis] - points[e.1 as usize][axis]
                        });
                        h[0] * d[0] * d[0] + 2.0 * h[1] * (d[0] * d[1]).abs() + h[2] * d[1] * d[1]
                    };
                    metric(a).total_cmp(&metric(b)).then_with(|| b.cmp(a))
                });
            if let Some(e) = candidate {
                split.insert(e);
            } else {
                centers.insert(i);
            }
        }
        let mut midpoint = BTreeMap::new();
        for &(a, b) in &split {
            let uv = uv_average(points[a as usize], points[b as usize]);
            let index = push_point(
                body,
                id,
                uv,
                placement,
                policy,
                emitted,
                sources,
                displacement,
                &mut points,
                &mut indices,
            )?;
            midpoint.insert((a, b), index);
        }
        let maximum_triangles = cover
            .len()
            .saturating_add(split.len().saturating_mul(2))
            .saturating_add(centers.len().saturating_mul(2));
        if maximum_triangles > u32::MAX as usize {
            return Err(RealizationError::Budget {
                storage: "triangles",
                required: maximum_triangles,
                maximum: u32::MAX,
            });
        }
        super::check_table_capacity::<[u32; 3]>("triangles", maximum_triangles)?;
        let mut refined = Vec::with_capacity(maximum_triangles);
        for (i, t) in cover.into_iter().enumerate() {
            let mids: [Option<u32>; 3] =
                core::array::from_fn(|k| midpoint.get(&edge(t[k], t[(k + 1) % 3])).copied());
            if mids.iter().all(Option::is_none) && !centers.contains(&i) {
                refined.push(t);
                continue;
            }
            let marked = mids.iter().filter(|m| m.is_some()).count();
            if marked == 3 {
                let [ab, bc, ca] = mids.map(|m| m.expect("three marked edges"));
                refined.extend([[t[0], ab, ca], [ab, t[1], bc], [ca, bc, t[2]], [ab, bc, ca]]);
            } else if marked == 2 {
                let k = (0..3)
                    .find(|&k| mids[k].is_some() && mids[(k + 1) % 3].is_some())
                    .expect("adjacent marked edges");
                let [a, b, c] = [t[k], t[(k + 1) % 3], t[(k + 2) % 3]];
                let ab = mids[k].expect("marked");
                let bc = mids[(k + 1) % 3].expect("marked");
                refined.extend([[ab, b, bc], [a, ab, c], [ab, bc, c]]);
            } else if marked == 1 {
                let k = mids
                    .iter()
                    .position(Option::is_some)
                    .expect("one marked edge");
                let [a, b, c] = [t[k], t[(k + 1) % 3], t[(k + 2) % 3]];
                let m = mids[k].expect("marked");
                refined.extend([[a, m, c], [m, b, c]]);
            } else {
                let p = t.map(|j| points[j as usize]);
                let uv = core::array::from_fn(|axis| {
                    p[0][axis] / 3.0 + p[1][axis] / 3.0 + p[2][axis] / 3.0
                });
                let center = push_point(
                    body,
                    id,
                    uv,
                    placement,
                    policy,
                    emitted,
                    sources,
                    displacement,
                    &mut points,
                    &mut indices,
                )?;
                refined.extend([
                    [t[0], t[1], center],
                    [t[1], t[2], center],
                    [t[2], t[0], center],
                ]);
            }
        }
        cover = refined;
    }
    super::check_table_capacity::<[f32; 3]>("trim parameter normals", points.len())?;
    let mut parameter_normals = Vec::with_capacity(points.len());
    for &parameter in &points {
        let field = body
            .evaluate_support(id, parameter)
            .and_then(|e| crate::geometry::transformed_normal(placement, e.normal))
            .ok_or(RealizationError::DegenerateFace { face: id })?;
        let normal = narrow(field);
        if !normal.iter().all(|value| value.is_finite()) {
            return Err(RealizationError::DegenerateFace { face: id });
        }
        parameter_normals.push(normal);
    }
    let mut bound = 0.0_f64;
    for t in cover {
        bound = bound.max(deviation(&points, t, h));
        let mut corners = t.map(|j| indices[j as usize]);
        if face.sense == Sense::Reverse {
            corners.swap(1, 2);
        }
        let p = corners.map(|j| emitted[j as usize].map(f64::from));
        let uv =
            core::array::from_fn(|axis| t.iter().map(|&j| points[j as usize][axis] / 3.0).sum());
        let normal = body
            .evaluate_support(id, uv)
            .ok_or(RealizationError::DegenerateFace { face: id })?
            .normal;
        if dot(
            cross(sub(p[1], p[0]), sub(p[2], p[0])),
            placement.transform_vector(normal),
        ) <= 0.0
        {
            return Err(RealizationError::DegenerateFace { face: id });
        }
        let required = triangles.len().saturating_add(1);
        if required > u32::MAX as usize {
            return Err(RealizationError::Budget {
                storage: "triangles",
                required,
                maximum: u32::MAX,
            });
        }
        let mut corner_normals = t.map(|j| parameter_normals[j as usize]);
        if face.sense == Sense::Reverse {
            corner_normals.swap(1, 2);
        }
        let normal_corners = normals.len().saturating_add(1).saturating_mul(3);
        if normal_corners > u32::MAX as usize {
            return Err(RealizationError::Budget {
                storage: "trim corner normals",
                required: normal_corners,
                maximum: u32::MAX,
            });
        }
        super::check_table_capacity::<TriangleNormals>(
            "trim triangle normals",
            normals.len().saturating_add(1),
        )?;
        normals.push(TriangleNormals {
            triangle: crate::usize_to_u32(triangles.len()),
            normals: corner_normals,
        });
        emit(triangles, corners, id, region, emitted, regions, faces)?;
    }
    work.normal_scratch_bytes = work.normal_scratch_bytes.max(
        parameter_normals.capacity() * size_of::<[f32; 3]>()
            + normals.capacity() * size_of::<TriangleNormals>(),
    );
    work.vertices += emitted.len() - initial_vertices;
    Ok(bound)
}

fn push_point<F, E>(
    body: &Body<F, E>,
    id: FaceId,
    uv: [f64; 2],
    placement: Placement3,
    policy: &RealizationPolicy,
    emitted: &mut Vec<[f32; 3]>,
    sources: &mut Vec<VertexSource>,
    displacement: &mut f64,
    points: &mut Vec<[f64; 2]>,
    indices: &mut Vec<u32>,
) -> Result<u32, RealizationError> {
    let required_parameters = points.len().saturating_add(1);
    if required_parameters > u32::MAX as usize {
        return Err(RealizationError::Budget {
            storage: "face parameters",
            required: required_parameters,
            maximum: u32::MAX,
        });
    }
    let required = emitted.len().saturating_add(1);
    if required > policy.max_vertices as usize {
        return Err(RealizationError::Budget {
            storage: "vertices",
            required,
            maximum: policy.max_vertices,
        });
    }
    let p = body
        .evaluate_support(id, uv)
        .ok_or(RealizationError::NonFiniteFaceSample { face: id, uv })?
        .point;
    let exact = placement.transform_point(p);
    let value = narrow(exact);
    if !value.iter().all(|v| v.is_finite()) {
        return Err(RealizationError::NonFiniteFaceSample { face: id, uv });
    }
    *displacement =
        displacement.max(distance(value.map(f64::from), exact) + placement_roundoff(placement, p));
    let local = crate::usize_to_u32(points.len());
    let mapped = crate::usize_to_u32(emitted.len());
    emitted.push(value);
    sources.push(VertexSource::Face { face: id, uv });
    points.push(uv);
    indices.push(mapped);
    Ok(local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{
        BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, LoopTrims, TrimCurve,
        ValidationPolicy, VertexId,
    };
    use crate::geometry::{Curve, ParameterDomain, ParameterRange};
    use alloc::vec;
    #[test]
    fn split_cylinder_seams_keep_optimized_sampling_and_authoritative_vertices() {
        let mut parts = crate::body::trims::tests::cylinder_parts();
        parts.vertices.push([2.0, 0.0, 1.5]);
        let seam = parts.edges[2].regular().unwrap().0.clone();
        parts.edges[2] = Edge::new(
            seam.clone(),
            ParameterRange::new(0.0, 0.5).unwrap(),
            VertexId(0),
            VertexId(2),
            (),
        );
        parts.edges.push(Edge::new(
            seam,
            ParameterRange::new(0.5, 1.0).unwrap(),
            VertexId(2),
            VertexId(1),
            (),
        ));
        parts.loops[0].coedges = vec![
            Coedge {
                edge: EdgeId(0),
                reversed: false,
            },
            Coedge {
                edge: EdgeId(2),
                reversed: false,
            },
            Coedge {
                edge: EdgeId(3),
                reversed: false,
            },
            Coedge {
                edge: EdgeId(1),
                reversed: true,
            },
            Coedge {
                edge: EdgeId(3),
                reversed: true,
            },
            Coedge {
                edge: EdgeId(2),
                reversed: true,
            },
        ];
        let tau = core::f64::consts::TAU;
        let line = |start, end| TrimCurve::Line { start, end };
        parts.trims[0].curves = vec![
            line([0.0, 0.0], [tau, 0.0]),
            line([tau, 0.0], [tau, 1.5]),
            line([tau, 1.5], [tau, 3.0]),
            line([0.0, 3.0], [tau, 3.0]),
            line([0.0, 1.5], [0.0, 3.0]),
            line([0.0, 0.0], [0.0, 1.5]),
        ];
        let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-10 }).unwrap();
        let output = body
            .realize(
                Placement3::IDENTITY,
                &RealizationPolicy {
                    tolerance: 0.05,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(output.report.within_tolerance(), "{:?}", output.report);
        assert_eq!(output.report.max_surface_deviation, 0.0);
        assert_eq!(output.report.work.trim_refinement_batches, 0);
        assert!(
            output
                .mesh_vertices
                .iter()
                .any(|(_, s)| *s == VertexSource::Vertex(VertexId(2)))
        );
        assert!(output.mesh.validate_deep().is_empty());
        assert!(output.mesh.boundary_loops().unwrap().is_empty());
        let mut constrained = RealizationPolicy {
            tolerance: 0.05,
            max_vertices: 20,
            ..Default::default()
        };
        assert!(matches!(
            body.realize(Placement3::IDENTITY, &constrained),
            Err(RealizationError::Budget { .. })
        ));
        constrained.max_vertices = 1 << 20;
        constrained.max_edge_segments = 8;
        assert!(matches!(
            body.realize(Placement3::IDENTITY, &constrained),
            Err(RealizationError::Sampling { .. } | RealizationError::SurfaceSampling { .. })
        ));
    }
    fn spherical_rectangle() -> Body {
        let surface = Surface::sphere(Placement3::IDENTITY, 2.0).unwrap();
        let (u, v) = (
            ParameterRange::new(0.2, 1.0).unwrap(),
            ParameterRange::new(-0.3, 0.4).unwrap(),
        );
        let corners = [[0.2, -0.3], [1.0, -0.3], [1.0, 0.4], [0.2, 0.4]];
        let vertices = corners
            .map(|uv| surface.evaluate(uv).unwrap().point)
            .to_vec();
        let latitude = |lat: f64| {
            Curve::circle(
                Placement3::translate(0.0, 0.0, 2.0 * libm::sin(lat)),
                2.0 * libm::cos(lat),
            )
            .unwrap()
        };
        let meridian = |angle: f64| {
            Curve::circle(
                Placement3::from_axes(
                    [libm::cos(angle), libm::sin(angle), 0.0],
                    [0.0, 0.0, 1.0],
                    [libm::sin(angle), -libm::cos(angle), 0.0],
                    [0.0; 3],
                ),
                2.0,
            )
            .unwrap()
        };
        let edges = vec![
            Edge::new(latitude(-0.3), u, VertexId(0), VertexId(1), ()),
            Edge::new(meridian(1.0), v, VertexId(1), VertexId(2), ()),
            Edge::new(latitude(0.4), u, VertexId(3), VertexId(2), ()),
            Edge::new(meridian(0.2), v, VertexId(0), VertexId(3), ()),
        ];
        let loop_ = BoundaryLoop {
            coedges: vec![
                Coedge {
                    edge: EdgeId(0),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(1),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(2),
                    reversed: true,
                },
                Coedge {
                    edge: EdgeId(3),
                    reversed: true,
                },
            ],
        };
        Body::try_from_parts(
            BodyParts {
                vertices,
                edges,
                loops: vec![loop_],
                faces: vec![Face {
                    surface,
                    domain: ParameterDomain { u, v },
                    sense: Sense::Forward,
                    loops: vec![LoopId(0)],
                    role: (),
                }],
                trims: vec![LoopTrims {
                    loop_id: LoopId(0),
                    curves: vec![
                        TrimCurve::Line {
                            start: corners[0],
                            end: corners[1],
                        },
                        TrimCurve::Line {
                            start: corners[1],
                            end: corners[2],
                        },
                        TrimCurve::Line {
                            start: corners[3],
                            end: corners[2],
                        },
                        TrimCurve::Line {
                            start: corners[0],
                            end: corners[3],
                        },
                    ],
                }],
            },
            &ValidationPolicy { max_gap: 1e-10 },
        )
        .unwrap()
    }
    #[test]
    fn unusable_subnormal_surface_budget_is_refused_before_sampling() {
        let body = spherical_rectangle();
        let policy = RealizationPolicy {
            tolerance: f64::from_bits(1),
            ..Default::default()
        };
        assert_eq!(
            body.realize(Placement3::IDENTITY, &policy).unwrap_err(),
            RealizationError::InvalidPolicy
        );
        assert_eq!(
            deviation(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], [0, 1, 2], [0.0; 3]),
            0.0
        );
    }
    #[test]
    fn spherical_trim_without_poles_uses_generic_chart_and_honest_error() {
        let body = spherical_rectangle();
        let mut last = f64::INFINITY;
        for tolerance in [0.1, 0.025] {
            let output = body
                .realize(
                    Placement3::IDENTITY,
                    &RealizationPolicy {
                        tolerance,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(output.report.within_tolerance());
            assert!(output.report.work.trim_refinement_batches > 0);
            assert!(output.report.work.trim_triangle_tests >= output.mesh_faces.len());
            assert!(output.report.work.trim_generated_vertices > 0);
            assert!(output.report.max_deviation < last);
            last = output.report.max_deviation;
            assert!(output.mesh.validate_deep().is_empty());
            assert_eq!(output.mesh.boundary_loops().unwrap().len(), 1);
            // Independent barycentric witnesses on every emitted triangle.
            for face in output.mesh.faces() {
                let p: Vec<_> = output
                    .mesh
                    .face_loop(face)
                    .map(|c| {
                        output
                            .mesh
                            .vertex_position(output.mesh.from_vertex(c).unwrap())
                            .unwrap()
                            .map(f64::from)
                    })
                    .collect();
                let center: [f64; 3] =
                    core::array::from_fn(|axis| p.iter().map(|p| p[axis] / 3.0).sum());
                assert!((exedra_math::norm(center) - 2.0).abs() <= output.report.max_deviation);
            }
        }
    }
}
