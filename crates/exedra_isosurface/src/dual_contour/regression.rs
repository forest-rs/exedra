// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Consumer-derived alignment regressions, using only public extraction APIs.

use crate::ExtractionLimits;
use crate::analytic::{SphereField, TaggedField};
use crate::{
    Aabb, DualContourError, DualContourParameter, DualContourParams, EdgeSearchParams, QefParams,
    ScalarField, dual_contour, dual_contour_with_regions,
};
use exedra_math::{cross, dot, norm, sub};
use exedra_mesh::Mesh;
use hashbrown::HashMap;

#[test]
fn small_aligned_sphere_remains_closed() {
    let result = dual_contour(
        &SphereField {
            center: [0.0; 3],
            radius: 0.02,
        },
        &DualContourParams {
            root_bounds: Aabb::new([-0.032; 3], [0.032; 3]).unwrap(),
            max_depth: 5,
            thin_feature_depth: 0,
            cell_budget: None,
            limits: ExtractionLimits::default(),
            witness_limit: 16,
            vertex_merge_tolerance: 0.0,
            edge_search: EdgeSearchParams { precision_bits: 14 },
            qef: QefParams::default(),
        },
    )
    .unwrap();
    assert_closed_sphere_topology(&result.mesh);
    // Vertices keep off cell faces, so cells meeting at the sphere's
    // grid-aligned extremes no longer produce coincident vertices to join.
    assert_eq!(result.stats.coincident_edge_collapses, 0);
    assert_eq!(result.work.vertex_joins, 0);
}

struct Junction {
    finish: bool,
}

fn box_distance(point: [f32; 3], center: [f32; 3], half: [f32; 3]) -> f32 {
    let q = core::array::from_fn(|i| (point[i] - center[i]).abs() - half[i]);
    norm(q.map(|v| v.max(0.0))) + q[0].max(q[1]).max(q[2]).min(0.0)
}

