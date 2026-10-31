// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native mesh timing, shape measurements and wireframes without render extraction.
use exedra::analytic::{
    body::{Body, EdgeId},
    extrusion::Extrusion,
    machining::{BoreDepth, BoreStep, BoreTool, MachiningPolicy, subtract_bore},
    realize::{RealizationPolicy, RealizedBody, TriangleQuality},
};
use exedra::constructive::ir::Placement3;
use exedra::profile::{Loop, Profile, ProfilePolicy, Segment};
use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::{PI, TAU},
    fmt::Write as _,
    hint::black_box,
    io,
    path::Path,
    time::Instant,
};

type Triangle = [[f64; 3]; 3];

#[derive(Clone, Copy)]
enum Fixture {
    Plate(usize),
    Pocket,
    Cylinder,
}
impl Fixture {
    fn name(self) -> String {
        match self {
            Self::Plate(n) => format!("plate-{n}-holes"),
            Self::Pocket => "blind-pocket".into(),
            Self::Cylinder => "cylinder".into(),
        }
    }
    fn centers(self) -> Vec<[f64; 2]> {
        match self {
            Self::Plate(n) => (0..n).map(|i| [-24.0 + 16.0 * scalar(i), -8.0]).collect(),
            Self::Pocket | Self::Cylinder => vec![[0.0; 2]],
        }
    }
    fn radius(self) -> f64 {
        if matches!(self, Self::Pocket) {
            5.0
        } else {
            2.0
        }
    }
    fn height(self) -> f64 {
        if matches!(self, Self::Cylinder) {
            18.0
        } else {
            6.0
        }
    }
    fn body(self) -> Body {
        let outer = if matches!(self, Self::Cylinder) {
            Loop::new(vec![
                Segment::arc([2.0, 0.0], 1.0),
                Segment::arc([-2.0, 0.0], 1.0),
            ])
            .unwrap()
        } else {
            Loop::new(vec![
                Segment::line([-40.0, -20.0]),
                Segment::line([40.0, -20.0]),
                Segment::line([40.0, 20.0]),
                Segment::line([-40.0, 20.0]),
            ])
            .unwrap()
        };
        let holes = if matches!(self, Self::Plate(_)) {
            self.centers()
                .into_iter()
                .map(|[x, y]| {
                    Loop::new(vec![
                        Segment::arc([x + 2.0, y], -1.0),
                        Segment::arc([x - 2.0, y], -1.0),
                    ])
                    .unwrap()
                })
                .collect()
        } else {
            vec![]
        };
        let extrusion = Extrusion::new(
            Profile::new(outer, holes, &ProfilePolicy::new(1e-6)).unwrap(),
            Placement3::IDENTITY,
            self.height(),
        )
        .unwrap();
        let body = extrusion.body().clone().map_roles(|_| (), |_| ());
        if matches!(self, Self::Pocket) {
            let tool = BoreTool::new(
                [0.0, 0.0, 6.0],
                [0.0, 0.0, -1.0],
                &[BoreStep {
                    radius: 5.0,
                    end: BoreDepth::ToDepth(4.0),
                }],
            )
            .unwrap();
            subtract_bore(&body, &tool, &MachiningPolicy::default())
                .unwrap()
                .body
        } else {
            body
        }
    }
    fn volume(self) -> f64 {
        match self {
            Self::Plate(n) => (3200.0 - scalar(n) * 4.0 * PI) * 6.0,
            Self::Pocket => 19200.0 - 100.0 * PI,
            Self::Cylinder => 4.0 * PI * 18.0,
        }
    }
    fn contains(self, p: [f64; 3]) -> bool {
        let radial = |[x, y]: [f64; 2]| (p[0] - x).powi(2) + (p[1] - y).powi(2);
        if !(p[2] > 0.0 && p[2] < self.height()) {
            return false;
        }
        if matches!(self, Self::Cylinder) {
            return radial([0.0; 2]) < 4.0;
        }
        p[0].abs() < 40.0
            && p[1].abs() < 20.0
            && match self {
                Self::Plate(_) => self.centers().iter().all(|&c| radial(c) > 4.0),
                Self::Pocket => p[2] < 2.0 || radial([0.0; 2]) > 25.0,
                Self::Cylinder => unreachable!(),
            }
    }
}

