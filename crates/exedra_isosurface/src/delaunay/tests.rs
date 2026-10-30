// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use exedra_implicit::{
    Aabb, Cuboid, Cylinder, Difference, Intersection, ScalarField, Sphere, Tagged, Union,
};

use super::{DelaunayError, DelaunayParams, DelaunayResult, delaunay_mesh};

fn mesh(field: &dyn ScalarField, cells: f64) -> DelaunayResult {
    let bounds = Aabb::new([-1.0; 3], [1.0; 3]).unwrap();
    delaunay_mesh(field, &DelaunayParams::new(bounds, 2.0 / cells)).expect("valid parameters")
}

/// Each edge used by exactly two triangles, in opposite directions.
fn assert_closed(result: &DelaunayResult) {
    let mut edges: Vec<(u32, u32)> = result
        .triangles
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .collect();
    edges.sort_unstable();
    assert!(
        edges.windows(2).all(|w| w[0] != w[1]),
        "an edge is used twice the same way"
    );
    for &(a, b) in &edges {
        assert!(
            edges.binary_search(&(b, a)).is_ok(),
            "edge {a}-{b} has one side"
        );
    }
    assert!(result.mesh().is_ok());
}

/// The largest distance from a vertex, and from a triangle's centroid, to
/// the surface of an exact distance field.
fn deviation(field: &dyn ScalarField, result: &DelaunayResult) -> (f64, f64) {
    let value = |p: [f64; 3]| {
        let mut out = [0.0];
        field.eval_points(&[p], &mut out);
        out[0].abs()
    };
    let vertices = result
        .positions
        .iter()
        .map(|&p| value(p))
        .fold(0.0, f64::max);
    let centroids = result
        .triangles
        .iter()
        .map(|t| {
            let p = t.map(|i| result.positions[i as usize]);
            value(core::array::from_fn(|k| {
                (p[0][k] + p[1][k] + p[2][k]) / 3.0
            }))
        })
        .fold(0.0, f64::max);
    (vertices, centroids)
}

/// The smallest angle of any triangle, in degrees.
fn smallest_angle(result: &DelaunayResult) -> f64 {
    result
        .triangles
        .iter()
        .flat_map(|t| {
            let p = t.map(|i| result.positions[i as usize]);
            (0..3).map(move |k| {
                let u = exedra_math::sub(p[(k + 1) % 3], p[k]);
                let v = exedra_math::sub(p[(k + 2) % 3], p[k]);
                let cos = exedra_math::dot(u, v) / (exedra_math::norm(u) * exedra_math::norm(v));
                exedra_math::Real::acos(cos.clamp(-1.0, 1.0)).to_degrees()
            })
        })
        .fold(180.0, f64::min)
}

#[test]
fn smooth_surfaces_mesh_closed_close_and_well_shaped() {
    let sphere = Sphere::new([0.1, -0.05, 0.0], 0.7).unwrap();
    let result = mesh(&sphere, 16.0);
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert_closed(&result);
    let (vertices, centroids) = deviation(&sphere, &result);
    let h = 2.0 / 16.0;
    // Surface points are searched to within a 256th of a cell.
    assert!(vertices < h / 64.0, "vertices {vertices}");
    assert!(centroids < 0.03 * h, "centroids {centroids}");
    assert!(smallest_angle(&result) > 20.0);
    assert_eq!(result.report.crease_lines, 0);
}

#[test]
fn creases_and_corners_stay_sharp() {
    // A box drilled through: straight convex edges, corners where three
    // faces meet, and concave circular creases where the bore leaves it.
    let drilled = Difference::new(
        Cuboid::new([0.0; 3], [0.6; 3]).unwrap(),
        Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.3, 1.0).unwrap(),
    );
    let result = mesh(&drilled, 16.0);
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert_closed(&result);
    assert_eq!(result.report.corners, 8);
    // Every box corner is a vertex, exactly.
    for corner in Aabb::new([-0.6; 3], [0.6; 3]).unwrap().corners() {
        assert!(result.positions.contains(&corner), "corner {corner:?}");
    }
    let (vertices, centroids) = deviation(&drilled, &result);
    let h = 2.0 / 16.0;
    assert!(vertices < h / 64.0, "vertices {vertices}");
    assert!(centroids < 0.05 * h, "centroids {centroids}");
    // Crease edges lie on the box's edges or the bore's rims, and the mesh
    // marks exactly those sharp.
    for &[a, b] in &result.creases {
        let m: [f64; 3] = core::array::from_fn(|k| {
            0.5 * (result.positions[a as usize][k] + result.positions[b as usize][k])
        });
        let on_edge = m.iter().filter(|c| (c.abs() - 0.6).abs() < 1e-9).count() >= 2;
        let on_rim = (m[2].abs() - 0.6).abs() < 1e-9 && (m[0].hypot(m[1]) - 0.3).abs() < 0.01;
        assert!(on_edge || on_rim, "crease edge through {m:?}");
    }
    let mesh = result.mesh().unwrap();
    let sharp = mesh
        .half_edges()
        .filter(|&e| mesh.canonical_edge(e) == Some(e))
        .filter(|&e| mesh.edge_sharpness(e).is_some_and(|s| s > 0.0))
        .count();
    assert_eq!(sharp, result.creases.len());
}

