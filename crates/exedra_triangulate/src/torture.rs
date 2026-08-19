// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Property and torture tests: generated polygon corpora, degenerate
//! mutations, adversarial inputs, and exact-output goldens.
//!
//! Generators are seeded with a fixed deterministic PRNG so every run of the
//! suite exercises the identical corpus. Generated coordinates use only
//! arithmetic (no trig), so the corpus is bit-identical across platforms.

use alloc::vec::Vec;

use crate::{PolygonInput, TriParams, Triangulation, triangulate};

/// `SplitMix64`: tiny deterministic PRNG for corpus generation.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `[lo, hi)` with 20 bits of entropy.
    fn unit(&mut self, lo: f64, hi: f64) -> f64 {
        let t = (self.next() >> 44) as f64 / (1_u64 << 20) as f64;
        lo + t * (hi - lo)
    }

    fn range(&mut self, n: usize) -> usize {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "n is a small test-corpus bound; the modulo keeps the value below it"
        )]
        {
            (self.next() % n as u64) as usize
        }
    }
}

/// Generates an x-monotone simple polygon: a bottom chain below y = 0 and a
/// top chain above it, sharing their two end vertices. Always simple and
/// counter-clockwise by construction.
fn staircase_polygon(rng: &mut Rng, chain_len: usize) -> Vec<[f64; 2]> {
    let n = chain_len.max(2);
    let mut xs: Vec<f64> = (0..=n).map(|i| i as f64).collect();
    // Jitter interior xs while preserving strict monotonicity.
    for x in xs.iter_mut().skip(1).take(n - 1) {
        *x += rng.unit(-0.3, 0.3);
    }
    let mut points = Vec::with_capacity(2 * n);
    points.push([xs[0], 0.0]);
    for &x in &xs[1..n] {
        points.push([x, -rng.unit(0.5, 2.0)]);
    }
    points.push([xs[n], 0.0]);
    for &x in xs[1..n].iter().rev() {
        points.push([x, rng.unit(0.5, 2.0)]);
    }
    points
}

fn expected_area2(points: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        sum += (a[0] - b[0]) * (a[1] + b[1]);
    }
    sum
}