fn scalar(n: usize) -> f64 {
    f64::from(u32::try_from(n).expect("bounded sample size"))
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn median(mut run: impl FnMut(), repetitions: usize) -> f64 {
    run();
    let mut times = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let start = Instant::now();
        run();
        times.push(start.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}
fn triangles(output: &RealizedBody) -> Vec<(u32, [u32; 3], Triangle)> {
    output
        .mesh_faces
        .iter()
        .map(|&(face, native)| {
            let vertices: Vec<_> = output
                .mesh
                .face_loop(face)
                .map(|h| output.mesh.from_vertex(h).unwrap())
                .collect();
            assert_eq!(vertices.len(), 3, "Body realization must emit triangles");
            let ids = std::array::from_fn(|i| vertices[i].index());
            let points = std::array::from_fn(|i| {
                output
                    .mesh
                    .vertex_position(vertices[i])
                    .unwrap()
                    .map(f64::from)
            });
            (native.0, ids, points)
        })
        .collect()
}
fn shape(triangle: Triangle) -> (f64, f64, f64) {
    let mut edges: [f64; 3] =
        std::array::from_fn(|i| norm(sub(triangle[i], triangle[(i + 1) % 3])));
    edges.sort_by(f64::total_cmp);
    let [a, b, c] = edges;
    let angle = ((b * b + c * c - a * a) / (2.0 * b * c))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    let area = 0.5
        * norm(cross(
            sub(triangle[1], triangle[0]),
            sub(triangle[2], triangle[0]),
        ));
    // Same normalized longest-edge/altitude measure as the Truck harness: one for equilateral.
    (
        angle,
        3.0_f64.sqrt() * c * c / (4.0 * area),
        b * c / (4.0 * area),
    )
}
fn inside(triangles: &[Triangle], point: [f64; 3]) -> bool {
    let angle: f64 = triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|p| sub(p, point));
            let numerator = dot(a, cross(b, c));
            let denominator = norm(a) * norm(b) * norm(c)
                + dot(a, b) * norm(c)
                + dot(b, c) * norm(a)
                + dot(c, a) * norm(b);
            2.0 * numerator.atan2(denominator)
        })
        .sum();
    angle.abs() > TAU
}
fn membership(triangles: &[Triangle], fixture: Fixture) -> (usize, usize) {
    let mut probes = Vec::new();
    for x in [-43.0, -31.0, -17.0, -3.0, 0.137, 11.0, 27.0, 42.0] {
        for y in [-23.0, -13.0, 0.219, 13.0, 23.0] {
            for z in [-1.0, 1.123, 3.123, 7.0, 17.123, 19.0] {
                probes.push([x, y, z]);
            }
        }
    }
    for [x, y] in fixture.centers() {
        for offset in [0.0, fixture.radius() * 0.75, fixture.radius() * 1.25] {
            for z in [1.123, 3.123, fixture.height() - 0.123] {
                probes.push([x + offset, y, z]);
            }
        }
    }
    (
        probes
            .iter()
            .filter(|&&p| inside(triangles, p) != fixture.contains(p))
            .count(),
        probes.len(),
    )
}

fn policy(mode: &str) -> RealizationPolicy {
    match mode {
        "default" => RealizationPolicy::default(),
        "quality" => RealizationPolicy {
            quality: Some(
                TriangleQuality::new(4.0)
                    .with_max_radius_edge_ratio(2.5)
                    .with_max_steiner_points(4096),
            ),
            ..RealizationPolicy::default()
        },
        _ => panic!("mesh mode must be default or quality"),
    }
}