#[test]
fn faces_take_the_region_of_their_part() {
    // A post with its ends cut flat: circular rims, each traced as a loop.
    let post = Intersection::new(
        Tagged::new(Cuboid::new([0.0; 3], [1.0; 3]).unwrap(), 10),
        Tagged::new(
            Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.72, 1.35).unwrap(),
            20,
        ),
    );
    let bounds = Aabb::new([-1.5; 3], [1.5; 3]).unwrap();
    let mut params = DelaunayParams::new(bounds, 3.0 / 32.0);
    params.regions = true;
    let result = delaunay_mesh(&post, &params).unwrap();
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert_eq!(result.regions.len(), result.triangles.len());
    for (triangle, &region) in result.triangles.iter().zip(&result.regions) {
        let p = triangle.map(|i| result.positions[i as usize]);
        let c: [f64; 3] = core::array::from_fn(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0);
        let radius = c[0].hypot(c[1]);
        match region {
            10 => assert!((c[2].abs() - 1.0).abs() < 1e-9 && radius < 0.72, "{c:?}"),
            20 => assert!((radius - 0.72).abs() < 0.01 && c[2].abs() < 1.0, "{c:?}"),
            _ => panic!("region {region}"),
        }
    }
    // The parts' faces meet only along the rims, all the way round.
    let mut regions = alloc::vec![0_u8; result.positions.len()];
    for (triangle, &region) in result.triangles.iter().zip(&result.regions) {
        for &v in triangle {
            regions[v as usize] |= if region == 10 { 1 } else { 2 };
        }
    }
    for (p, _) in result
        .positions
        .iter()
        .zip(&regions)
        .filter(|(_, r)| **r == 3)
    {
        assert!((p[0].hypot(p[1]) - 0.72).abs() < 1e-12, "{p:?}");
        assert!((p[2].abs() - 1.0).abs() < 1e-12, "{p:?}");
    }
    let mesh = result.mesh().unwrap();
    let ends = result.regions.iter().filter(|&&r| r == 10).count();
    assert!(ends > 0);
    assert_eq!(mesh.faces_in_region(10).count(), ends);
}

#[test]
fn creases_are_followed_only_where_the_mesh_reaches() {
    // A slab's edge runs just outside the bounds, for 2e300 along z.
    let slab = Cuboid::new([-34.5, 63.2, 0.0], [81.8, 69.9, 1e300]).unwrap();
    let bounds = Aabb::new([-23.2, -23.0, -44.9], [42.9, 40.7, -12.0]).unwrap();
    let result = delaunay_mesh(&slab, &DelaunayParams::new(bounds, 11.0)).unwrap();
    assert!(result.report.crease_samples < 100, "{:?}", result.report);
    assert!(result.report.clipped);
}

#[test]
fn fields_too_large_to_mesh_are_reported_not_chased() {
    // At this size no surface forms between samples; splitting crease
    // balls could not help, and must not run away.
    let huge = Difference::new(
        Cuboid::new([78.1, 0.0, 0.0], [1e300; 3]).unwrap(),
        Sphere::new([0.0; 3], 1.0).unwrap(),
    );
    let bounds = Aabb::new([-1.7e300; 3], [1.7e300; 3]).unwrap();
    let mut params = DelaunayParams::new(bounds, 5.6e299);
    params.max_points = 2000;
    let result = delaunay_mesh(&huge, &params).unwrap();
    assert!(!result.report.is_complete());
    assert_eq!(result.report.ball_splits, 0, "{:?}", result.report);
}

