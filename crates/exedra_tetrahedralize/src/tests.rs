// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use super::*;

/// Six times the volume of every live tetrahedron, summed.
fn volume6(tets: &Tetrahedralization) -> f64 {
    tets.tetrahedra()
        .map(|tet| {
            let [a, b, c, d] = tets.vertices(tet).map(|v| tets.point(v));
            let [u, v, w] = [b, c, d].map(|p| [p[0] - a[0], p[1] - a[1], p[2] - a[2]]);
            -(u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
                + u[2] * (v[0] * w[1] - v[1] * w[0]))
        })
        .sum()
}

fn random_points(count: usize, seed: u64, scale: f64) -> Vec<[f64; 3]> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            [0, 1, 2].map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 11) as f64 / (1_u64 << 53) as f64 * scale
            })
        })
        .collect()
}

#[test]
fn grids_split_every_cell_into_six_delaunay_tetrahedra() {
    // Uneven spacing still gives exact boxes, so the split stays Delaunay.
    let x = [0.0, 0.1, 0.35, 1.0];
    let y = [-2.0, 0.3, 0.7];
    let z = [5.0, 5.5];
    let tets = Tetrahedralization::grid(&x, &y, &z).expect("valid grid");
    assert_eq!(tets.tetrahedron_count(), 3 * 2 * 6);
    assert_eq!(tets.points().len(), 4 * 3 * 2);
    assert_eq!(tets.validate(), Ok(()));
    let expected = 6.0 * 1.0 * 2.7 * 0.5;
    assert!((volume6(&tets) - expected).abs() < 1e-12);
    let hull = tets
        .tetrahedra()
        .flat_map(|tet| tets.neighbors(tet))
        .filter(Option::is_none)
        .count();
    // Two triangles per boundary rectangle.
    assert_eq!(hull, 2 * 2 * (3 * 2 + 3 + 2));
}

#[test]
fn rejects_malformed_axes() {
    let good = [0.0, 1.0];
    for (axis, bad) in [
        (0, &[0.0][..]),
        (1, &[0.0, 0.0][..]),
        (2, &[1.0, 0.0][..]),
        (0, &[0.0, f64::NAN][..]),
    ] {
        let mut axes = [&good[..], &good[..], &good[..]];
        axes[axis] = bad;
        assert_eq!(
            Tetrahedralization::grid(axes[0], axes[1], axes[2]).unwrap_err(),
            TetError::InvalidAxis { axis }
        );
    }
}

