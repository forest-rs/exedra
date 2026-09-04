// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Quality metrics, validation, and deterministic signatures.

use crate::fixtures::{Fixture, FixtureRole};
use crate::{FNV_OFFSET, FNV_PRIME};
use crate::{strategy_label, strategy_params};
use exedra_triangulate::{
    RefineParams, RefineStats, RefinedTriangulation, TriStrategy, Triangulation, refine,
    triangulate_with_stats,
};

pub(crate) struct QualityReport {
    pub(crate) name: &'static str,
    pub(crate) strategy: &'static str,
    pub(crate) role: FixtureRole,
    pub(crate) vertices: usize,
    pub(crate) holes: usize,
    pub(crate) triangles: usize,
    pub(crate) signature: u64,
    pub(crate) min_angle_deg: f64,
    pub(crate) p01_angle_deg: f64,
    pub(crate) worst_quality: f64,
    pub(crate) below_1deg: usize,
    pub(crate) below_5deg: usize,
    pub(crate) below_10deg: usize,
    pub(crate) edge_flips: usize,
}

impl QualityReport {
    pub(crate) fn print(&self) {
        println!(
            "scenario={} strategy={} role={} vertices={} holes={} triangles={} signature={:016x} min_angle_deg={:.9} p01_angle_deg={:.9} worst_quality={:.9e} below_1deg={} below_5deg={} below_10deg={} edge_flips={}",
            self.name,
            self.strategy,
            self.role.label(),
            self.vertices,
            self.holes,
            self.triangles,
            self.signature,
            self.min_angle_deg,
            self.p01_angle_deg,
            self.worst_quality,
            self.below_1deg,
            self.below_5deg,
            self.below_10deg,
            self.edge_flips,
        );
    }
}

pub(crate) fn analyze(fixture: &Fixture, strategy: TriStrategy) -> QualityReport {
    let params = strategy_params(strategy);
    let prepared = fixture.prepare();
    let input = prepared.input();
    let first = triangulate_with_stats(&input, &params).expect("fixed fixture must triangulate");
    let second =
        triangulate_with_stats(&input, &params).expect("fixed fixture repeat must triangulate");
    assert_eq!(
        first, second,
        "{} must produce byte-identical triangles",
        fixture.name
    );
    let result = &first.triangulation;

    let points = fixture.points();
    validate_cover(fixture, &points, result);
    let mut minimum_angles = Vec::with_capacity(result.triangles.len());
    let mut worst_quality = f64::INFINITY;
    for &triangle in &result.triangles {
        let [a, b, c] = triangle_points(&points, triangle);
        minimum_angles.push(triangle_min_angle(a, b, c));
        worst_quality = worst_quality.min(normalized_quality(a, b, c));
    }
    minimum_angles.sort_by(f64::total_cmp);
    let min_angle = minimum_angles.first().copied().unwrap_or(0.0);
    let p01_angle = first_percentile(&minimum_angles);
    let degrees = 180.0 / std::f64::consts::PI;

    QualityReport {
        name: fixture.name,
        strategy: strategy_label(strategy),
        role: fixture.role,
        vertices: fixture.vertex_count(),
        holes: fixture.holes.len(),
        triangles: result.triangles.len(),
        signature: signature(fixture, result),
        min_angle_deg: min_angle * degrees,
        p01_angle_deg: p01_angle * degrees,
        worst_quality,
        below_1deg: count_below(&minimum_angles, 1.0 / degrees),
        below_5deg: count_below(&minimum_angles, 5.0 / degrees),
        below_10deg: count_below(&minimum_angles, 10.0 / degrees),
        edge_flips: first.stats.edge_flips,
    }
}

/// Quality and work of one budgeted refinement run under the default bound.
pub(crate) struct RefineReport {
    pub(crate) name: &'static str,
    pub(crate) role: FixtureRole,
    pub(crate) vertices: usize,
    pub(crate) triangles: usize,
    pub(crate) signature: u64,
    pub(crate) min_angle_deg: f64,
    pub(crate) p01_angle_deg: f64,
    pub(crate) worst_quality: f64,
    pub(crate) below_10deg: usize,
    pub(crate) below_20deg: usize,
    pub(crate) stats: RefineStats,
}

