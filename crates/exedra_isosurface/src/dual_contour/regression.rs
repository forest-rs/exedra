// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Consumer-derived alignment regressions, using only public extraction APIs.

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
            cell_budget: None,
            vertex_merge_tolerance: 0.0,
            edge_search: EdgeSearchParams {
                bisection_steps: 14,
            },
            qef: QefParams::default(),
        },
    )
    .unwrap();
    assert_closed_sphere_topology(&result.mesh);
    assert!(result.stats.coincident_edge_collapses > 0);
    assert_eq!(result.stats.max_vertex_merge_displacement, 0.0);
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
                        cell_budget: None,
                        vertex_merge_tolerance: 0.000002,
                        edge_search: EdgeSearchParams {
                            bisection_steps: 14,
                        },
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
        cell_budget: None,
        vertex_merge_tolerance: 0.0,
        edge_search: EdgeSearchParams {
            bisection_steps: 14,
        },
        qef: QefParams::default(),
    }
}

#[test]
fn insufficient_join_tolerance_is_an_error() {
    assert!(matches!(
        dual_contour(&Junction { finish: false }, &extraction_params()),
        Err(DualContourError::Build(
            exedra_mesh::BuildError::DegenerateTriangle { .. }
        ))
    ));
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
            DualContourError::InvalidParameter(DualContourParameter::VertexMergeTolerance)
        );
    }
    for depth in [32, 255] {
        let params = DualContourParams {
            max_depth: depth,
            ..base
        };
        assert_eq!(
            dual_contour(&NoEvaluation, &params).unwrap_err(),
            DualContourError::InvalidParameter(DualContourParameter::MaxDepth)
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
        DualContourError::InvalidParameter(DualContourParameter::RootBounds)
    );
    let params = DualContourParams {
        root_bounds: Aabb::new([1_000_000.0; 3], [1_000_001.0; 3]).unwrap(),
        ..base
    };
    assert_eq!(
        dual_contour(&NoEvaluation, &params).unwrap_err(),
        DualContourError::InvalidParameter(DualContourParameter::MaxDepth)
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
        DualContourError::InvalidParameter(DualContourParameter::Qef)
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