#[test]
fn random_insertions_stay_delaunay_and_fill_the_box() {
    let axis = [0.0, 0.5, 1.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    for (n, point) in random_points(400, 7, 1.0).into_iter().enumerate() {
        let vertex = tets.insert(point).expect("inside the box");
        assert_eq!(tets.point(vertex), point);
        if n % 50 == 0 {
            assert_eq!(tets.validate(), Ok(()));
        }
    }
    assert_eq!(tets.validate(), Ok(()));
    assert!((volume6(&tets) - 6.0).abs() < 1e-9);
    assert_eq!(tets.stats().insertions, 400);
    // Every vertex belongs to some tetrahedron.
    let mut used = vec![false; tets.points().len()];
    for tet in tets.tetrahedra() {
        for vertex in tets.vertices(tet) {
            used[vertex.index()] = true;
        }
    }
    assert!(used.into_iter().all(|u| u));
}

#[test]
fn cospherical_and_coplanar_insertions_stay_valid() {
    // Lattice-aligned points sit on many circumspheres and planes at once.
    let axis = [0.0, 1.0, 2.0, 3.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    for i in 0..6 {
        for j in 0..6 {
            for k in 0..6 {
                let point = [i, j, k].map(|v| f64::from(v) * 0.5);
                tets.insert(point).expect("inside the box");
            }
        }
    }
    assert_eq!(tets.validate(), Ok(()));
    assert!((volume6(&tets) - 6.0 * 27.0).abs() < 1e-9);
}

#[test]
fn hull_points_split_hull_faces_and_vertices_coincide() {
    let axis = [0.0, 1.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    let corner = tets.insert([1.0, 1.0, 1.0]).expect("existing vertex");
    assert_eq!(tets.point(corner), [1.0, 1.0, 1.0]);
    assert_eq!(tets.stats().coincident, 1);
    assert_eq!(tets.points().len(), 8);

    for point in [
        [0.5, 0.25, 0.0],
        [0.0, 0.5, 0.5],
        [1.0, 0.0, 0.5],
        [0.3, 1.0, 0.9],
    ] {
        tets.insert(point).expect("on the hull");
        assert_eq!(tets.validate(), Ok(()));
    }
    assert!((volume6(&tets) - 6.0).abs() < 1e-12);

    assert_eq!(tets.insert([1.5, 0.5, 0.5]), Err(TetError::OutsideHull));
    assert_eq!(tets.insert([0.5, f64::NAN, 0.5]), Err(TetError::NonFinite));
    assert_eq!(tets.locate([2.0, 0.0, 0.0]), Location::Outside);
    assert!(matches!(
        tets.locate([0.5, 0.5, 0.5]),
        Location::Tetrahedron(_)
    ));
}

#[test]
fn faces_wind_outward() {
    let axis = [0.0, 1.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    tets.insert([0.4, 0.6, 0.5]).expect("inside");
    for tet in tets.tetrahedra() {
        for (i, vertex) in tets.vertices(tet).into_iter().enumerate() {
            let [a, b, c] = tets.face(tet, i).map(|v| tets.point(v));
            assert_eq!(orient3d(a, b, c, tets.point(vertex)), Orientation3d::Below);
        }
    }
}

#[test]
fn identical_input_gives_identical_tetrahedra() {
    let build = || {
        let axis = [0.0, 0.25, 1.0];
        let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
        for point in random_points(200, 11, 1.0) {
            tets.insert(point).expect("inside the box");
        }
        tets.tetrahedra()
            .map(|tet| (tets.vertices(tet), tets.neighbors(tet)))
            .collect::<Vec<_>>()
    };
    assert_eq!(build(), build());
}

#[test]
fn tiny_and_huge_scales_stay_exact() {
    for scale in [f64::from_bits(1) * 1024.0, 1e-200, 1e200] {
        let axis = [0.0, scale];
        let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
        for point in random_points(50, 3, 1.0) {
            tets.insert(point.map(|v| v * scale))
                .expect("inside the box");
        }
        assert_eq!(tets.validate(), Ok(()));
    }
}

#[test]
fn weighted_insertions_stay_regular() {
    let axis = [0.0, 0.5, 1.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    let points = random_points(300, 23, 1.0);
    let weights = random_points(300, 29, 0.002);
    let mut hidden = 0;
    for (point, weight) in points.into_iter().zip(weights) {
        match tets.insert_weighted(point, weight[0]) {
            Ok(vertex) => assert_eq!(tets.weight(vertex), weight[0]),
            Err(TetError::Hidden) => hidden += 1,
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(tets.validate(), Ok(()));
    assert!((volume6(&tets) - 6.0).abs() < 1e-9);
    assert_eq!(tets.stats().hidden_points, hidden);
}

#[test]
fn heavy_points_hide_light_ones() {
    let axis = [0.0, 1.0, 2.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    let light = tets.insert([1.05, 1.0, 0.95]).expect("inside");
    // A heavy ball swallows the light point and the interior grid vertex;
    // hull vertices keep unbounded power cells and are never hidden.
    let heavy = tets.insert_weighted([1.0, 1.0, 0.95], 0.3).expect("inside");
    let center = VertexId((3 + 1) * 3 + 1);
    assert_eq!(tets.point(center), [1.0, 1.0, 1.0]);
    assert!(tets.is_hidden(light));
    assert!(tets.is_hidden(center));
    assert!(!tets.is_hidden(heavy));
    assert!(!tets.is_hidden(VertexId(0)));
    assert_eq!(tets.stats().hidden_vertices, 2);
    assert_eq!(tets.validate(), Ok(()));
    // Unweighted points inside the heavy ball are refused, and walks that
    // would start at the hidden grid vertex still find their way.
    assert_eq!(tets.insert([0.95, 1.0, 1.0]), Err(TetError::Hidden));
    // A hidden vertex is still the vertex at its position.
    assert_eq!(tets.insert([1.05, 1.0, 0.95]), Ok(light));
    assert_eq!(tets.insert([1.0, 1.0, 1.0]), Ok(center));
    tets.insert([1.3, 1.6, 1.0])
        .expect("outside the heavy ball");
    assert_eq!(tets.validate(), Ok(()));
    assert!((volume6(&tets) - 6.0 * 8.0).abs() < 1e-9);
}

#[test]
fn near_walks_and_new_tetrahedra_follow_each_insertion() {
    let axis = [0.0, 1.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    let mut previous = tets.insert([0.5, 0.5, 0.5]).expect("inside");
    for point in random_points(200, 41, 1.0) {
        assert!(matches!(
            tets.locate_near(point, previous),
            Location::Tetrahedron(_)
        ));
        let vertex = tets.insert_near(point, 0.0, previous).expect("inside");
        // Exactly the new vertex's star was created, and nothing else.
        let created = tets.new_tetrahedra();
        assert!(!created.is_empty());
        assert!(created.iter().all(|&t| tets.is_live(t)));
        assert!(created.iter().all(|&t| tets.vertices(t).contains(&vertex)));
        let star = tets
            .tetrahedra()
            .filter(|&t| tets.vertices(t).contains(&vertex))
            .count();
        assert_eq!(created.len(), star);
        previous = vertex;
    }
    assert_eq!(tets.validate(), Ok(()));
}

#[test]
fn stars_hold_exactly_the_tetrahedra_around_a_vertex() {
    let axis = [0.0, 0.5, 1.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    for point in random_points(150, 53, 1.0) {
        tets.insert(point).expect("inside");
    }
    for index in 0..tets.points().len() {
        let vertex = VertexId(index_u32(index));
        let mut star = tets.star(vertex);
        star.sort_unstable();
        let mut expected: Vec<TetId> = tets
            .tetrahedra()
            .filter(|&t| tets.vertices(t).contains(&vertex))
            .collect();
        expected.sort_unstable();
        assert_eq!(star, expected, "vertex {index}");
    }
}

#[test]
fn spaced_insertions_hold_at_any_scale() {
    for scale in [1e-200, 1.0, 1e200] {
        let axis = [0.0, scale];
        let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
        // Every corner is sqrt(3)/2 of the spacing from the center.
        let center = [0.5 * scale; 3];
        assert!(
            matches!(
                tets.insert_spaced(center, 0.0, VertexId(0), scale),
                Err(TetError::Crowded { .. })
            ),
            "scale {scale}"
        );
        assert!(
            tets.insert_spaced(center, 0.0, VertexId(0), 0.8 * scale)
                .is_ok()
        );
    }
}

#[test]
fn spaced_insertions_refuse_points_near_a_vertex_and_change_nothing() {
    let axis = [0.0, 0.5, 1.0];
    let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
    let points = random_points(200, 71, 1.0);
    for &point in &points {
        tets.insert(point).expect("inside");
    }
    let before = (tets.points().len(), tets.tetrahedron_count());
    let near = VertexId(0);
    for (index, &point) in points.iter().enumerate() {
        let nudged = [point[0] + 1e-4, point[1], point[2]];
        let nearest = (0..tets.points().len())
            .map(|v| VertexId(index_u32(v)))
            .min_by(|&a, &b| {
                let d = |v: VertexId| exedra_math::distance_squared(tets.point(v), nudged);
                d(a).total_cmp(&d(b))
            })
            .expect("vertices");
        assert_eq!(
            tets.insert_spaced(nudged, 0.0, near, 1e-3),
            Err(TetError::Crowded { vertex: nearest }),
            "point {index}"
        );
    }
    assert_eq!((tets.points().len(), tets.tetrahedron_count()), before);
    assert_eq!(tets.stats().crowded_points, points.len());
    assert_eq!(tets.validate(), Ok(()));
    let apart = tets
        .insert_spaced([0.31, 0.62, 0.17], 0.0, near, 1e-6)
        .expect("far enough");
    assert_eq!(tets.point(apart), [0.31, 0.62, 0.17]);
    assert_eq!(tets.validate(), Ok(()));
}