/// Isolated allocation-profiler protocol: one plate realization, source and warmup excluded.
/// The default mode is quality; set `EXEDRA_NATIVE_QUALITY_MODE=default` for the fast path.
pub fn allocation_window(mut checkpoint: impl FnMut(&'static str)) {
    let body = Fixture::Plate(4).body();
    let mode = std::env::var("EXEDRA_NATIVE_QUALITY_MODE").unwrap_or_else(|_| "quality".into());
    let policy = policy(&mode);
    black_box(body.realize(Placement3::IDENTITY, &policy).unwrap());
    checkpoint("ready");
    let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
    black_box(&output.report);
    checkpoint("retained");
    drop(output);
    checkpoint("released");
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--profile") {
        let iterations = args
            .get(2)
            .map_or(10000, |s| s.parse::<u32>().expect("profile iterations"));
        let mode = args.get(3).map_or("quality", String::as_str);
        let policy = policy(mode);
        let body = Fixture::Plate(4).body();
        black_box(body.realize(Placement3::IDENTITY, &policy).unwrap());
        println!(
            "profile_pid={} iterations={iterations} mode={mode}",
            std::process::id()
        );
        for _ in 0..iterations {
            black_box(
                body.realize(Placement3::IDENTITY, black_box(&policy))
                    .unwrap(),
            );
        }
        return Ok(());
    }
    let repetitions = args
        .get(1)
        .map_or(31, |s| s.parse::<usize>().expect("positive repetitions"));
    assert!(repetitions > 0, "timing repetitions must be positive");
    let directory = Path::new(
        args.get(2)
            .map_or(".local/native-tessellation-quality", String::as_str),
    );
    let label = args.get(3).map_or("current", String::as_str);
    let mode = args.get(4).map_or("default", String::as_str);
    assert!(
        label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "artifact label must contain only ASCII letters, digits, hyphens or underscores"
    );
    std::fs::create_dir_all(directory)?;
    let policy = policy(mode);
    let mut json = String::from("{\"schema\":\"exedra.native-mesh-quality.v1\",\"label\":\"");
    write!(
        json,
        "{label}\",\"mode\":\"{mode}\",\"repetitions\":{repetitions},\"tolerance\":{},\"rows\":[",
        policy.tolerance
    )
    .unwrap();
    let mut csv = String::from(
        "label,mode,fixture,construction_us,realization_us,vertices,triangles,native_faces,min_angle_deg,p5_angle_deg,median_aspect,p95_aspect,max_radius_edge_ratio,triangles_above_radius_edge_2_5,reported_deviation,chord_midpoint_error,exact_volume,mesh_volume,bad_edges,membership_failures,membership_probes\n",
    );
    for (i, fixture) in [
        Fixture::Plate(0),
        Fixture::Plate(1),
        Fixture::Plate(4),
        Fixture::Pocket,
        Fixture::Cylinder,
    ]
    .into_iter()
    .enumerate()
    {
        let construction = median(
            || {
                black_box(fixture.body());
            },
            repetitions,
        );
        let body = fixture.body();
        let realization = median(
            || {
                black_box(
                    body.realize(Placement3::IDENTITY, black_box(&policy))
                        .unwrap(),
                );
            },
            repetitions,
        );
        let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
        // Extraction, independent checks, statistics and SVG/file output are outside timing windows.
        let records = triangles(&output);
        let triangles: Vec<_> = records.iter().map(|r| r.2).collect();
        let mut angles = Vec::new();
        let mut aspects = Vec::new();
        let mut max_radius_edge_ratio = 0.0_f64;
        let mut above_radius_edge_2_5 = 0_usize;
        let mut provenance = BTreeMap::<u32, usize>::new();
        let mut edges = BTreeMap::<(u32, u32), (usize, i32)>::new();
        let mut boundary_segments = BTreeSet::new();
        for edge in 0..body.edges().len() {
            let chain = output
                .edge_chains
                .get(EdgeId(u32::try_from(edge).unwrap()))
                .unwrap();
            for pair in chain.windows(2) {
                let a = pair[0].index();
                let b = pair[1].index();
                boundary_segments.insert(if a < b { (a, b) } else { (b, a) });
            }
        }
        let mut volume = 0.0;
        let mut chord_error = 0.0_f64;
        let mut triangle_csv = String::from(
            "triangle,native_face,v0,v1,v2,x0,y0,z0,x1,y1,z1,x2,y2,z2,min_angle_deg,normalized_aspect,radius_edge_ratio\n",
        );
        for (j, &(native, ids, triangle)) in records.iter().enumerate() {
            *provenance.entry(native).or_default() += 1;
            let (angle, aspect, radius_edge_ratio) = shape(triangle);
            assert!(
                angle.is_finite() && aspect.is_finite() && radius_edge_ratio.is_finite(),
                "emitted triangles must have finite shape measurements"
            );
            angles.push(angle);
            aspects.push(aspect);
            max_radius_edge_ratio = max_radius_edge_ratio.max(radius_edge_ratio);
            above_radius_edge_2_5 += usize::from(radius_edge_ratio > 2.5);
            volume += dot(triangle[0], cross(triangle[1], triangle[2])) / 6.0;
            write!(
                triangle_csv,
                "{j},{native},{},{},{}",
                ids[0], ids[1], ids[2]
            )
            .unwrap();
            for p in triangle {
                for v in p {
                    write!(triangle_csv, ",{v:.12}").unwrap();
                }
            }
            writeln!(
                triangle_csv,
                ",{angle:.9},{aspect:.9},{radius_edge_ratio:.9}"
            )
            .unwrap();
            for k in 0..3 {
                let (a, b) = (ids[k], ids[(k + 1) % 3]);
                let (key, sign) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                let edge = edges.entry(key).or_default();
                edge.0 += 1;
                edge.1 += sign;
                let (a, b) = (triangle[k], triangle[(k + 1) % 3]);
                for [x, y] in fixture
                    .centers()
                    .into_iter()
                    .filter(|_| boundary_segments.contains(&key))
                {
                    let radius = |p: [f64; 3]| (p[0] - x).hypot(p[1] - y);
                    if (radius(a) - fixture.radius()).abs() < 1e-4
                        && (radius(b) - fixture.radius()).abs() < 1e-4
                        && (a[2] - b[2]).abs() < 1e-4
                    {
                        chord_error = chord_error.max(
                            (fixture.radius()
                                - radius(std::array::from_fn(|k| (a[k] + b[k]) * 0.5)))
                            .abs(),
                        );
                    }
                }
            }
        }
        angles.sort_by(f64::total_cmp);
        aspects.sort_by(f64::total_cmp);
        let (min_angle, p5, median_aspect, p95) = (
            angles[0],
            angles[angles.len() / 20],
            aspects[aspects.len() / 2],
            aspects[aspects.len() * 95 / 100],
        );
        let bad_edges = edges
            .values()
            .filter(|&&(count, orientation)| count != 2 || orientation != 0)
            .count();
        let (failures, probes) = membership(&triangles, fixture);
        assert_eq!(
            bad_edges,
            0,
            "{} must be closed by indexed edge incidence",
            fixture.name()
        );
        assert_eq!(failures, 0, "{} independent membership", fixture.name());
        assert!(
            output.mesh.validate_deep().is_empty(),
            "emitted mesh must pass deep validation"
        );
        let name = fixture.name();
        let vertices = output.mesh_vertices.len();
        let count = triangles.len();
        let faces = provenance.len();
        let deviation = output.report.max_deviation;
        let exact = fixture.volume();
        let quality_json = output.report.quality.as_ref().map_or_else(|| "null".to_owned(), |quality| {
            let settings = policy.quality.expect("quality report requires a quality request");
            assert_eq!(quality.remaining_bad_triangles, above_radius_edge_2_5,
                "native quality report must match independent emitted-triangle measurements");
            assert!((quality.max_radius_edge_ratio-max_radius_edge_ratio).abs() <= 1e-10*max_radius_edge_ratio.max(1.0),
                "reported maximum ratio must match independent emitted-triangle measurement");
            assert!(quality.planar_steiner_points <= settings.max_steiner_points,
                "generated planar points must respect the shared point budget");
            assert!(quality.candidate_insertions <= settings.max_candidate_insertions,
                "attempted insertions must respect the shared candidate budget");
            assert_eq!(quality.within_target(), above_radius_edge_2_5 == 0,
                "target achievement must match the final independent violation count");
            format!("{{\"target_radius_edge_ratio\":{},\"max_radius_edge_ratio\":{},\"remaining_bad_triangles\":{},\"planar_steiner_points\":{},\"declined_insertions\":{},\"candidate_insertions\":{},\"budget_exhausted\":{},\"within_target\":{}}}",
                quality.target_radius_edge_ratio, quality.max_radius_edge_ratio, quality.remaining_bad_triangles,
                quality.planar_steiner_points, quality.declined_insertions, quality.candidate_insertions,
                quality.budget_exhausted, quality.within_target())
        });
        println!(
            "{label} {mode} {name}: construct_us={construction:.3} realize_us={realization:.3} vertices={vertices} triangles={count} min/p5_angle={min_angle:.3}/{p5:.3} aspect_median/p95={median_aspect:.3}/{p95:.3} radius_edge_above_2_5={above_radius_edge_2_5} deviation={deviation:.9} chord_midpoint={chord_error:.9} volume={volume:.9}/{exact:.9} bad_edges={bad_edges} membership={failures}/{probes}"
        );
        writeln!(csv,"{label},{mode},{name},{construction:.6},{realization:.6},{vertices},{count},{faces},{min_angle:.9},{p5:.9},{median_aspect:.9},{p95:.9},{max_radius_edge_ratio:.9},{above_radius_edge_2_5},{deviation:.12},{chord_error:.12},{exact:.12},{volume:.12},{bad_edges},{failures},{probes}").unwrap();
        if i > 0 {
            json.push(',');
        }
        write!(json,"{{\"fixture\":\"{name}\",\"construction_us\":{construction},\"realization_us\":{realization},\"vertices\":{vertices},\"triangles\":{count},\"native_faces\":{faces},\"min_angle_deg\":{min_angle},\"p5_angle_deg\":{p5},\"median_aspect\":{median_aspect},\"p95_aspect\":{p95},\"max_radius_edge_ratio\":{max_radius_edge_ratio},\"triangles_above_radius_edge_2_5\":{above_radius_edge_2_5},\"reported_deviation\":{deviation},\"chord_midpoint_error\":{chord_error},\"exact_volume\":{exact},\"mesh_volume\":{volume},\"bad_edges\":{bad_edges},\"membership_failures\":{failures},\"membership_probes\":{probes},\"quality\":{quality_json},\"native_face_triangles\":{{").unwrap();
        for (j, (face, count)) in provenance.iter().enumerate() {
            if j > 0 {
                json.push(',');
            }
            write!(json, "\"{face}\":{count}").unwrap();
        }
        json.push_str("}}");
        let stem = format!("{label}-{mode}-{name}");
        std::fs::write(
            directory.join(format!("{stem}-triangles.csv")),
            triangle_csv,
        )?;
        std::fs::write(
            directory.join(format!("{stem}.svg")),
            wireframe(&triangles, fixture, &format!("{label} / {mode} / {name}")),
        )?;
    }
    json.push_str("]}\n");
    std::fs::write(directory.join(format!("{label}-{mode}.csv")), csv)?;
    std::fs::write(directory.join(format!("{label}-{mode}.json")), json)?;
    Ok(())
}