#[test]
fn short_edges_between_corners_stay_sharp() {
    // A plate under two cells thick: its short edges are shorter than a
    // crease step, and run corner to corner.
    let plate = Cuboid::new([0.0; 3], [0.7, 0.5, 0.1]).unwrap();
    let result = mesh(&plate, 16.0);
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert_eq!(result.report.crease_lines, 12);
    assert_eq!(result.report.corners, 8);
    // Each short edge is followed by sharp edges.
    for (x, y) in [(0.7, 0.5), (0.7, -0.5), (-0.7, 0.5), (-0.7, -0.5)] {
        let on_edge = |i: u32| {
            let p = result.positions[i as usize];
            (p[0] - x).abs() < 1e-9 && (p[1] - y).abs() < 1e-9
        };
        let along = result
            .creases
            .iter()
            .filter(|&&[a, b]| on_edge(a) && on_edge(b));
        assert!(along.count() >= 2, "edge at ({x}, {y})");
    }
}

#[test]
fn both_sides_of_a_thin_fin_stay_sharp() {
    // A fin under half a cell thick: a ball on one side's edges would
    // reach the other's.
    let fin = Cuboid::new([0.0; 3], [0.025, 0.6, 0.5]).unwrap();
    let result = mesh(&fin, 16.0);
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert_closed(&result);
    assert_eq!(result.report.corners, 8);
    // Every long edge, on either side, is followed by sharp edges.
    for (x, z) in [(0.025, 0.5), (-0.025, 0.5), (0.025, -0.5), (-0.025, -0.5)] {
        let on_edge = |i: u32| {
            let p = result.positions[i as usize];
            (p[0] - x).abs() < 1e-9 && (p[2] - z).abs() < 1e-9
        };
        let along = result
            .creases
            .iter()
            .filter(|&&[a, b]| on_edge(a) && on_edge(b))
            .count();
        assert!(along >= 8, "edge at x {x}, z {z}: {along}");
    }
}

#[test]
fn every_edge_of_a_thin_rod_stays_sharp() {
    // A rod half a cell across, running out of the bounds: opposite edges
    // have parallel ties, within a ball of each other, and all four end
    // side by side at the bounds.
    let rod = Cuboid::new([0.0; 3], [0.03, 0.03, 2.0]).unwrap();
    let result = mesh(&rod, 16.0);
    let report = &result.report;
    assert!(report.clipped && report.broken_creases == 0, "{report:?}");
    assert_eq!((report.crease_lines, report.corners), (4, 8), "{report:?}");
    for (x, y) in [(0.03, 0.03), (0.03, -0.03), (-0.03, 0.03), (-0.03, -0.03)] {
        let on_edge = |i: u32| {
            let p = result.positions[i as usize];
            (p[0] - x).abs() < 1e-9 && (p[1] - y).abs() < 1e-9
        };
        let along = result
            .creases
            .iter()
            .filter(|&&[a, b]| on_edge(a) && on_edge(b))
            .count();
        assert!(along >= 8, "edge at ({x}, {y}): {along}");
    }
    // No crease runs across the rod.
    let across = result.creases.iter().filter(|&&[a, b]| {
        let [p, q] = [a, b].map(|i| result.positions[i as usize]);
        (p[0] - q[0]).abs() > 1e-9 && (p[1] - q[1]).abs() > 1e-9
    });
    assert_eq!(across.count(), 0);
}

#[test]
fn creases_leaving_a_corner_close_together_stay_apart() {
    // A wedge ten degrees across, capped: at each end of its sharp edge,
    // two cap edges leave the corner closer together than a step, and the
    // back edges run corner to corner, under two steps long. (The sharp
    // edge is a knife edge, whose mesh this does not judge.)
    struct Wedge;
    impl exedra_implicit::Formula for Wedge {
        fn eval<N: exedra_implicit::Number>(&self, [x, y, z]: [N; 3]) -> N {
            let (sin, cos) = (0.087_155_742_747_658_17, 0.996_194_698_091_745_5);
            let side = |s: f64| y * (s * cos) - x * sin;
            side(1.0).max(side(-1.0)).max(x - 0.7).max(z.abs() - 0.3)
        }
    }
    let report = mesh(&Wedge, 16.0).report;
    assert_eq!(report.broken_creases, 0, "{report:?}");
    assert_eq!((report.crease_lines, report.corners), (9, 6), "{report:?}");
}