impl RefineReport {
    pub(crate) fn print(&self) {
        let stats = self.stats;
        println!(
            "scenario={} strategy=Refined role={} vertices={} triangles={} signature={:016x} min_angle_deg={:.9} p01_angle_deg={:.9} worst_quality={:.9e} below_10deg={} below_20deg={} steiner_points={} boundary_splits={} interior_insertions={} declined_insertions={} remaining_bad={} input_limited={} budget_exhausted={} edge_flips={}",
            self.name,
            self.role.label(),
            self.vertices,
            self.triangles,
            self.signature,
            self.min_angle_deg,
            self.p01_angle_deg,
            self.worst_quality,
            self.below_10deg,
            self.below_20deg,
            stats.steiner_points,
            stats.boundary_splits,
            stats.interior_insertions,
            stats.declined_insertions,
            stats.remaining_bad_triangles,
            stats.input_limited_triangles,
            stats.budget_exhausted,
            stats.edge_flips,
        );
    }
}

pub(crate) fn refine_params() -> RefineParams {
    RefineParams::default()
}

pub(crate) fn analyze_refined(fixture: &Fixture) -> RefineReport {
    let params = refine_params();
    let prepared = fixture.prepare();
    let input = prepared.input();
    let first = refine(&input, &params).expect("fixed fixture must refine");
    let second = refine(&input, &params).expect("fixed fixture repeat must refine");
    assert_eq!(
        first, second,
        "{} must produce byte-identical refinement",
        fixture.name
    );
    validate_refined_cover(fixture, &first);

    let mut minimum_angles = Vec::with_capacity(first.triangles.len());
    let mut worst_quality = f64::INFINITY;
    for &triangle in &first.triangles {
        let [a, b, c] = triangle_points(&first.points, triangle);
        minimum_angles.push(triangle_min_angle(a, b, c));
        worst_quality = worst_quality.min(normalized_quality(a, b, c));
    }
    minimum_angles.sort_by(f64::total_cmp);
    let degrees = 180.0 / std::f64::consts::PI;
    RefineReport {
        name: fixture.name,
        role: fixture.role,
        vertices: first.points.len(),
        triangles: first.triangles.len(),
        signature: refined_signature(fixture, &first),
        min_angle_deg: minimum_angles.first().copied().unwrap_or(0.0) * degrees,
        p01_angle_deg: first_percentile(&minimum_angles) * degrees,
        worst_quality,
        below_10deg: count_below(&minimum_angles, 10.0 / degrees),
        below_20deg: count_below(&minimum_angles, 20.0 / degrees),
        stats: first.stats,
    }
}

/// Refined covers add rounded boundary midpoints, so the area check allows
/// rounding-scale slivers; indices, orientation, and provenance are exact.
pub(crate) fn validate_refined_cover(fixture: &Fixture, result: &RefinedTriangulation) {
    let input_count = result.input_vertex_count as usize;
    assert_eq!(input_count, fixture.vertex_count(), "{}", fixture.name);
    assert_eq!(
        result.points.len(),
        input_count + result.steiner.len(),
        "{}",
        fixture.name
    );
    assert_eq!(
        &result.points[..input_count],
        fixture.points(),
        "{}",
        fixture.name
    );
    let mut triangle_area2 = 0.0;
    for &triangle in &result.triangles {
        for index in triangle {
            assert!(
                (index as usize) < result.points.len(),
                "{} emitted out-of-range index {index}",
                fixture.name
            );
        }
        let [a, b, c] = triangle_points(&result.points, triangle);
        let area2 = cross(a, b, c);
        assert!(
            area2 > 0.0,
            "{} emitted non-CCW refined triangle {triangle:?}",
            fixture.name
        );
        triangle_area2 += area2;
    }
    let expected_area2 = signed_area2(&fixture.outer)
        + fixture
            .holes
            .iter()
            .map(|hole| signed_area2(hole))
            .sum::<f64>();
    let scale = expected_area2.abs().max(f64::MIN_POSITIVE);
    assert!(
        (triangle_area2 - expected_area2).abs() <= scale * 1e-10,
        "{} refined area mismatch: triangles={triangle_area2:e} polygon={expected_area2:e}",
        fixture.name
    );
}

/// FNV-1a over the fixture, every generated coordinate in insertion order,
/// and every emitted triangle index.
pub(crate) fn refined_signature(fixture: &Fixture, result: &RefinedTriangulation) -> u64 {
    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, fixture.name.as_bytes());
    hash_u64(&mut hash, fixture.outer.len() as u64);
    for point in &fixture.outer {
        hash_point(&mut hash, *point);
    }
    hash_u64(&mut hash, fixture.holes.len() as u64);
    for hole in &fixture.holes {
        hash_u64(&mut hash, hole.len() as u64);
        for point in hole {
            hash_point(&mut hash, *point);
        }
    }
    hash_u64(&mut hash, result.steiner.len() as u64);
    for point in &result.points[result.input_vertex_count as usize..] {
        hash_point(&mut hash, *point);
    }
    for triangle in &result.triangles {
        for &index in triangle {
            hash_u64(&mut hash, u64::from(index));
        }
    }
    hash
}

