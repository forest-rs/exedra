// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use exedra_spatial::Aabb;

use super::{BoundedField, CapsuleField, RoundConeField, SmoothUnionN};
use crate::{DualContourParams, EdgeSearchParams, QefParams, ScalarField, dual_contour};

/// Deterministic `SplitMix64` stream for fixtures.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1_u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn point(&mut self, extent: f32) -> [f32; 3] {
        [
            self.range(-extent, extent),
            self.range(-extent, extent),
            self.range(-extent, extent),
        ]
    }
}

fn value<F: ScalarField + ?Sized>(field: &F, point: [f32; 3]) -> f32 {
    let mut out = [0.0];
    field.eval_points(&[point], &mut out);
    out[0]
}

fn gradient<F: ScalarField>(field: &F, point: [f32; 3]) -> [f32; 4] {
    let mut out = [[0.0; 4]];
    field.eval_gradients(&[point], &mut out);
    out[0]
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Distance to the union of spheres swept along the segment: exact outside a
/// round cone, since that union is the hull.
fn swept_outside(p: [f32; 3], a: [f32; 3], b: [f32; 3], ra: f32, rb: f32) -> f64 {
    let (p, a, b) = (p.map(f64::from), a.map(f64::from), b.map(f64::from));
    (0..=20_000)
        .map(|i| {
            let t = f64::from(i) / 20_000.0;
            dist(p, lerp3(a, b, t)) - (f64::from(ra) + (f64::from(rb) - f64::from(ra)) * t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Signed distance of a convex set from its support function,
/// `sup_n (n . p - h(n))`, sampled over Fibonacci directions: exact inside and
/// outside up to the direction spacing.
fn support_oracle(p: [f32; 3], a: [f32; 3], b: [f32; 3], ra: f32, rb: f32) -> f64 {
    let (p, a, b) = (p.map(f64::from), a.map(f64::from), b.map(f64::from));
    let count = 40_000;
    let golden = core::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..count)
        .map(|i| {
            let z = 1.0 - 2.0 * (f64::from(i) + 0.5) / f64::from(count);
            let r = (1.0 - z * z).sqrt();
            let phi = golden * f64::from(i);
            let n = [r * phi.cos(), r * phi.sin(), z];
            let dot = |v: [f64; 3]| n[0] * v[0] + n[1] * v[1] + n[2] * v[2];
            let support = (dot(a) + f64::from(ra)).max(dot(b) + f64::from(rb));
            dot(p) - support
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

#[test]
fn capsule_matches_sampled_segment_distance() {
    let mut rng = Stream(1);
    let capsule = CapsuleField {
        a: [-0.4, 0.1, 0.2],
        b: [0.7, -0.3, 0.5],
        radius: 0.25,
    };
    for _ in 0..200 {
        let p = rng.point(1.5);
        let exact = swept_outside(p, capsule.a, capsule.b, capsule.radius, capsule.radius);
        let got = f64::from(value(&capsule, p));
        // Swept spheres are exact for capsules inside and out.
        assert!((got - exact).abs() < 1.0e-4, "{p:?}: {got} vs {exact}");
    }
}

#[test]
fn round_cone_matches_support_and_swept_oracles() {
    let mut rng = Stream(2);
    let cones = [
        RoundConeField {
            a: [0.0, 0.0, -0.5],
            b: [0.1, 0.2, 0.6],
            radius_a: 0.4,
            radius_b: 0.15,
        },
        RoundConeField {
            a: [0.3, -0.2, 0.0],
            b: [-0.5, 0.4, 0.1],
            radius_a: 0.05,
            radius_b: 0.3,
        },
    ];
    for cone in cones {
        for _ in 0..150 {
            let p = rng.point(1.4);
            let got = f64::from(value(&cone, p));
            let support = support_oracle(p, cone.a, cone.b, cone.radius_a, cone.radius_b);
            assert!(
                (got - support).abs() < 5.0e-3 && got >= support - 1.0e-5,
                "{p:?}: {got} vs support {support}"
            );
            if got > 0.0 {
                let swept = swept_outside(p, cone.a, cone.b, cone.radius_a, cone.radius_b);
                assert!(
                    (got - swept).abs() < 1.0e-4,
                    "{p:?}: {got} vs swept {swept}"
                );
            }
        }
    }
}

#[test]
fn contained_and_degenerate_cones_are_spheres() {
    let contained = RoundConeField {
        a: [0.0; 3],
        b: [0.1, 0.0, 0.0],
        radius_a: 0.5,
        radius_b: 0.2,
    };
    assert_eq!(value(&contained, [2.0, 0.0, 0.0]), 1.5);
    assert_eq!(value(&contained, [0.0, -1.0, 0.0]), 0.5);
    let point = CapsuleField {
        a: [1.0; 3],
        b: [1.0; 3],
        radius: 0.5,
    };
    assert_eq!(value(&point, [1.0, 1.0, 3.0]), 1.5);
}

fn assert_gradients_match_differences<F: ScalarField>(field: &F, points: &[[f32; 3]]) {
    for p in points {
        let analytic = gradient(field, *p);
        assert_eq!(analytic[0], value(field, *p), "gradient row value");
        let h = 1.0e-3;
        for axis in 0..3 {
            let (mut plus, mut minus) = (*p, *p);
            plus[axis] += h;
            minus[axis] -= h;
            let numeric = (value(field, plus) - value(field, minus)) / (2.0 * h);
            assert!(
                (analytic[axis + 1] - numeric).abs() < 5.0e-3,
                "{p:?} axis {axis}: {} vs {numeric}",
                analytic[axis + 1]
            );
        }
    }
}

#[test]
fn gradients_match_finite_differences() {
    let capsule = CapsuleField {
        a: [0.0; 3],
        b: [0.0, 0.0, 1.0],
        radius: 0.2,
    };
    assert_gradients_match_differences(
        &capsule,
        &[[0.5, 0.1, 0.4], [0.1, 0.2, 1.3], [0.05, 0.0, 0.5]],
    );
    let cone = RoundConeField {
        a: [0.0; 3],
        b: [0.0, 0.0, 1.0],
        radius_a: 0.3,
        radius_b: 0.1,
    };
    // Near-a sphere, side, near-b sphere and inside.
    assert_gradients_match_differences(
        &cone,
        &[
            [0.2, 0.1, -0.4],
            [0.5, 0.2, 0.5],
            [0.1, 0.05, 1.3],
            [0.05, 0.02, 0.4],
        ],
    );
    let union = fork(false);
    assert_gradients_match_differences(
        &union,
        &[
            [0.12, 0.05, 0.02],
            [-0.1, 0.18, 0.1],
            [0.3, 0.3, 0.3],
            [0.0, 0.2, -0.3],
        ],
    );
}

fn fork(culling: bool) -> SmoothUnionN<RoundConeField> {
    let children = vec![
        RoundConeField {
            a: [0.0, 0.0, -0.8],
            b: [0.0, 0.0, 0.0],
            radius_a: 0.2,
            radius_b: 0.14,
        },
        RoundConeField {
            a: [0.0, 0.0, 0.0],
            b: [0.5, 0.1, 0.6],
            radius_a: 0.12,
            radius_b: 0.06,
        },
        RoundConeField {
            a: [0.0, 0.0, 0.0],
            b: [-0.4, -0.2, 0.7],
            radius_a: 0.11,
            radius_b: 0.05,
        },
    ];
    let union = SmoothUnionN::new(children, 0.06);
    if culling {
        union.with_distance_culling()
    } else {
        union
    }
}

/// A fork among many scattered twigs, most of them far from any one point.
fn crowd(culling: bool) -> SmoothUnionN<CapsuleField> {
    let mut rng = Stream(7);
    let children = (0..40)
        .map(|_| {
            let a = rng.point(3.0);
            let b = [0, 1, 2].map(|i| a[i] + rng.range(-0.5, 0.5));
            CapsuleField {
                a,
                b,
                radius: rng.range(0.02, 0.1),
            }
        })
        .collect();
    let union = SmoothUnionN::new(children, 0.05);
    if culling {
        union.with_distance_culling()
    } else {
        union
    }
}

#[test]
fn intervals_contain_sampled_values() {
    let mut rng = Stream(3);
    let cone = RoundConeField {
        a: [0.1, -0.2, 0.0],
        b: [-0.3, 0.4, 0.5],
        radius_a: 0.3,
        radius_b: 0.08,
    };
    let capsule = CapsuleField {
        a: [0.0; 3],
        b: [0.4, 0.4, 0.0],
        radius: 0.1,
    };
    let fields: [&dyn ScalarField; 6] = [
        &cone,
        &capsule,
        &fork(false),
        &fork(true),
        &crowd(false),
        &crowd(true),
    ];
    for field in fields {
        for _ in 0..300 {
            let center = rng.point(2.0);
            let half = [
                rng.range(0.001, 0.6),
                rng.range(0.001, 0.6),
                rng.range(0.001, 0.6),
            ];
            let bounds = Aabb {
                min: [0, 1, 2].map(|i| center[i] - half[i]),
                max: [0, 1, 2].map(|i| center[i] + half[i]),
            };
            let interval = field.eval_interval(&bounds).expect("finite interval");
            assert!(interval[0] <= interval[1]);
            for _ in 0..40 {
                let p =
                    [0, 1, 2].map(|i| bounds.min[i] + (bounds.max[i] - bounds.min[i]) * rng.unit());
                let v = value(field, p);
                assert!(
                    interval[0] <= v && v <= interval[1],
                    "{v} outside {interval:?} at {p:?}"
                );
            }
        }
    }
}

#[test]
fn culling_returns_the_same_bits() {
    let full = crowd(false);
    let culled = crowd(true);
    let mut rng = Stream(4);
    let points: Vec<[f32; 3]> = (0..500).map(|_| rng.point(3.2)).collect();
    let mut a = vec![0.0; points.len()];
    let mut b = vec![0.0; points.len()];
    full.eval_points(&points, &mut a);
    culled.eval_points(&points, &mut b);
    assert_eq!(a.map_bits(), b.map_bits());
    let mut ga = vec![[0.0; 4]; points.len()];
    let mut gb = vec![[0.0; 4]; points.len()];
    full.eval_gradients(&points, &mut ga);
    culled.eval_gradients(&points, &mut gb);
    for (x, y) in ga.iter().zip(&gb) {
        assert_eq!(x.map(f32::to_bits), y.map(f32::to_bits));
    }
    let stats = culled.stats();
    assert_eq!(stats.points, 1000);
    assert!(stats.child_culls > stats.child_evaluations, "{stats:?}");
    assert_eq!(full.stats().child_culls, 0);
    assert_eq!(full.stats().child_evaluations, 40 * 1000);
    culled.reset_stats();
    assert_eq!(culled.stats(), super::SmoothUnionStats::default());
}

trait MapBits {
    fn map_bits(&self) -> Vec<u32>;
}

impl MapBits for Vec<f32> {
    fn map_bits(&self) -> Vec<u32> {
        self.iter().map(|v| v.to_bits()).collect()
    }
}

#[test]
fn blend_is_symmetric_and_bounded() {
    // Two equal children lower the value by 2k (1 - 1/sqrt 2).
    let sphere = |x: f32| CapsuleField {
        a: [x, 0.0, 0.0],
        b: [x, 0.0, 0.0],
        radius: 0.5,
    };
    let pair = SmoothUnionN::new(vec![sphere(-1.0), sphere(1.0)], 0.2);
    let v = f64::from(value(&pair, [0.0, 3.0, 0.0]));
    let single = f64::from(value(&sphere(1.0), [0.0, 3.0, 0.0]));
    let drop = 2.0 * 0.2 * (1.0 - core::f64::consts::FRAC_1_SQRT_2);
    assert!(
        (v - (single - drop)).abs() < 1.0e-6,
        "{v} vs {}",
        single - drop
    );

    // At a three-way tie every child weighs a third, whichever is first.
    let tripod = |order: [usize; 3]| {
        let legs = [
            CapsuleField {
                a: [0.0; 3],
                b: [1.0, 0.0, 0.0],
                radius: 0.1,
            },
            CapsuleField {
                a: [0.0; 3],
                b: [-0.5, 0.866_025_4, 0.0],
                radius: 0.1,
            },
            CapsuleField {
                a: [0.0; 3],
                b: [-0.5, -0.866_025_4, 0.0],
                radius: 0.1,
            },
        ];
        SmoothUnionN::new(order.map(|i| legs[i]).to_vec(), 0.3)
    };
    let p = [0.0, 0.0, 0.6];
    let first = gradient(&tripod([0, 1, 2]), p);
    let second = gradient(&tripod([2, 0, 1]), p);
    for axis in 0..4 {
        assert!((first[axis] - second[axis]).abs() < 1.0e-6);
    }
    assert!((first[3] - 1.0).abs() < 1.0e-5, "{first:?}");

    // Children farther than 2k above the minimum do not contribute.
    let far = SmoothUnionN::new(vec![sphere(0.0), sphere(10.0)], 0.2);
    assert_eq!(
        value(&far, [0.0, 1.0, 0.0]),
        value(&sphere(0.0), [0.0, 1.0, 0.0])
    );
}

#[test]
fn empty_and_hard_unions_are_explicit() {
    let empty: SmoothUnionN<CapsuleField> = SmoothUnionN::new(Vec::new(), 0.1);
    assert_eq!(value(&empty, [0.0; 3]), f32::INFINITY);
    let bounds = Aabb {
        min: [0.0; 3],
        max: [1.0; 3],
    };
    assert_eq!(empty.eval_interval(&bounds), Some([f32::INFINITY; 2]));
    let hard = SmoothUnionN::new(
        vec![
            CapsuleField {
                a: [0.0; 3],
                b: [0.0; 3],
                radius: 1.0,
            },
            CapsuleField {
                a: [1.0, 0.0, 0.0],
                b: [1.0, 0.0, 0.0],
                radius: 1.0,
            },
        ],
        0.0,
    );
    assert_eq!(
        value(&hard, [0.5, 2.0, 0.0]),
        value(&hard.children()[0], [0.5, 2.0, 0.0])
    );
}

#[test]
fn distance_bounds_hold() {
    let mut rng = Stream(5);
    let cone = RoundConeField {
        a: [0.2, 0.0, 0.1],
        b: [-0.6, 0.5, 0.4],
        radius_a: 0.25,
        radius_b: 0.1,
    };
    let bound = cone.distance_bound();
    for _ in 0..500 {
        let p = rng.point(2.0);
        let gap = [0, 1, 2].map(|i| {
            (bound.core.min[i] - p[i])
                .max(p[i] - bound.core.max[i])
                .max(0.0)
        });
        let lower = (gap[0] * gap[0] + gap[1] * gap[1] + gap[2] * gap[2]).sqrt() - bound.radius;
        assert!(value(&cone, p) >= lower - 1.0e-6);
    }
}

#[test]
fn a_three_branch_fork_extracts_a_closed_mesh() {
    for culling in [false, true] {
        let field = fork(culling);
        let params = DualContourParams {
            root_bounds: Aabb::new([-0.7, -0.5, -1.1], [0.8, 0.5, 0.9]).expect("ordered"),
            max_depth: 6,
            cell_budget: None,
            limits: crate::ExtractionLimits::default(),
            witness_limit: 16,
            vertex_merge_tolerance: 0.0,
            edge_search: EdgeSearchParams::default(),
            qef: QefParams::default(),
        };
        let result = dual_contour(&field, &params).expect("fork extraction");
        assert!(result.stats.faces > 500, "{:?}", result.stats);
        assert!(result.mesh.validate_deep().is_empty());
        assert!(
            result
                .mesh
                .boundary_loops()
                .expect("boundary loops")
                .is_empty(),
            "closed fork"
        );
    }
}

fn sphere_at(center: [f32; 3]) -> CapsuleField {
    CapsuleField {
        a: center,
        b: center,
        radius: 0.5,
    }
}

#[test]
fn gradient_is_continuous_across_a_tie_with_a_third_child_in_the_band() {
    // Two spheres tie on the x = 0 plane while a third is within the band:
    // the closed form this replaced had a crease there.
    let legs = [
        sphere_at([-1.0, 0.0, 0.0]),
        sphere_at([1.0, 0.0, 0.0]),
        sphere_at([0.0, 2.0, 0.0]),
    ];
    let union = |order: [usize; 3]| SmoothUnionN::new(order.map(|i| legs[i]).to_vec(), 1.0);
    let at = |field: &SmoothUnionN<CapsuleField>, x: f32| f64::from(value(field, [x, 0.0, 0.0]));
    let field = union([0, 1, 2]);
    let h = 1.0e-3_f32;
    let left = (at(&field, 0.0) - at(&field, -h)) / f64::from(h);
    let right = (at(&field, h) - at(&field, 0.0)) / f64::from(h);
    assert!(
        (left - right).abs() < 1.0e-3,
        "one-sided slopes {left} and {right}"
    );
    let g = gradient(&field, [0.0; 3]);
    assert!(g[1].abs() < 1.0e-6, "{g:?}");

    // Every child order gives the same bits for the value and gradient.
    let reference = gradient(&field, [0.0; 3]).map(f32::to_bits);
    for order in [[0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        let other = gradient(&union(order), [0.0; 3]).map(f32::to_bits);
        assert_eq!(other, reference, "order {order:?}");
    }
    for point in [[0.3, 0.2, -0.1], [-0.7, 0.9, 0.4]] {
        let reference = gradient(&field, point).map(f32::to_bits);
        for order in [[2, 1, 0], [1, 2, 0]] {
            assert_eq!(gradient(&union(order), point).map(f32::to_bits), reference);
        }
    }
}

#[test]
fn a_single_child_in_the_band_is_returned_unchanged() {
    let child = sphere_at([0.2, -0.1, 0.3]);
    let far = sphere_at([50.0, 0.0, 0.0]);
    for union in [
        SmoothUnionN::new(vec![child], 0.4),
        SmoothUnionN::new(vec![far, child], 0.4),
    ] {
        for point in [[0.0; 3], [1.0, 2.0, -0.5], [0.2, -0.1, 0.9]] {
            assert_eq!(
                gradient(&union, point).map(f32::to_bits),
                gradient(&child, point).map(f32::to_bits)
            );
        }
    }
}

#[test]
fn blend_stays_within_its_stated_bounds() {
    let mut stream = Stream(0x5eed_0b1e);
    for _ in 0..200 {
        let n = 1 + (stream.next() % 6) as usize;
        let k = stream.range(0.05, 1.0);
        let children: Vec<_> = (0..n).map(|_| sphere_at(stream.point(1.0))).collect();
        let union = SmoothUnionN::new(children.clone(), k);
        let p = stream.point(1.5);
        let values: Vec<f64> = children.iter().map(|c| f64::from(value(c, p))).collect();
        let m = values.iter().copied().fold(f64::INFINITY, f64::min);
        let in_band = values
            .iter()
            .filter(|v| **v - m < 2.0 * f64::from(k))
            .count() as f64;
        let floor = m - 2.0 * f64::from(k) * (1.0 - 1.0 / in_band.sqrt());
        let v = f64::from(value(&union, p));
        assert!(
            v <= m + 1.0e-6 && v >= floor - 1.0e-6,
            "{v} not in [{floor}, {m}]"
        );
    }
}

/// A field with one value and gradient everywhere.
#[derive(Copy, Clone, Debug)]
struct Constant {
    value: f32,
    gradient: [f32; 3],
}

impl ScalarField for Constant {
    fn eval_interval(&self, _bounds: &Aabb) -> Option<[f32; 2]> {
        Some([self.value; 2])
    }

    fn eval_points(&self, _points: &[[f32; 3]], out: &mut [f32]) {
        out.fill(self.value);
    }

    fn eval_gradients(&self, _points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        let [x, y, z] = self.gradient;
        out.fill([self.value, x, y, z]);
    }
}

#[test]
fn infinite_child_bounds_give_valid_intervals() {
    let bounds = Aabb {
        min: [0.0; 3],
        max: [1.0; 3],
    };
    let nested = SmoothUnionN::new(
        vec![SmoothUnionN::<CapsuleField>::new(Vec::new(), 0.1)],
        0.1,
    );
    assert_eq!(nested.eval_interval(&bounds), Some([f32::INFINITY; 2]));
    let far = SmoothUnionN::new(
        vec![Constant {
            value: f32::INFINITY,
            gradient: [0.0; 3],
        }],
        0.1,
    );
    assert_eq!(far.eval_interval(&bounds), Some([f32::INFINITY; 2]));
}

#[test]
fn hard_minimum_zero_sign_does_not_depend_on_child_order() {
    let positive = Constant {
        value: 0.0,
        gradient: [1.0, 0.0, 0.0],
    };
    let negative = Constant {
        value: -0.0,
        gradient: [0.0, 1.0, 0.0],
    };
    let a = SmoothUnionN::new(vec![positive, negative], 0.0);
    let b = SmoothUnionN::new(vec![negative, positive], 0.0);
    let point = [0.25, 0.5, 0.75];
    assert_eq!(value(&a, point).to_bits(), value(&b, point).to_bits());
    assert_eq!(value(&a, point).to_bits(), (-0.0_f32).to_bits());
    assert_eq!(
        gradient(&a, point).map(f32::to_bits),
        gradient(&b, point).map(f32::to_bits)
    );
}