#[test]
fn features_thinner_than_a_cell_are_found() {
    // A plate two fifths of a cell thick, on a post.
    let plate = Union::new(
        Cuboid::new([0.0, 0.0, 0.3], [0.6, 0.6, 0.025]).unwrap(),
        Cuboid::new([0.0; 3], [0.1, 0.1, 0.3]).unwrap(),
    );
    let result = mesh(&plate, 16.0);
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert_closed(&result);
    // The plate's far corners are reached.
    let reach = result
        .positions
        .iter()
        .map(|p| p[0].abs().max(p[1].abs()))
        .fold(0.0, f64::max);
    assert!(reach > 0.59, "plate reaches {reach}");
}

#[test]
fn coarse_samples_still_find_small_surfaces() {
    // At one cell across, the seeds on a sphere crowd each other out; the
    // ones left out are brought back where the surface would be lost.
    let sphere = Sphere::new([0.0; 3], 0.7).unwrap();
    let result = mesh(&sphere, 2.0);
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert!(!result.triangles.is_empty());
    assert_closed(&result);
}

#[test]
fn lines_trimmed_away_at_their_corners_are_dropped() {
    // Steps as long as the box: a trace can hold one sample between two
    // corners, which trimming then removes.
    let cuboid = Cuboid::new([0.0; 3], [1.0; 3]).unwrap();
    let bounds = Aabb::new([-1.5; 3], [1.5; 3]).unwrap();
    let mut params = DelaunayParams::new(bounds, 0.5);
    params.crease.max_step = 3.0;
    params.crease.min_step = 1.5;
    let result = delaunay_mesh(&cuboid, &params).unwrap();
    assert_eq!(result.report.corners, 8);
    assert_closed(&result);
}

#[test]
fn identical_input_gives_identical_meshes() {
    let field = Difference::new(
        Sphere::new([0.0; 3], 0.8).unwrap(),
        Cuboid::new([0.5, 0.5, 0.5], [0.4; 3]).unwrap(),
    );
    let (a, b) = (mesh(&field, 12.0), mesh(&field, 12.0));
    assert_eq!(a.positions, b.positions);
    assert_eq!(a.triangles, b.triangles);
    assert_eq!(a.report, b.report);
}

#[test]
fn interiors_reaching_the_bounds_are_reported() {
    let large = Sphere::new([0.0; 3], 1.5).unwrap();
    let result = mesh(&large, 8.0);
    assert!(result.report.clipped);
    assert!(!result.report.is_complete());
}

#[test]
fn clipping_is_judged_at_the_bounds_asked_for() {
    // Cells of 0.3 do not divide 2; the sphere crosses x = 1 but not where
    // whole cells would end.
    let sphere = Sphere::new([0.8, 0.0, 0.0], 0.25).unwrap();
    let bounds = Aabb::new([-1.0; 3], [1.0; 3]).unwrap();
    let result = delaunay_mesh(&sphere, &DelaunayParams::new(bounds, 0.3)).unwrap();
    assert!(result.report.clipped);
}

#[test]
fn empty_space_is_skipped_by_bounds() {
    let small = Sphere::new([0.6, 0.6, 0.6], 0.15).unwrap();
    let result = mesh(&small, 32.0);
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert!(result.report.empty_blocks * 2 > result.report.blocks);
}

#[test]
fn invalid_parameters_are_refused() {
    let sphere = Sphere::new([0.0; 3], 0.5).unwrap();
    let bounds = Aabb::new([-1.0; 3], [1.0; 3]).unwrap();
    let mut params = DelaunayParams::new(bounds, 0.1);
    params.cell_size = 0.0;
    assert_eq!(
        delaunay_mesh(&sphere, &params).unwrap_err(),
        DelaunayError::InvalidParameter { name: "cell_size" }
    );
    let mut params = DelaunayParams::new(bounds, 0.1);
    params.crease.max_step = 0.001;
    assert!(delaunay_mesh(&sphere, &params).is_err());
    let tiny = DelaunayParams::new(bounds, 1e-9);
    assert_eq!(
        delaunay_mesh(&sphere, &tiny).unwrap_err(),
        DelaunayError::TooManyCells
    );
}