pub(crate) fn validate_cover(fixture: &Fixture, points: &[[f64; 2]], result: &Triangulation) {
    let mut triangle_area2 = 0.0;
    for &triangle in &result.triangles {
        for index in triangle {
            assert!(
                (index as usize) < points.len(),
                "{} emitted out-of-range input index {index}",
                fixture.name
            );
        }
        let [a, b, c] = triangle_points(points, triangle);
        let area2 = cross(a, b, c);
        assert!(
            area2 > 0.0,
            "{} emitted non-CCW triangle {triangle:?}",
            fixture.name
        );
        triangle_area2 += area2;
    }

    let expected_area2 = signed_area2(&fixture.outer)
        + fixture
            .holes
            .iter()
            .map(|hole| signed_area2(hole))
            .sum::<f64>();
    let scale = expected_area2.abs().max(f64::MIN_POSITIVE);
    assert!(
        (triangle_area2 - expected_area2).abs() <= scale * 1e-10,
        "{} area mismatch: triangles={triangle_area2:e} polygon={expected_area2:e}",
        fixture.name
    );
}

pub(crate) fn triangle_points(points: &[[f64; 2]], triangle: [u32; 3]) -> [[f64; 2]; 3] {
    [
        points[triangle[0] as usize],
        points[triangle[1] as usize],
        points[triangle[2] as usize],
    ]
}

pub(crate) fn signed_area2(loop_points: &[[f64; 2]]) -> f64 {
    let mut area2 = 0.0;
    for (index, &a) in loop_points.iter().enumerate() {
        let b = loop_points[(index + 1) % loop_points.len()];
        area2 += a[0] * b[1] - a[1] * b[0];
    }
    area2
}

pub(crate) fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

pub(crate) fn edge_sq(a: [f64; 2], b: [f64; 2]) -> f64 {
    let x = b[0] - a[0];
    let y = b[1] - a[1];
    x * x + y * y
}

pub(crate) fn normalized_quality(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let longest = edge_sq(a, b).max(edge_sq(b, c)).max(edge_sq(c, a));
    cross(a, b, c).abs() / longest
}

pub(crate) fn triangle_min_angle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    angle(b, a, c).min(angle(a, b, c)).min(angle(a, c, b))
}

pub(crate) fn angle(a: [f64; 2], vertex: [f64; 2], b: [f64; 2]) -> f64 {
    let u = [a[0] - vertex[0], a[1] - vertex[1]];
    let v = [b[0] - vertex[0], b[1] - vertex[1]];
    let cross = (u[0] * v[1] - u[1] * v[0]).abs();
    let dot = u[0] * v[0] + u[1] * v[1];
    cross.atan2(dot)
}

pub(crate) fn count_below(values: &[f64], threshold: f64) -> usize {
    values.partition_point(|&value| value < threshold)
}

pub(crate) fn first_percentile(sorted_values: &[f64]) -> f64 {
    let index = sorted_values.len().div_ceil(100).saturating_sub(1);
    sorted_values.get(index).copied().unwrap_or(0.0)
}

pub(crate) fn signature(fixture: &Fixture, result: &Triangulation) -> u64 {
    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, fixture.name.as_bytes());
    hash_u64(&mut hash, fixture.outer.len() as u64);
    for point in &fixture.outer {
        hash_point(&mut hash, *point);
    }
    hash_u64(&mut hash, fixture.holes.len() as u64);
    for hole in &fixture.holes {
        hash_u64(&mut hash, hole.len() as u64);
        for point in hole {
            hash_point(&mut hash, *point);
        }
    }
    for triangle in &result.triangles {
        for &index in triangle {
            hash_u64(&mut hash, u64::from(index));
        }
    }
    hash
}

pub(crate) fn hash_point(hash: &mut u64, point: [f64; 2]) {
    hash_u64(hash, point[0].to_bits());
    hash_u64(hash, point[1].to_bits());
}

pub(crate) fn hash_u64(hash: &mut u64, value: u64) {
    hash_bytes(hash, &value.to_le_bytes());
}

pub(crate) fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}