fn wireframe(triangles: &[Triangle], fixture: Fixture, title: &str) -> String {
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1440" height="960" viewBox="0 0 1440 960"><rect width="1440" height="960" fill="#f7f9fb"/><style>text{font:16px system-ui,sans-serif;fill:#183044}.small{font-size:13px}</style>"##,
    );
    writeln!(svg,r#"<text x="24" y="30">{title} · actual emitted triangles</text><text x="24" y="53" class="small">Native Body realization; orthographic views. No welding or mesh repair. QA and drawing excluded from timing.</text>"#).unwrap();
    let center = fixture.centers().first().copied().unwrap_or([0.0; 2]);
    for panel in 0..4 {
        let left = 24.0 + f64::from(panel % 2) * 710.0;
        let top = 90.0 + f64::from(panel / 2) * 425.0;
        let label = match panel {
            0 => "Overall 3D",
            1 => "Top cap",
            2 => "Cap detail",
            _ => "Cylindrical wall only (isolated)",
        };
        writeln!(svg,r##"<rect x="{left}" y="{top}" width="690" height="400" rx="5" fill="white" stroke="#d8e0e6"/><text x="{}" y="{}">{label}</text><clipPath id="clip{panel}"><rect x="{}" y="{}" width="674" height="355"/></clipPath><g clip-path="url(#clip{panel})">"##,left+12.0,top+23.0,left+8.0,top+36.0).unwrap();
        let project = |p: [f64; 3]| -> [f64; 2] {
            match panel {
                0 => [p[0] - 0.8 * p[1], -0.25 * p[0] - 0.4 * p[1] - 2.0 * p[2]],
                1 => [p[0], -p[1]],
                2 => [p[0] - center[0], -(p[1] - center[1])],
                _ => [
                    p[0] - center[0] - 0.8 * (p[1] - center[1]),
                    -0.35 * (p[0] - center[0]) - 0.35 * (p[1] - center[1]) - p[2],
                ],
            }
        };
        let selected: Vec<_> = triangles
            .iter()
            .filter(|t| {
                if panel == 1 || panel == 2 {
                    t.iter().all(|p| (p[2] - fixture.height()).abs() < 1e-4)
                } else if panel == 3 {
                    t.iter().all(|p| {
                        ((p[0] - center[0]).hypot(p[1] - center[1]) - fixture.radius()).abs() < 1e-4
                    }) && (t[0][2] - t[1][2]).abs().max((t[0][2] - t[2][2]).abs()) > 1e-5
                } else {
                    true
                }
            })
            .collect();
        let bounds = if panel == 2 {
            let extent = fixture.radius() * 2.1;
            [[-extent, -extent], [extent, extent]]
        } else {
            let mut b = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
            for t in &selected {
                for p in **t {
                    let p = project(p);
                    for (k, value) in p.into_iter().enumerate() {
                        b[0][k] = b[0][k].min(value);
                        b[1][k] = b[1][k].max(value);
                    }
                }
            }
            b
        };
        if !selected.is_empty() {
            let scale =
                (640.0 / (bounds[1][0] - bounds[0][0])).min(320.0 / (bounds[1][1] - bounds[0][1]));
            let map = |p: [f64; 3]| {
                let p = project(p);
                [
                    left + 345.0 + (p[0] - (bounds[0][0] + bounds[1][0]) * 0.5) * scale,
                    top + 218.0 + (p[1] - (bounds[0][1] + bounds[1][1]) * 0.5) * scale,
                ]
            };
            for triangle in selected {
                let p = triangle.map(map);
                writeln!(svg,r##"<path d="M{:.3},{:.3}L{:.3},{:.3}L{:.3},{:.3}Z" fill="#b8d2df" fill-opacity="0.14" stroke="#284e63" stroke-opacity="0.65" stroke-width="0.55"/>"##,p[0][0],p[0][1],p[1][0],p[1][1],p[2][0],p[2][1]).unwrap();
            }
        } else {
            writeln!(
                svg,
                r#"<text x="{}" y="{}" class="small">No cylindrical wall in this fixture.</text>"#,
                left + 24.0,
                top + 70.0
            )
            .unwrap();
        }
        svg.push_str("</g>");
    }
    svg.push_str("</svg>\n");
    svg
}