fn weight(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Junction {
    fn value(&self, point: [f32; 3]) -> f32 {
        let [a, b] = [[-0.145, -0.035, 0.215], [0.145, -0.035, 0.215]]
            .map(|center| norm(sub(point, center)) - 0.190);
        let k = if self.finish {
            0.024
                * weight(1.0 - point[0].abs() / 0.055)
                * weight(1.0 - (point[2] - 0.180).abs() / 0.065)
        } else {
            0.0
        };
        let cavity = if k > 1e-7 {
            let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
            b * (1.0 - h) + a * h - k * h * (1.0 - h)
        } else {
            a.min(b)
        };
        box_distance(point, [0.0, 0.120, 0.230], [0.350, 0.120, 0.230])
            .max(-cavity)
            .max(-box_distance(
                point,
                [0.0, 0.0, 0.430],
                [0.044, 0.014, 0.016],
            ))
    }
}

impl ScalarField for Junction {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let radius = norm(sub(bounds.max, bounds.min)) * 0.5;
        // CSG distances are 1-Lipschitz; the spatial blend mask adds less than
        // 0.303 to that bound. The consumer deliberately uses a looser bound.
        let bound = radius * if self.finish { 2.0 } else { 1.0 } + 2e-6;
        let value = self.value(bounds.center());
        Some([value - bound, value + bound])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        for (&point, value) in points.iter().zip(out) {
            *value = self.value(point);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        for (&point, value) in points.iter().zip(out) {
            let gradient: [f32; 3] = core::array::from_fn(|i| {
                let mut a = point;
                let mut b = point;
                a[i] += 0.00002;
                b[i] -= 0.00002;
                (self.value(a) - self.value(b)) / 0.00004
            });
            *value = [self.value(point), gradient[0], gradient[1], gradient[2]];
        }
    }
}

#[test]
fn junction_aligned_domain_remains_closed() {
    for depth in [5, 6, 7] {
        for finish in [false, true] {
            for offset in [false, true] {
                let result = dual_contour_with_regions(
                    &TaggedField {
                        field: Junction { finish },
                        provenance: 37_u32,
                    },
                    &DualContourParams {
                        root_bounds: if offset {
                            Aabb::new([-0.3791, -0.0493, -0.0297], [0.3809, 0.2907, 0.4903])
                                .unwrap()
                        } else {
                            Aabb::new([-0.38, -0.05, -0.03], [0.38, 0.29, 0.49]).unwrap()
                        },
                        max_depth: depth,
                        thin_feature_depth: 0,
                        cell_budget: None,
                        limits: ExtractionLimits::default(),
                        witness_limit: 16,
                        vertex_merge_tolerance: 0.000002,
                        edge_search: EdgeSearchParams { precision_bits: 14 },
                        qef: QefParams::default(),
                    },
                )
                .unwrap_or_else(|error| {
                    panic!("depth={depth} finish={finish} offset={offset}: {error:?}")
                });
                assert_closed_sphere_topology(&result.mesh);
                let regions = result
                    .mesh
                    .attrs()
                    .dense(exedra_mesh::attr::FACE_REGION)
                    .unwrap();
                assert!(
                    result
                        .mesh
                        .faces()
                        .all(|face| regions.get(face.as_id()) == Some(&37))
                );
                assert_junction_geometry(&result.mesh, &Junction { finish }, depth);
                assert!(result.stats.max_vertex_merge_displacement <= f64::from(0.000002_f32));
                #[cfg(feature = "std")]
                std::eprintln!(
                    "depth={depth} finish={finish} offset={offset}: {:?}",
                    result.stats
                );
            }
        }
    }
}

fn extraction_params() -> DualContourParams {
    DualContourParams {
        root_bounds: Aabb::new([-0.38, -0.05, -0.03], [0.38, 0.29, 0.49]).unwrap(),
        max_depth: 5,
        thin_feature_depth: 0,
        cell_budget: None,
        limits: ExtractionLimits::default(),
        witness_limit: 16,
        vertex_merge_tolerance: 0.0,
        edge_search: EdgeSearchParams { precision_bits: 14 },
        qef: QefParams::default(),
    }
}

#[test]
fn blended_junction_extracts_without_joining() {
    // With vertices on cell faces, this junction once produced coincident
    // vertices that no zero-tolerance join could resolve.
    let field = TaggedField {
        field: Junction { finish: false },
        provenance: 37,
    };
    let result = dual_contour_with_regions(&field, &extraction_params()).unwrap();
    assert_eq!(result.work.vertex_joins, 0);
    assert!(result.mesh.validate_deep().is_empty());
    assert!(result.mesh.faces().count() > 0);
}

#[test]
fn refused_face_preserves_transition_and_source_evidence() {
    use crate::{DualContourErrorKind, ExtractionResource, ExtractionStage, ExtractionWitness};
    let field = TaggedField {
        field: Junction { finish: false },
        provenance: 37,
    };
    let mut params = extraction_params();
    params.limits.faces = Some(0);
    let error = dual_contour_with_regions(&field, &params).unwrap_err();
    assert!(matches!(
        error.kind,
        DualContourErrorKind::LimitExceeded {
            resource: ExtractionResource::Faces,
            used: 0,
            limit: 0,
            ..
        }
    ));
    assert_eq!(error.context.stage, ExtractionStage::Emission);
    let Some(ExtractionWitness::Transition(witness)) = error.context.witness else {
        panic!("missing transition evidence")
    };
    assert_eq!(witness.sampled_source, Some(37));
    assert_ne!(witness.start, witness.end);
    assert!(witness.candidates.iter().all(Option::is_some));
    for candidate in witness.candidates.into_iter().flatten() {
        assert!(candidate.cell.bounds.contains(candidate.position));
    }
    assert!(error.context.work.point_evaluations > 0);
    assert!(error.context.stats.octree_cells > 0);
}

#[test]
fn invalid_policies_fail_before_field_evaluation() {
    struct NoEvaluation;
    impl ScalarField for NoEvaluation {
        fn eval_interval(&self, _: &Aabb) -> Option<[f32; 2]> {
            panic!("must not evaluate");
        }
        fn eval_points(&self, _: &[[f32; 3]], _: &mut [f32]) {
            panic!("must not evaluate");
        }
        fn eval_gradients(&self, _: &[[f32; 3]], _: &mut [[f32; 4]]) {
            panic!("must not evaluate");
        }
    }
    let base = extraction_params();
    for tolerance in [-1.0, f32::NAN, f32::INFINITY] {
        let params = DualContourParams {
            vertex_merge_tolerance: tolerance,
            ..base
        };
        assert_eq!(
            dual_contour(&NoEvaluation, &params).unwrap_err(),
            DualContourError::invalid(DualContourParameter::VertexMergeTolerance)
        );
    }
    for depth in [32, 255] {
        let params = DualContourParams {
            max_depth: depth,
            thin_feature_depth: 0,
            ..base
        };
        assert_eq!(
            dual_contour(&NoEvaluation, &params).unwrap_err(),
            DualContourError::invalid(DualContourParameter::MaxDepth)
        );
    }
    let params = DualContourParams {
        root_bounds: Aabb {
            min: [0.0; 3],
            max: [0.0; 3],
        },
        ..base
    };
    assert_eq!(
        dual_contour(&NoEvaluation, &params).unwrap_err(),
        DualContourError::invalid(DualContourParameter::RootBounds)
    );
    let params = DualContourParams {
        root_bounds: Aabb::new([1_000_000.0; 3], [1_000_001.0; 3]).unwrap(),
        ..base
    };
    assert_eq!(
        dual_contour(&NoEvaluation, &params).unwrap_err(),
        DualContourError::invalid(DualContourParameter::MaxDepth)
    );
    let params = DualContourParams {
        qef: QefParams {
            jacobi_sweeps: 0,
            ..base.qef
        },
        ..base
    };
    assert_eq!(
        dual_contour(&NoEvaluation, &params).unwrap_err(),
        DualContourError::invalid(DualContourParameter::Qef)
    );
}

fn assert_closed_sphere_topology(mesh: &Mesh) {
    assert!(mesh.validate_deep().is_empty());
    assert!(mesh.boundary_loops().unwrap().is_empty());
    let vertices = mesh.vertices().count();
    let edges = mesh.half_edges().count() / 2;
    let faces = mesh.faces().count();
    assert_eq!(vertices + faces, edges + 2);
    let mut degrees = HashMap::<_, usize>::new();
    for edge in mesh.half_edges() {
        *degrees.entry(mesh.from_vertex(edge).unwrap()).or_default() += 1;
    }
    for vertex in mesh.vertices() {
        let start = mesh.vertex_out(vertex).expect("no isolated vertex");
        let degree = degrees[&vertex];
        let mut edge = start;
        for step in 0..degree {
            edge = mesh.next(mesh.twin(edge).unwrap()).unwrap();
            assert_eq!(mesh.from_vertex(edge), Some(vertex));
            assert_eq!(
                edge == start,
                step + 1 == degree,
                "disconnected vertex link"
            );
        }
    }
}

fn assert_junction_geometry(mesh: &Mesh, field: &Junction, depth: u8) {
    let triangles = mesh
        .faces()
        .map(|face| {
            let corners = mesh.face_loop(face).collect::<alloc::vec::Vec<_>>();
            assert_eq!(corners.len(), 3);
            core::array::from_fn::<_, 3, _>(|i| {
                mesh.vertex_position(mesh.to_vertex(corners[i]).unwrap())
                    .unwrap()
                    .map(f64::from)
            })
        })
        .collect::<alloc::vec::Vec<_>>();
    let volume = triangles
        .iter()
        .map(|&[a, b, c]| dot(a, cross(b, c)) / 6.0)
        .sum::<f64>();
    assert!((0.04..0.07).contains(&volume), "volume={volume}");
    let hit = |x: f64, z: f64| {
        triangles
            .iter()
            .filter_map(|&[a, b, c]| {
                let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
                if det == 0.0 {
                    return None;
                }
                let u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
                let v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
                let w = 1.0 - u - v;
                (u >= -1e-8 && v >= -1e-8 && w >= -1e-8).then_some(u * a[1] + v * b[1] + w * c[1])
            })
            .min_by(f64::total_cmp)
            .expect("front ray must hit actual triangles")
    };
    let expected = |x, z| {
        let (mut lo, mut hi) = (0.0, 0.239);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if field.value([x, mid, z]) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        f64::from((lo + hi) * 0.5)
    };
    let mut worst = 0.0_f64;
    for x in 0..21_u8 {
        for z in 0..16_u8 {
            let x = -0.325 + f32::from(x) * 0.0325;
            let z = 0.019 + f32::from(z) * 0.028;
            worst = worst.max((hit(f64::from(x), f64::from(z)) - expected(x, z)).abs());
        }
    }
    // Fixture acceptance thresholds, not a general distance-error certificate.
    let maximum_error = match depth {
        5 => 0.015,
        6 => 0.005,
        _ => 0.003,
    };
    assert!(worst < maximum_error, "depth={depth} front error={worst}");
    let socket_error = (hit(0.0, f64::from(0.430_f32)) - expected(0.0, 0.430)).abs();
    assert!(
        socket_error < if depth == 5 { 0.015 } else { 0.001 },
        "socket error={socket_error}"
    );
    let cusp_error = (hit(0.0, f64::from(0.332_f32)) - expected(0.0, 0.332)).abs();
    assert!(cusp_error < maximum_error, "cusp error={cusp_error}");
}

/// A gyroid sheet of half-thickness `half` cut by a ball: where the sheet
/// meets the sphere at an acute angle, the solid is a wedge that tapers
/// below any cell.
fn gyroid_ball(half: f64) -> impl ScalarField {
    use crate::Metric;
    use crate::function::{Formula, FormulaField};
    use crate::number::Number;
    struct GyroidBall {
        half: f64,
    }
    impl Formula for GyroidBall {
        fn eval<N: Number>(&self, p: [N; 3]) -> N {
            let [x, y, z] = p.map(|v| v * 7.0);
            let sheet = (x.sin() * y.cos() + y.sin() * z.cos() + z.sin() * x.cos()) / 7.0;
            let radius = (p[0].square() + p[1].square() + p[2].square()).sqrt();
            (sheet.abs() - self.half).max(radius - 0.95)
        }
    }
    FormulaField::new(
        GyroidBall { half },
        // Each gradient component of the sheet is bounded by Cauchy-Schwarz,
        // as `cos(7y)^2 + sin(7z)^2` and its cyclic siblings, which sum to 3.
        Metric::Lipschitz(1.732_051),
        Aabb::new([-1.0; 3], [1.0; 3]),
    )
    .unwrap()
}

fn gyroid_params(max_depth: u8, thin_feature_depth: u8) -> DualContourParams {
    DualContourParams {
        root_bounds: Aabb::new([-1.0137, -0.9871, -1.0213], [1.0049, 1.0311, 0.9917]).unwrap(),
        max_depth,
        thin_feature_depth,
        edge_search: EdgeSearchParams::default(),
        ..extraction_params()
    }
}

#[test]
fn gyroid_sheet_cut_by_a_ball_stays_manifold() {
    // Where the sheet meets the ball, the solid is a thin wedge that two
    // neighboring cells each cross in two arcs of their shared face; their
    // one-vertex components doubled the dual edge between them.
    let field = gyroid_ball(0.15);
    for depth in [5, 6] {
        let result = dual_contour(&field, &gyroid_params(depth, 0)).unwrap();
        assert!(result.mesh.validate_deep().is_empty(), "depth {depth}");
    }
}

#[test]
fn thin_wedges_refine_below_max_depth_and_stay_manifold() {
    // A thinner sheet leaves cells whose corners all lie outside although
    // the wedge passes through them. Below `max_depth`, those cells' faces
    // are split by finer neighbors, and a doubly crossed face still gets a
    // vertex per arc.
    let field = gyroid_ball(0.05);
    let plain = dual_contour(&field, &gyroid_params(5, 0)).unwrap();
    assert_eq!(plain.report.thin_feature_cells, 0);
    for thin in [1, 2] {
        let result = dual_contour(&field, &gyroid_params(5, thin)).unwrap();
        assert!(result.mesh.validate_deep().is_empty(), "thin {thin}");
        assert!(result.report.thin_feature_cells > 0, "thin {thin}");
        assert!(result.stats.faces > plain.stats.faces, "thin {thin}");
    }
}

#[test]
fn thin_feature_depth_must_fit_the_grid() {
    let params = DualContourParams {
        max_depth: 20,
        thin_feature_depth: 5,
        ..extraction_params()
    };
    assert_eq!(
        dual_contour(&gyroid_ball(0.05), &params).unwrap_err(),
        DualContourError::invalid(DualContourParameter::ThinFeatureDepth)
    );
}

#[test]
fn stray_samples_of_a_thin_wedge_are_dropped() {
    // A lone grid node inside a wedge tip closes a tiny surface of its own,
    // cut off from the sheet it samples.
    let result = dual_contour(&gyroid_ball(0.05), &gyroid_params(6, 0)).unwrap();
    assert!(result.report.dropped_islands > 0);
    assert!(result.mesh.validate_deep().is_empty());
}

#[test]
fn a_component_smaller_than_a_cell_is_kept() {
    use crate::Metric;
    use crate::function::FnField;
    use exedra_math::Real;
    // A ball much smaller than a cell around a grid node, far from a large
    // one: only that node is inside, and nothing else encloses it.
    let distance = |p: [f64; 3], c: [f64; 3], r: f64| {
        let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
        Real::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) - r
    };
    let field = FnField::new(
        move |p: [f64; 3]| distance(p, [-0.4; 3], 0.4).min(distance(p, [0.5; 3], 0.03)),
        Metric::Lipschitz(1.0),
        Aabb::new([-1.0; 3], [1.0; 3]),
    )
    .unwrap();
    let params = DualContourParams {
        root_bounds: Aabb::new([-1.0; 3], [1.0; 3]).unwrap(),
        max_depth: 5,
        thin_feature_depth: 0,
        ..extraction_params()
    };
    let result = dual_contour(&field, &params).unwrap();
    assert_eq!(result.report.dropped_islands, 0);
    let tiny = result
        .mesh
        .vertices()
        .filter(|&v| {
            let p = result.mesh.vertex_position(v).unwrap();
            p.iter().all(|&c| (c - 0.5).abs() < 0.07)
        })
        .count();
    assert_eq!(tiny, 8, "the lone node keeps its eight cell vertices");
}