fn tri_area2(points: &[[f64; 2]], t: [u32; 3]) -> f64 {
    let a = points[t[0] as usize];
    let b = points[t[1] as usize];
    let c = points[t[2] as usize];
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Checks every invariant a successful triangulation must uphold.
fn check_invariants(outer: &[[f64; 2]], holes: &[&[[f64; 2]]], label: &str) -> Triangulation {
    let input = PolygonInput { outer, holes };
    let params = TriParams::default();
    let result = triangulate(&input, &params)
        .unwrap_or_else(|e| panic!("{label}: triangulation failed: {e}"));
    let again = triangulate(&input, &params).expect("second run");
    assert_eq!(result, again, "{label}: determinism violated");

    let mut points: Vec<[f64; 2]> = outer.to_vec();
    let mut expected = expected_area2(outer);
    for h in holes {
        points.extend_from_slice(h);
        expected += expected_area2(h);
    }

    let count = points.len();
    let mut area2 = 0.0;
    for &t in &result.triangles {
        for &i in &t {
            assert!((i as usize) < count, "{label}: index {i} out of bounds");
        }
        let a2 = tri_area2(&points, t);
        assert!(a2 > 0.0, "{label}: non-CCW triangle {t:?}");
        area2 += a2;
    }
    let scale = expected.abs().max(1.0);
    assert!(
        (area2 - expected).abs() <= scale * 1e-9,
        "{label}: area {area2} != expected {expected}"
    );
    result
}

#[test]
fn staircase_corpus_triangulates() {
    let mut rng = Rng(0xE0DA_0001);
    for case in 0..200 {
        let chain = 2 + rng.range(30);
        let polygon = staircase_polygon(&mut rng, chain);
        check_invariants(&polygon, &[], &alloc::format!("staircase {case}"));
    }
}

#[test]
fn staircase_with_hole_corpus_triangulates() {
    let mut rng = Rng(0xE0DA_0002);
    for case in 0..100 {
        let chain = 4 + rng.range(20);
        let polygon = staircase_polygon(&mut rng, chain);
        // A thin clockwise rectangle around y = 0 is strictly interior for
        // x within the second..second-to-last chain columns.
        let x0 = polygon[1][0] + 0.05;
        let x1 = polygon[chain.max(2) - 1][0] - 0.05;
        if x1 - x0 < 0.2 {
            continue;
        }
        let m = 0.2;
        let hole: [[f64; 2]; 4] = [[x0, -m], [x0, m], [x1, m], [x1, -m]];
        check_invariants(
            &polygon,
            &[&hole],
            &alloc::format!("holed staircase {case}"),
        );
    }
}

#[test]
fn duplicate_vertex_mutations_survive() {
    let mut rng = Rng(0xE0DA_0003);
    for case in 0..100 {
        let chain = 3 + rng.range(15);
        let mut polygon = staircase_polygon(&mut rng, chain);
        let at = rng.range(polygon.len());
        polygon.insert(at, polygon[at]);
        check_invariants(&polygon, &[], &alloc::format!("duplicated {case}"));
    }
}

#[test]
fn collinear_midpoint_mutations_survive() {
    let mut rng = Rng(0xE0DA_0004);
    for case in 0..100 {
        let chain = 3 + rng.range(15);
        let mut polygon = staircase_polygon(&mut rng, chain);
        let at = rng.range(polygon.len());
        let next = (at + 1) % polygon.len();
        let mid = [
            (polygon[at][0] + polygon[next][0]) / 2.0,
            (polygon[at][1] + polygon[next][1]) / 2.0,
        ];
        polygon.insert(at + 1, mid);
        check_invariants(&polygon, &[], &alloc::format!("collinear {case}"));
    }
}

#[test]
fn self_intersecting_inputs_fail_typed_not_panic() {
    let mut rng = Rng(0xE0DA_0005);
    let mut rejected = 0;
    for _ in 0..200 {
        // Random quadrilaterals: many are bowties. Whatever happens must be
        // deterministic and typed — never a panic.
        let quad: Vec<[f64; 2]> = (0..4)
            .map(|_| [rng.unit(-5.0, 5.0), rng.unit(-5.0, 5.0)])
            .collect();
        let input = PolygonInput {
            outer: &quad,
            holes: &[],
        };
        let params = TriParams::default();
        let first = triangulate(&input, &params);
        let second = triangulate(&input, &params);
        assert_eq!(first, second, "error paths must be deterministic too");
        if first.is_err() {
            rejected += 1;
        }
    }
    assert!(rejected > 0, "corpus must include rejected bowties");
}

#[test]
fn dense_arc_like_profile_with_hole() {
    // A 256-gon approximating a circle, with a clockwise 128-gon hole —
    // the shape profile flattening feeds in practice. Vertices are computed
    // by integer-seeded rational rotation (no trig): iterating the rotation
    // (x, y) -> (x - y*k, y + x*k) traces a near-circle deterministically.
    fn near_circle(n: usize, radius: f64, clockwise: bool) -> Vec<[f64; 2]> {
        let k = 4.0 / n as f64;
        let mut points = Vec::with_capacity(n);
        let (mut x, mut y) = (radius, 0.0_f64);
        for _ in 0..n {
            points.push([x, y]);
            let nx = x - y * k;
            let ny = y + x * k;
            x = nx;
            y = ny;
        }
        if clockwise {
            points.reverse();
            points.rotate_right(1);
        }
        points
    }

    let outer = near_circle(256, 10.0, false);
    let hole = near_circle(128, 4.0, true);
    let outer_area = expected_area2(&outer);
    let hole_area = expected_area2(&hole);
    assert!(outer_area > 0.0 && hole_area < 0.0, "fixture windings");

    let result = check_invariants(&outer, &[&hole], "dense arc profile");
    assert!(result.len() > 300, "dense profile produces a full cover");
}

#[test]
fn golden_square_with_hole_indices() {
    // Exact output pin: any change to tie-breaks, bridging order, or
    // predicates shows up here as a reviewable diff.
    let outer: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let hole: [[f64; 2]; 4] = [[0.4, 0.4], [0.4, 0.6], [0.6, 0.6], [0.6, 0.4]];
    let input = PolygonInput {
        outer: &outer,
        holes: &[&hole],
    };
    let result = triangulate(&input, &TriParams::default()).expect("golden fixture");
    assert_eq!(
        result.triangles,
        alloc::vec![
            [7, 0, 1],
            [7, 1, 2],
            [0, 7, 4],
            [3, 0, 4],
            [3, 4, 5],
            [2, 3, 5],
            [2, 5, 6],
            [2, 6, 7]
        ],
        "golden triangulation changed; review and re-bless deliberately"
    );
}

#[test]
fn golden_l_profile_indices() {
    let l: [[f64; 2]; 6] = [
        [0.0, 0.0],
        [1.0, 0.0],
        [1.0, 0.5],
        [0.5, 0.5],
        [0.5, 1.0],
        [0.0, 1.0],
    ];
    let input = PolygonInput {
        outer: &l,
        holes: &[],
    };
    let result = triangulate(&input, &TriParams::default()).expect("golden fixture");
    assert_eq!(
        result.triangles,
        // Vertex 0 is not the first ear: vertex 3 lies exactly on the
        // hypotenuse of its candidate triangle and the exact on-edge test
        // rejects it, so clipping starts at vertex 1.
        alloc::vec![[0, 1, 2], [0, 2, 3], [5, 0, 3], [5, 3, 4]],
        "golden triangulation changed; review and re-bless deliberately"
    );
}