#[test]
fn convex_edges_are_traced_from_outside_and_whole() {
    // A post standing out of a box's face: the post's rims curve away from
    // the solid, and each box edge reaches corners at both ends.
    let field = Union::new(
        Cuboid::new([0.0; 3], [0.8; 3]).unwrap(),
        Cylinder::new([0.65, 0.0, 0.0], [0.0, 0.0, 1.0], 0.55, 0.6).unwrap(),
    );
    let bounds = Aabb::new([-1.5; 3], [1.5; 3]).unwrap();
    let result = delaunay_mesh(&field, &DelaunayParams::new(bounds, 3.0 / 32.0)).unwrap();
    assert!(result.report.is_complete(), "{:?}", result.report);
    assert_closed(&result);
    // Twelve box edges; two rims and four seams against the face.
    assert_eq!(result.report.crease_lines, 18);
    assert_eq!(result.report.corners, 12);
}

#[test]
fn creases_keep_to_their_side_of_a_thin_fin() {
    use super::{Settings, crease::Creases, sample::Sampler, seed};
    // Fins two fifths of a cell thick on a plate, all cut by a ball: each
    // side of a fin meets the plate in its own crease, closer together
    // than a step.
    struct Fins;
    impl exedra_implicit::Formula for Fins {
        fn eval<N: exedra_implicit::Number>(&self, [x, y, z]: [N; 3]) -> N {
            let plate = (z + 0.5).abs() - 0.08;
            let fin = |offset: f64| (x - offset).abs() - 0.012;
            let fins = fin(-0.4).min(fin(-0.1)).min(fin(0.2)).min(fin(0.5));
            let body = plate.min(fins.max(z - 0.6).max(-z - 0.5));
            body.max(x.hypot(y).hypot(z) - 0.9)
        }
    }
    let field = Fins;
    let bounds = Aabb::new([-1.0; 3], [1.0; 3]).unwrap();
    let params = DelaunayParams::new(bounds, 2.0 / 32.0);
    let settings = Settings::new(&params).unwrap();
    let sampler = Sampler::new(&field);
    let grid = seed::Grid::covering(&params.bounds, settings.cell);
    let seeds = seed::seed(&sampler, &settings, &grid);
    let creases = Creases::trace(&sampler, &seeds.creases, &settings);
    let mut sides = Vec::new();
    for line in &creases.lines {
        // Along the fin's foot, every sample on one side.
        let foot: Vec<f64> = line
            .iter()
            .filter(|p| (p[2] + 0.42).abs() < 1e-9 && (p[0] + 0.1).abs() < 0.02 && p[1] < 0.0)
            .map(|p| p[0])
            .collect();
        if let Some(&first) = foot.first() {
            assert!(foot.iter().all(|&x| (x - first).abs() < 1e-9), "{foot:?}");
            sides.push(first);
        }
    }
    // Both sides are traced, each once.
    for side in [-0.112, -0.088] {
        let lines = sides.iter().filter(|&&x| (x - side).abs() < 1e-9).count();
        assert_eq!(lines, 1, "side at {side}: {sides:?}");
    }
}

#[test]
fn creases_joining_the_same_corners_stay_apart() {
    use super::{Settings, crease::Creases, sample::Sampler};
    // A capped lens: the rims of its top face run between the same two
    // corners with nearly the same ties, 0.4 apart at their middles.
    struct Lens;
    impl exedra_implicit::Formula for Lens {
        fn eval<N: exedra_implicit::Number>(&self, [x, y, z]: [N; 3]) -> N {
            let disc = |cx: f64| (x - cx).hypot(y) - 1.0;
            disc(0.8).max(disc(-0.8)).max((z.abs() - 0.2) * 0.1)
        }
    }
    let field = Lens;
    let bounds = Aabb::new([-1.0; 3], [1.0; 3]).unwrap();
    let params = DelaunayParams::new(bounds, 0.125);
    let settings = Settings::new(&params).unwrap();
    let sampler = Sampler::new(&field);
    // Seeded on one rim only, so corner completion must find the other.
    let creases = Creases::trace(&sampler, &[[0.2, 0.0, 0.2]], &settings);
    let rim = |side: f64| {
        creases.lines.iter().any(|line| {
            line.iter()
                .any(|p| (p[2] - 0.2).abs() < 1e-6 && p[0] * side > 0.1 && p[1].abs() < 0.2)
        })
    };
    assert!(rim(1.0) && rim(-1.0), "{:?}", creases.lines);
}
