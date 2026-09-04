// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! SVG gallery rendering for benchmark results.

use crate::fixtures::Fixture;
use crate::metrics::refine_params;
use crate::{strategy_label, strategy_params};
use exedra_triangulate::{TriStrategy, refine, triangulate};

pub(crate) fn write_svg_gallery(directory: &std::path::Path, fixtures: &[Fixture]) {
    std::fs::create_dir_all(directory).expect("svg directory is writable");
    for fixture in fixtures {
        let prepared = fixture.prepare();
        let input = prepared.input();
        for strategy in [TriStrategy::EarClip, TriStrategy::ConstrainedDelaunay] {
            let result = triangulate(&input, &strategy_params(strategy)).expect("fixture");
            let points = fixture.points();
            let svg = triangulation_svg(&points, points.len(), &result.triangles);
            let path = directory.join(format!("{}_{}.svg", fixture.name, strategy_label(strategy)));
            std::fs::write(path, svg).expect("svg file is writable");
        }
        let refined = refine(&input, &refine_params()).expect("fixture refines");
        let svg = triangulation_svg(
            &refined.points,
            refined.input_vertex_count as usize,
            &refined.triangles,
        );
        let path = directory.join(format!("{}_Refined.svg", fixture.name));
        std::fs::write(path, svg).expect("svg file is writable");
    }
}

fn triangulation_svg(points: &[[f64; 2]], input_count: usize, triangles: &[[u32; 3]]) -> String {
    use std::fmt::Write as _;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for point in points {
        min_x = min_x.min(point[0]);
        min_y = min_y.min(point[1]);
        max_x = max_x.max(point[0]);
        max_y = max_y.max(point[1]);
    }
    let span = (max_x - min_x).max(max_y - min_y).max(f64::MIN_POSITIVE);
    let size = 720.0;
    let margin = 24.0;
    let scale = (size - 2.0 * margin) / span;
    let map = |point: [f64; 2]| {
        (
            margin + (point[0] - min_x) * scale,
            size - margin - (point[1] - min_y) * scale,
        )
    };
    let width = margin * 2.0 + (max_x - min_x) * scale;
    let height = margin * 2.0 + (max_y - min_y) * scale;
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 {:.1} {width:.1} {height:.1}\">",
        size - height
    );
    let _ = writeln!(
        svg,
        "<rect x=\"0\" y=\"{:.1}\" width=\"100%\" height=\"100%\" fill=\"white\"/>",
        size - height
    );
    for triangle in triangles {
        let [a, b, c] = triangle.map(|index| map(points[index as usize]));
        let _ = writeln!(
            svg,
            "<polygon points=\"{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}\" fill=\"#dde8f5\" stroke=\"#1f3b63\" stroke-width=\"1\" stroke-linejoin=\"round\"/>",
            a.0, a.1, b.0, b.1, c.0, c.1
        );
    }
    for (index, point) in points.iter().enumerate() {
        let (x, y) = map(*point);
        if index < input_count {
            let _ = writeln!(
                svg,
                "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"3\" fill=\"white\" stroke=\"#1f3b63\" stroke-width=\"1.2\"/>"
            );
        } else {
            let _ = writeln!(
                svg,
                "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"3\" fill=\"#c8401e\"/>"
            );
        }
    }
    svg.push_str("</svg>\n");
    svg
}
