// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Property and torture tests: generated polygon corpora, degenerate
//! mutations, adversarial inputs, and exact-output goldens.
//!
//! Generators are seeded with a fixed deterministic PRNG so every run of the
//! suite exercises the identical corpus. Generated coordinates use only
//! arithmetic (no trig), so the corpus is bit-identical across platforms.

use alloc::vec::Vec;

use crate::delaunay::{illegal_edge_count, legalize_edges};
use crate::{
    PolygonInput, RefineParams, SteinerOrigin, TriParams, Triangulation, refine, triangulate,
    triangulate_with_stats,
};

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

fn cross(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// Independent closed-segment check used only to discard ambiguous samples.
fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    cross(a, b, p) == 0.0
        && p[0] >= a[0].min(b[0])
        && p[0] <= a[0].max(b[0])
        && p[1] >= a[1].min(b[1])
        && p[1] <= a[1].max(b[1])
}

/// Independent odd-even point-in-loop test. Samples on an edge are filtered
/// before this function is called, so the horizontal-ray crossing rule has no
/// boundary convention to choose.
fn inside_loop(point: [f64; 2], loop_points: &[[f64; 2]]) -> bool {
    let mut inside = false;
    for index in 0..loop_points.len() {
        let a = loop_points[index];
        let b = loop_points[(index + 1) % loop_points.len()];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = a[0] + (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]);
            if point[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

fn source_contains(point: [f64; 2], outer: &[[f64; 2]], holes: &[&[[f64; 2]]]) -> bool {
    inside_loop(point, outer) && holes.iter().all(|hole| !inside_loop(point, hole))
}

fn triangle_contains(point: [f64; 2], points: &[[f64; 2]], [a, b, c]: [u32; 3]) -> bool {
    let (a, b, c) = (points[a as usize], points[b as usize], points[c as usize]);
    let signs = [cross(a, b, point), cross(b, c, point), cross(c, a, point)];
    signs.iter().all(|&sign| sign > 0.0) || signs.iter().all(|&sign| sign < 0.0)
}

fn on_any_edge(
    point: [f64; 2],
    loops: &[&[[f64; 2]]],
    points: &[[f64; 2]],
    triangles: &[[u32; 3]],
) -> bool {
    loops.iter().any(|loop_points| {
        (0..loop_points.len()).any(|index| {
            on_segment(
                loop_points[index],
                loop_points[(index + 1) % loop_points.len()],
                point,
            )
        })
    }) || triangles.iter().any(|&[a, b, c]| {
        on_segment(points[a as usize], points[b as usize], point)
            || on_segment(points[b as usize], points[c as usize], point)
            || on_segment(points[c as usize], points[a as usize], point)
    })
}

/// Compares a triangulation's union with an independent source-domain oracle
/// at a fixed lattice of samples. Source and triangle edge samples are
/// ignored because both point-in-polygon and triangle coverage are boundary
/// convention questions; every other sample must have exactly one covering
/// triangle inside the outer loop and holes, and none outside.
fn assert_sampled_occupancy(
    outer: &[[f64; 2]],
    holes: &[&[[f64; 2]]],
    points: &[[f64; 2]],
    triangles: &[[u32; 3]],
    label: &str,
) {
    let mut min = outer[0];
    let mut max = outer[0];
    for &point in outer
        .iter()
        .chain(holes.iter().flat_map(|hole| hole.iter()))
    {
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    let span = [(max[0] - min[0]).max(1.0), (max[1] - min[1]).max(1.0)];
    let loops = core::iter::once(outer)
        .chain(holes.iter().copied())
        .collect::<Vec<_>>();
    const GRID: usize = 17;
    let extent_min = [min[0] - span[0] / 4.0, min[1] - span[1] / 4.0];
    let extent_span = [span[0] * 1.5, span[1] * 1.5];
    for row in 0..GRID {
        for column in 0..GRID {
            let sample = [
                extent_min[0] + extent_span[0] * (column as f64 + 0.5) / GRID as f64,
                extent_min[1] + extent_span[1] * (row as f64 + 0.5) / GRID as f64,
            ];
            if on_any_edge(sample, &loops, points, triangles) {
                continue;
            }
            let expected = usize::from(source_contains(sample, outer, holes));
            let covered = triangles
                .iter()
                .filter(|&&triangle| triangle_contains(sample, points, triangle))
                .count();
            assert_eq!(
                covered, expected,
                "{label}: sample {sample:?} expected {expected} covering triangle(s), got {covered}"
            );
        }
    }
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
    check_constrained_delaunay(&input, &points, &result, label);
    check_refinement(&input, &result, expected, label);
    result
}

/// Asserts the refinement invariants: determinism, CCW triangles, area
/// equal to the polygon within rounding of generated boundary points, every
/// boundary edge of the result lying on one input boundary edge, no
/// remaining illegal edge, and either a met bound or an honest stop reason.
fn check_refinement(
    input: &PolygonInput<'_>,
    reference: &Triangulation,
    expected_area2: f64,
    label: &str,
) {
    let params = RefineParams {
        max_steiner_points: 256,
        ..RefineParams::default()
    };
    let first = refine(input, &params).unwrap_or_else(|e| panic!("{label}: refine failed: {e}"));
    let second = refine(input, &params).expect("second run");
    assert_eq!(first, second, "{label}: refinement determinism violated");

    let count = first.input_vertex_count as usize;
    assert_eq!(first.points.len(), count + first.steiner.len());
    assert_eq!(first.stats.steiner_points as usize, first.steiner.len());
    assert!(first.triangles.len() >= reference.len());
    let mut area2 = 0.0;
    for &t in &first.triangles {
        for &i in &t {
            assert!((i as usize) < first.points.len(), "{label}: index {i}");
        }
        let a2 = tri_area2(&first.points, t);
        assert!(a2 > 0.0, "{label}: non-CCW refined triangle {t:?}");
        area2 += a2;
    }
    let scale = expected_area2.abs().max(1.0);
    assert!(
        (area2 - expected_area2).abs() <= scale * 1e-9,
        "{label}: refined area {area2} != expected {expected_area2}"
    );

    // Every refined boundary edge lies within rounding of one reference
    // boundary edge: both endpoints are input vertices of that edge or
    // generated points whose origin names it.
    let reference_boundary = boundary_edges(&reference.triangles);
    let origin_of = |index: u32| -> Option<[u32; 2]> {
        let steiner = (index as usize).checked_sub(count)?;
        match first.steiner[steiner] {
            SteinerOrigin::Boundary { edge } => Some(edge),
            SteinerOrigin::Interior => None,
        }
    };
    for (a, b) in boundary_edges(&first.triangles) {
        let owner = origin_of(a).or_else(|| origin_of(b));
        let (lo, hi) = match owner {
            Some(edge) => (edge[0], edge[1]),
            None => (a, b),
        };
        assert!(
            reference_boundary.contains(&(lo, hi)),
            "{label}: boundary edge ({a}, {b}) is not on a reference boundary edge"
        );
        for endpoint in [a, b] {
            if let Some(edge) = origin_of(endpoint) {
                assert_eq!(edge, [lo, hi], "{label}: mixed boundary origins");
            } else {
                assert!(endpoint == lo || endpoint == hi, "{label}: stray endpoint");
            }
        }
    }

    assert_eq!(
        illegal_edge_count(&first.points, &first.triangles),
        0,
        "{label}: refined cover is not Delaunay"
    );
    let stats = first.stats;
    assert!(
        stats.remaining_bad_triangles == stats.input_limited_triangles
            || stats.budget_exhausted
            || stats.declined_insertions > 0,
        "{label}: unexplained remaining violations {stats:?}"
    );
}

/// Sorted `(min, max)` edges that have exactly one incident triangle: the
/// simplified boundary the triangulation represents.
fn boundary_edges(triangles: &[[u32; 3]]) -> Vec<(u32, u32)> {
    let mut edges: Vec<(u32, u32)> = triangles
        .iter()
        .flat_map(|&[a, b, c]| {
            [
                (a.min(b), a.max(b)),
                (b.min(c), b.max(c)),
                (c.min(a), c.max(a)),
            ]
        })
        .collect();
    edges.sort_unstable();
    let mut boundary = Vec::new();
    let mut index = 0;
    while index < edges.len() {
        let run = edges[index..]
            .iter()
            .take_while(|&&e| e == edges[index])
            .count();
        if run == 1 {
            boundary.push(edges[index]);
        }
        index += run;
    }
    boundary
}

/// Asserts the constrained-Delaunay invariants against the ear-clipped
/// `reference` for the same input: identical triangle count, CCW triangles,
/// the same area and boundary edge set, no remaining illegal edge,
/// idempotence, and double-run determinism including the flip count.
fn check_constrained_delaunay(
    input: &PolygonInput<'_>,
    points: &[[f64; 2]],
    reference: &Triangulation,
    label: &str,
) {
    let params = TriParams::constrained_delaunay();
    let first = triangulate_with_stats(input, &params)
        .unwrap_or_else(|e| panic!("{label}: constrained Delaunay failed: {e}"));
    let second = triangulate_with_stats(input, &params).expect("second run");
    assert_eq!(
        first, second,
        "{label}: constrained Delaunay determinism violated"
    );

    let triangles = &first.triangulation.triangles;
    assert_eq!(
        triangles.len(),
        reference.len(),
        "{label}: flips changed the count"
    );
    let mut area2 = 0.0;
    let mut reference_area2 = 0.0;
    for (&t, &r) in triangles.iter().zip(&reference.triangles) {
        let a2 = tri_area2(points, t);
        assert!(a2 > 0.0, "{label}: non-CCW legalized triangle {t:?}");
        area2 += a2;
        reference_area2 += tri_area2(points, r);
    }
    let scale = reference_area2.abs().max(1.0);
    assert!(
        (area2 - reference_area2).abs() <= scale * 1e-9,
        "{label}: legalized area {area2} != ear-clipped {reference_area2}"
    );
    assert_eq!(
        boundary_edges(triangles),
        boundary_edges(&reference.triangles),
        "{label}: legalization changed the boundary"
    );
    assert_eq!(
        illegal_edge_count(points, triangles),
        0,
        "{label}: illegal edge remains"
    );
    let mut again = triangles.clone();
    assert_eq!(
        legalize_edges(points, &mut again),
        0,
        "{label}: not idempotent"
    );
    assert_eq!(&again, triangles, "{label}: canonical order unstable");
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
fn sampled_occupancy_matches_independent_polygon_oracle() {
    // Exercise both a hole and a concave source, checking the ordinary cover
    // and the generated-vertex refinement without making the corpus tests
    // multiply their existing sample cost.
    let outer: [[f64; 2]; 4] = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
    let hole: [[f64; 2]; 4] = [[1.25, 1.25], [1.25, 2.75], [2.75, 2.75], [2.75, 1.25]];
    let holes: [&[[f64; 2]]; 1] = [&hole];
    let input = PolygonInput {
        outer: &outer,
        holes: &holes,
    };
    let mut source_points = outer.to_vec();
    source_points.extend_from_slice(&hole);

    let triangulated = triangulate(&input, &TriParams::constrained_delaunay())
        .expect("sampled source triangulates");
    assert_sampled_occupancy(
        &outer,
        &holes,
        &source_points,
        &triangulated.triangles,
        "triangulated square with hole",
    );

    let refined = refine(&input, &RefineParams::default()).expect("sampled source refines");
    assert_sampled_occupancy(
        &outer,
        &holes,
        &refined.points,
        &refined.triangles,
        "refined square with hole",
    );

    let concave: [[f64; 2]; 6] = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.5],
        [1.5, 1.5],
        [1.5, 4.0],
        [0.0, 4.0],
    ];
    let concave_input = PolygonInput {
        outer: &concave,
        holes: &[],
    };
    let concave_triangles = triangulate(&concave_input, &TriParams::constrained_delaunay())
        .expect("sampled concave source triangulates");
    assert_sampled_occupancy(
        &concave,
        &[],
        &concave,
        &concave_triangles.triangles,
        "triangulated concave source",
    );
    let concave_refined =
        refine(&concave_input, &RefineParams::default()).expect("sampled concave source refines");
    assert_sampled_occupancy(
        &concave,
        &[],
        &concave_refined.points,
        &concave_refined.triangles,
        "refined concave source",
    );
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
fn golden_square_with_hole_constrained_delaunay_indices() {
    // The legalized cover is canonical (each triangle rotated to its lowest
    // index, then sorted), so this pins the unique perturbed constrained
    // Delaunay triangulation and the exact flip count.
    let outer: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let hole: [[f64; 2]; 4] = [[0.4, 0.4], [0.4, 0.6], [0.6, 0.6], [0.6, 0.4]];
    let input = PolygonInput {
        outer: &outer,
        holes: &[&hole],
    };
    let params = TriParams::constrained_delaunay();
    let result = triangulate_with_stats(&input, &params).expect("golden fixture");
    assert_eq!(result.stats.edge_flips, 2, "flip count changed");
    // The quad 0-4-5-3 is an isosceles trapezoid, hence exactly cocircular;
    // the lowest-index rule keeps vertex 0 on its diagonal.
    assert_eq!(
        result.triangulation.triangles,
        alloc::vec![
            [0, 1, 7],
            [0, 4, 5],
            [0, 5, 3],
            [0, 7, 4],
            [1, 2, 6],
            [1, 6, 7],
            [2, 3, 5],
            [2, 5, 6]
        ],
        "golden legalized triangulation changed; review and re-bless deliberately"
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
