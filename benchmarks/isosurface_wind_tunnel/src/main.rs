// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic adaptive-isosurface quality and reduction wind tunnel.

#![expect(
    clippy::missing_assert_message,
    reason = "wind-tunnel gates expose the failed predicate or compared values"
)]

mod fixture;
mod measure;
mod quality;
mod report;
mod uniform;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{collections::BTreeMap, collections::BTreeSet};

use exedra::{Mesh, VertexId, attr};
use exedra_isosurface::{
    DualContourStats, ScalarField, SemiAnalyticContourResult, SemiAnalyticContourStats,
    dual_contour_semi_analytic,
};

use crate::fixture::{BOX_A_REGION, BOX_B_REGION, CYLINDER_REGION};
use crate::measure::{DyadicMeasurements, Measured, WorkMeasurements, reconstruct_dyadic};
use crate::quality::QualityReport;
use crate::report::TopologyReport;

const H1_PRIVATE_SIGNATURE: u64 = 0xf9f3_2216_4cf5_214a;
const H1_PRIVATE_STATS: DualContourStats = DualContourStats {
    octree_cells: 100_937,
    active_cells: 30_122,
    vertices: 30_122,
    faces: 60_240,
};
const H1_PRIVATE_FINAL_DEPTHS: [usize; 8] = [0, 0, 0, 199, 1_388, 5_277, 21_744, 59_712];
const H1_PRIVATE_CONTRIBUTING_DEPTHS: [usize; 8] = [0, 0, 0, 0, 0, 0, 0, 30_122];
const H1_PRIVATE_REGIONS: [(u32, usize); 2] = [(BOX_A_REGION, 38_884), (BOX_B_REGION, 21_356)];
const H1_ADAPTIVE_SIGNATURE: u64 = 0x8528_ca68_8b8f_b2c2;
const H1_ADAPTIVE_STATS: DualContourStats = DualContourStats {
    octree_cells: 4_089,
    active_cells: 939,
    vertices: 939,
    faces: 1_874,
};
const H1_ADAPTIVE_FINAL_DEPTHS: [usize; 8] = [0, 0, 1, 336, 1_184, 1_189, 708, 160];
const H1_ADAPTIVE_CONTRIBUTING_DEPTHS: [usize; 8] = [0, 0, 0, 38, 232, 299, 288, 82];
const H1_ADAPTIVE_REGIONS: [(u32, usize); 2] = [(BOX_A_REGION, 1_050), (BOX_B_REGION, 824)];
const REDUCTION_GATE_FACTOR: usize = 10;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Profile {
    Quick,
    Gate,
}

impl Profile {
    const fn depth(self) -> u8 {
        match self {
            Self::Quick => 5,
            Self::Gate => 7,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Gate => "gate",
        }
    }
}

struct AdaptiveRun {
    result: SemiAnalyticContourResult,
    work: WorkMeasurements,
    dyadic: DyadicMeasurements,
}

fn main() {
    let (profile, write_artifacts) = parse_args();
    let depth = profile.depth();
    let fixture = fixture::h1(depth);

    let uniform = uniform::extract(&fixture);
    let uniform_signature = report::extraction_signature(&uniform.mesh);
    let uniform_regions = report::region_histogram(&uniform.mesh);
    let uniform_topology = report::topology(&uniform.mesh);
    assert!(
        uniform_topology.is_closed_clean(),
        "uniform topology: {uniform_topology:?}"
    );

    let adaptive = run_adaptive(fixture::h1(depth));
    let repeated = run_adaptive(fixture::h1(depth));
    let adaptive_signature = report::extraction_signature(&adaptive.result.mesh);
    assert_eq!(
        adaptive_signature,
        report::extraction_signature(&repeated.result.mesh)
    );
    assert_eq!(adaptive.result.stats, repeated.result.stats);
    assert_eq!(adaptive.result.semi_analytic, repeated.result.semi_analytic);
    assert_eq!(adaptive.work, repeated.work);
    assert_eq!(adaptive.dyadic, repeated.dyadic);
    let adaptive_topology = report::topology(&adaptive.result.mesh);
    assert!(
        adaptive_topology.is_closed_clean(),
        "adaptive topology: {adaptive_topology:?}"
    );
    assert_counter_partition(&adaptive.result);
    assert_eq!(
        adaptive.dyadic.unique_interval_cells,
        adaptive.result.stats.octree_cells
    );
    assert_eq!(
        adaptive.work.projection_attempts,
        adaptive.result.stats.active_cells
    );

    let private_pin_matched = if profile == Profile::Gate {
        assert_uniform_private_pin(
            &uniform,
            uniform_signature,
            &uniform_regions,
            &uniform_topology,
        );
        assert_adaptive_pin(&adaptive, adaptive_signature);
        true
    } else {
        false
    };

    let quality = private_pin_matched.then(|| {
        let spacing = finest_spacing(fixture.params.root_bounds.extent(), depth);
        let cap = finest_cap(fixture.params.root_bounds.extent(), depth);
        let patches = fixture::visible_union_patches(fixture.boxes);
        let uniform_quality =
            quality::measure(&quality::triangles(&uniform.mesh), &patches, spacing, cap);
        let adaptive_quality = quality::measure(
            &quality::triangles(&adaptive.result.mesh),
            &patches,
            spacing,
            cap,
        );
        assert_quality(&uniform_quality);
        assert_quality(&adaptive_quality);
        (uniform_quality, adaptive_quality)
    });
    if profile == Profile::Gate {
        assert!(
            reduction_gate_passes(
                uniform.stats.vertices,
                uniform.stats.faces,
                adaptive.result.stats.vertices,
                adaptive.result.stats.faces,
            ),
            "gate requires at least {REDUCTION_GATE_FACTOR}x fewer adaptive vertices and faces"
        );
    }

    let hard = run_hard(depth.saturating_sub(1).max(4));
    let adaptive_timings = adaptive_timings(depth);
    let report = deterministic_report(
        profile,
        &fixture.params,
        private_pin_matched,
        &uniform,
        uniform_signature,
        &uniform_regions,
        &uniform_topology,
        &adaptive,
        adaptive_signature,
        &adaptive_topology,
        quality.as_ref(),
        &hard,
    );
    print!("{report}");
    println!("timing.adaptive_best_ms={:.3}", millis(adaptive_timings[0]));
    println!(
        "timing.adaptive_median_ms={:.3}",
        millis(adaptive_timings[1])
    );

    if write_artifacts {
        let directory = artifact_directory(profile);
        write_artifacts_to(
            &directory,
            &report,
            &uniform.mesh,
            &adaptive.result.mesh,
            &hard.result.mesh,
        );
        println!("artifacts={}", directory.display());
    }
}

fn parse_args() -> (Profile, bool) {
    let mut profile = None;
    let mut write_artifacts = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--quick" => set_profile(&mut profile, Profile::Quick),
            "--gate" => set_profile(&mut profile, Profile::Gate),
            "--write-artifacts" => write_artifacts = true,
            "--help" | "-h" => {
                println!("Usage: isosurface_wind_tunnel (--quick|--gate) [--write-artifacts]");
                std::process::exit(0);
            }
            _ => panic!("unknown argument: {argument}"),
        }
    }
    (profile.unwrap_or(Profile::Quick), write_artifacts)
}

fn set_profile(slot: &mut Option<Profile>, value: Profile) {
    assert!(slot.replace(value).is_none(), "select exactly one profile");
}

fn run_adaptive(fixture: fixture::H1Fixture) -> AdaptiveRun {
    let root = fixture.params.root_bounds;
    let depth = fixture.params.max_depth;
    let measured = Measured::new(fixture.field);
    let result = dual_contour_semi_analytic(&measured, &fixture.params)
        .expect("public adaptive H1 extraction");
    let work = measured.snapshot();
    let dyadic = reconstruct_dyadic(&work, root, depth);
    AdaptiveRun {
        result,
        work,
        dyadic,
    }
}

fn adaptive_timings(depth: u8) -> [Duration; 2] {
    let mut samples = (0..3)
        .map(|_| {
            let fixture = fixture::h1(depth);
            let started = Instant::now();
            let result = dual_contour_semi_analytic(&fixture.field, &fixture.params)
                .expect("unmeasured adaptive timing extraction");
            std::hint::black_box(result);
            started.elapsed()
        })
        .collect::<Vec<_>>();
    samples.sort_unstable();
    [samples[0], samples[1]]
}

struct HardRun {
    result: SemiAnalyticContourResult,
    signature: u64,
    topology: TopologyReport,
    regions: Vec<(u32, usize)>,
    max_output_implicit_residual: f32,
    seam_candidates: usize,
    max_seam_joint_implicit_residual: f32,
    projection_attempts: usize,
    projection_unobserved_cells: usize,
}

fn run_hard(depth: u8) -> HardRun {
    let fixture = fixture::hard(depth);
    let measured = Measured::new(fixture.field);
    let result = dual_contour_semi_analytic(&measured, &fixture.params)
        .expect("box-cylinder adaptive extraction");
    let work = measured.snapshot();
    let repeated = dual_contour_semi_analytic(&fixture.field, &fixture.params)
        .expect("repeated box-cylinder extraction");
    let signature = report::extraction_signature(&result.mesh);
    assert_eq!(signature, report::extraction_signature(&repeated.mesh));
    assert_eq!(result.stats, repeated.stats);
    assert_eq!(result.semi_analytic, repeated.semi_analytic);
    assert_counter_partition(&result);
    let topology = report::topology(&result.mesh);
    assert!(
        topology.is_closed_clean(),
        "box-cylinder topology: {topology:?}"
    );
    let regions = report::region_histogram(&result.mesh);
    assert!(
        regions
            .iter()
            .all(|&(region, _)| { region == BOX_A_REGION || region == CYLINDER_REGION })
    );
    let points = result
        .mesh
        .vertices()
        .map(|vertex| {
            *result
                .mesh
                .vertex_position(vertex)
                .expect("vertex position")
        })
        .collect::<Vec<_>>();
    let mut values = vec![0.0; points.len()];
    fixture.field.eval_points(&points, &mut values);
    let max_output_implicit_residual = values.into_iter().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(max_output_implicit_residual.is_finite());
    let seam_vertices = seam_vertices(&result.mesh, BOX_A_REGION, CYLINDER_REGION);
    let max_seam_joint_implicit_residual = seam_vertices
        .iter()
        .map(|&vertex| {
            let point = *result.mesh.vertex_position(vertex).expect("seam vertex");
            box_implicit_residual(point, fixture.box_center, fixture.box_half_extents).max(
                cylinder_implicit_residual(
                    point,
                    fixture.cylinder_center,
                    fixture.cylinder_axis,
                    fixture.cylinder_radius,
                    fixture.cylinder_half_height,
                ),
            )
        })
        .fold(0.0_f32, f32::max);
    assert!(max_seam_joint_implicit_residual.is_finite());
    let unique_projection_cells = work
        .projection_cells
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let unique_interval_cells = work.interval_cells.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(work.projection_attempts, work.projection_cells.len());
    assert_eq!(unique_projection_cells.len(), work.projection_cells.len());
    assert_eq!(unique_interval_cells.len(), work.interval_cells.len());
    assert!(unique_projection_cells.is_subset(&unique_interval_cells));
    assert!(unique_projection_cells.len() <= result.stats.active_cells);
    let projection_unobserved_cells = result.stats.active_cells - unique_projection_cells.len();
    HardRun {
        result,
        signature,
        topology,
        regions,
        max_output_implicit_residual,
        seam_candidates: seam_vertices.len(),
        max_seam_joint_implicit_residual,
        projection_attempts: work.projection_attempts,
        projection_unobserved_cells,
    }
}

fn seam_vertices(mesh: &Mesh, first_region: u32, second_region: u32) -> BTreeSet<VertexId> {
    let regions = mesh.attrs().dense(attr::FACE_REGION).expect("FACE_REGION");
    let mut masks = BTreeMap::<VertexId, u8>::new();
    for face in mesh.faces() {
        let region = regions.get(face.as_id()).copied().unwrap_or(u32::MAX);
        let bit = if region == first_region {
            1
        } else if region == second_region {
            2
        } else {
            0
        };
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).expect("face-loop vertex");
            *masks.entry(vertex).or_insert(0) |= bit;
        }
    }
    masks
        .into_iter()
        .filter_map(|(vertex, mask)| (mask == 3).then_some(vertex))
        .collect()
}

fn box_implicit_residual(point: [f32; 3], center: [f32; 3], half_extents: [f32; 3]) -> f32 {
    let q = core::array::from_fn::<_, 3, _>(|axis| {
        (point[axis] - center[axis]).abs() - half_extents[axis]
    });
    let outside = q.map(|value| value.max(0.0));
    let outside_length = outside
        .into_iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    (outside_length + q.into_iter().fold(f32::NEG_INFINITY, f32::max).min(0.0)).abs()
}

fn cylinder_implicit_residual(
    point: [f32; 3],
    center: [f32; 3],
    axis: [f32; 3],
    radius: f32,
    half_height: f32,
) -> f32 {
    let delta = core::array::from_fn::<_, 3, _>(|component| point[component] - center[component]);
    let axis_length = axis
        .into_iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    let axis = axis.map(|value| value / axis_length);
    let axial_position = dot3(delta, axis);
    let radial = core::array::from_fn::<_, 3, _>(|component| {
        delta[component] - axial_position * axis[component]
    });
    let radial_distance = radial
        .into_iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    let q = [radial_distance - radius, axial_position.abs() - half_height];
    let outside = q.map(|value| value.max(0.0));
    let outside_length = (outside[0] * outside[0] + outside[1] * outside[1]).sqrt();
    (outside_length + q[0].max(q[1]).min(0.0)).abs()
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn assert_uniform_private_pin(
    uniform: &uniform::UniformResult,
    signature: u64,
    regions: &[(u32, usize)],
    topology: &TopologyReport,
) {
    assert_eq!(uniform.stats, H1_PRIVATE_STATS);
    assert_eq!(
        uniform.semi_analytic,
        SemiAnalyticContourStats {
            unsupported_fallbacks: H1_PRIVATE_STATS.active_cells,
            ..SemiAnalyticContourStats::default()
        }
    );
    assert_eq!(uniform.final_leaf_depths, H1_PRIVATE_FINAL_DEPTHS);
    assert_eq!(uniform.contributing_depths, H1_PRIVATE_CONTRIBUTING_DEPTHS);
    assert_eq!(regions, H1_PRIVATE_REGIONS);
    assert!(topology.is_closed_clean());
    assert_eq!(signature, H1_PRIVATE_SIGNATURE);
}

fn assert_adaptive_pin(adaptive: &AdaptiveRun, signature: u64) {
    assert_eq!(adaptive.result.stats, H1_ADAPTIVE_STATS);
    assert_eq!(signature, H1_ADAPTIVE_SIGNATURE);
    assert_eq!(adaptive.dyadic.final_leaf_depths, H1_ADAPTIVE_FINAL_DEPTHS);
    assert_eq!(
        adaptive.dyadic.contributing_depths,
        H1_ADAPTIVE_CONTRIBUTING_DEPTHS
    );
    assert_eq!(
        report::region_histogram(&adaptive.result.mesh),
        H1_ADAPTIVE_REGIONS
    );
    assert_eq!(
        adaptive.result.semi_analytic,
        SemiAnalyticContourStats {
            unsupported_fallbacks: H1_ADAPTIVE_STATS.active_cells,
            ..SemiAnalyticContourStats::default()
        }
    );
}

fn assert_counter_partition(result: &SemiAnalyticContourResult) {
    let counters = result.semi_analytic;
    let partition = counters.surface_projections
        + counters.feature_snaps
        + counters.unsupported_fallbacks
        + counters.ambiguous_fallbacks
        + counters.tangent_fallbacks
        + counters.coincident_fallbacks
        + counters.over_budget_fallbacks
        + counters.invalid_fallbacks;
    assert_eq!(partition, result.stats.active_cells);
}

fn finest_spacing(extent: [f32; 3], depth: u8) -> f64 {
    let resolution = f64::from(1_u32 << depth);
    0.5 * extent
        .into_iter()
        .map(f64::from)
        .fold(f64::INFINITY, f64::min)
        / resolution
}

fn finest_cap(extent: [f32; 3], depth: u8) -> f64 {
    let resolution = f64::from(1_u32 << depth);
    let diagonal = extent
        .into_iter()
        .map(|value| {
            let edge = f64::from(value) / resolution;
            edge * edge
        })
        .sum::<f64>()
        .sqrt();
    0.5 * diagonal
}

fn assert_quality(quality: &QualityReport) {
    assert!(quality.mesh_to_analytic.maximum.is_finite());
    assert!(quality.analytic_to_mesh.maximum.is_finite());
    assert!(quality.mesh_to_analytic.maximum <= quality.cap);
    assert!(quality.analytic_to_mesh.maximum <= quality.cap);
}

fn reduction_gate_passes(
    uniform_vertices: usize,
    uniform_faces: usize,
    adaptive_vertices: usize,
    adaptive_faces: usize,
) -> bool {
    adaptive_vertices > 0
        && adaptive_faces > 0
        && adaptive_vertices
            .checked_mul(REDUCTION_GATE_FACTOR)
            .is_some_and(|required| uniform_vertices >= required)
        && adaptive_faces
            .checked_mul(REDUCTION_GATE_FACTOR)
            .is_some_and(|required| uniform_faces >= required)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one deterministic report owns all gate witnesses"
)]
fn deterministic_report(
    profile: Profile,
    params: &exedra_isosurface::DualContourParams,
    private_pin_matched: bool,
    uniform: &uniform::UniformResult,
    uniform_signature: u64,
    uniform_regions: &[(u32, usize)],
    uniform_topology: &TopologyReport,
    adaptive: &AdaptiveRun,
    adaptive_signature: u64,
    adaptive_topology: &TopologyReport,
    quality: Option<&(QualityReport, QualityReport)>,
    hard: &HardRun,
) -> String {
    let mut output = String::new();
    writeln!(output, "schema=isosurface-wind-tunnel-v1").expect("String write");
    writeln!(output, "profile={}", profile.name()).expect("String write");
    writeln!(output, "depth={}", profile.depth()).expect("String write");
    writeln!(output, "parameters={params:?}").expect("String write");
    writeln!(output, "uniform.private_pin_matched={private_pin_matched}").expect("String write");
    write_mesh_report(
        &mut output,
        "uniform",
        &uniform.stats,
        uniform_signature,
        uniform_regions,
        uniform_topology,
    );
    writeln!(
        output,
        "uniform.final_leaf_depths={:?}",
        uniform.final_leaf_depths
    )
    .expect("String write");
    writeln!(
        output,
        "uniform.contributing_depths={:?}",
        uniform.contributing_depths
    )
    .expect("String write");
    writeln!(output, "uniform.semi_analytic={:?}", uniform.semi_analytic).expect("String write");
    writeln!(output, "uniform.work={:?}", uniform.work).expect("String write");
    write_mesh_report(
        &mut output,
        "adaptive",
        &adaptive.result.stats,
        adaptive_signature,
        &report::region_histogram(&adaptive.result.mesh),
        adaptive_topology,
    );
    writeln!(
        output,
        "adaptive.final_leaf_depths={:?}",
        adaptive.dyadic.final_leaf_depths
    )
    .expect("String write");
    writeln!(
        output,
        "adaptive.contributing_depths={:?}",
        adaptive.dyadic.contributing_depths
    )
    .expect("String write");
    writeln!(
        output,
        "adaptive.semi_analytic={:?}",
        adaptive.result.semi_analytic
    )
    .expect("String write");
    writeln!(
        output,
        "adaptive.interval_calls={}",
        adaptive.work.interval_calls
    )
    .expect("String write");
    writeln!(
        output,
        "adaptive.interval_elements={}",
        adaptive.work.interval_elements
    )
    .expect("String write");
    writeln!(output, "adaptive.point_calls={}", adaptive.work.point_calls).expect("String write");
    writeln!(
        output,
        "adaptive.point_elements={}",
        adaptive.work.point_elements
    )
    .expect("String write");
    writeln!(
        output,
        "adaptive.gradient_calls={}",
        adaptive.work.gradient_calls
    )
    .expect("String write");
    writeln!(
        output,
        "adaptive.gradient_elements={}",
        adaptive.work.gradient_elements
    )
    .expect("String write");
    writeln!(
        output,
        "adaptive.projection_attempts={}",
        adaptive.work.projection_attempts
    )
    .expect("String write");
    if let Some((uniform_quality, adaptive_quality)) = quality {
        write_quality(&mut output, "uniform", uniform_quality);
        write_quality(&mut output, "adaptive", adaptive_quality);
        let vertex_ratio = uniform.stats.vertices as f64 / adaptive.result.stats.vertices as f64;
        let face_ratio = uniform.stats.faces as f64 / adaptive.result.stats.faces as f64;
        let reduction_gate_passed = reduction_gate_passes(
            uniform.stats.vertices,
            uniform.stats.faces,
            adaptive.result.stats.vertices,
            adaptive.result.stats.faces,
        );
        writeln!(output, "reduction.vertex_ratio={vertex_ratio:.9}").expect("String write");
        writeln!(output, "reduction.face_ratio={face_ratio:.9}").expect("String write");
        writeln!(output, "reduction.gate_passed={}", reduction_gate_passed).expect("String write");
    } else {
        writeln!(output, "quality.status=not_run_without_private_uniform_pin")
            .expect("String write");
        writeln!(
            output,
            "reduction.status=not_reported_without_private_uniform_pin"
        )
        .expect("String write");
    }
    write_mesh_report(
        &mut output,
        "hard",
        &hard.result.stats,
        hard.signature,
        &hard.regions,
        &hard.topology,
    );
    writeln!(output, "hard.semi_analytic={:?}", hard.result.semi_analytic).expect("String write");
    writeln!(
        output,
        "hard.max_output_implicit_residual={:.9e}",
        hard.max_output_implicit_residual
    )
    .expect("String write");
    writeln!(output, "hard.seam_candidates={}", hard.seam_candidates).expect("String write");
    writeln!(
        output,
        "hard.max_seam_joint_implicit_residual={:.9e}",
        hard.max_seam_joint_implicit_residual
    )
    .expect("String write");
    writeln!(
        output,
        "hard.projection_attempts={}",
        hard.projection_attempts
    )
    .expect("String write");
    writeln!(
        output,
        "hard.projection_unobserved_cells={}",
        hard.projection_unobserved_cells
    )
    .expect("String write");
    output
}

fn write_mesh_report(
    output: &mut String,
    prefix: &str,
    stats: &DualContourStats,
    signature: u64,
    regions: &[(u32, usize)],
    topology: &TopologyReport,
) {
    writeln!(output, "{prefix}.signature={signature:016x}").expect("String write");
    writeln!(output, "{prefix}.stats={stats:?}").expect("String write");
    writeln!(output, "{prefix}.regions={regions:?}").expect("String write");
    writeln!(output, "{prefix}.topology={topology:?}").expect("String write");
}

fn write_quality(output: &mut String, prefix: &str, quality: &QualityReport) {
    writeln!(output, "{prefix}.quality.spacing={:.9e}", quality.spacing).expect("String write");
    writeln!(output, "{prefix}.quality.cap={:.9e}", quality.cap).expect("String write");
    writeln!(
        output,
        "{prefix}.quality.mesh_to_analytic.samples={}",
        quality.mesh_to_analytic.samples
    )
    .expect("String write");
    writeln!(
        output,
        "{prefix}.quality.mesh_to_analytic.max={:.9e}",
        quality.mesh_to_analytic.maximum
    )
    .expect("String write");
    writeln!(
        output,
        "{prefix}.quality.analytic_to_mesh.samples={}",
        quality.analytic_to_mesh.samples
    )
    .expect("String write");
    writeln!(
        output,
        "{prefix}.quality.analytic_to_mesh.max={:.9e}",
        quality.analytic_to_mesh.maximum
    )
    .expect("String write");
}

fn artifact_directory(profile: Profile) -> PathBuf {
    Path::new("target")
        .join("isosurface_wind_tunnel")
        .join(profile.name())
}

fn write_artifacts_to(
    directory: &Path,
    report_text: &str,
    uniform: &Mesh,
    adaptive: &Mesh,
    hard: &Mesh,
) {
    fs::create_dir_all(directory).expect("artifact directory");
    fs::write(directory.join("report.txt"), report_text).expect("report artifact");
    fs::write(
        directory.join("uniform.obj"),
        report::grouped_obj(uniform, "independent forced-uniform H1 witness"),
    )
    .expect("uniform OBJ");
    fs::write(
        directory.join("adaptive.obj"),
        report::grouped_obj(adaptive, "public adaptive H1 output"),
    )
    .expect("adaptive OBJ");
    fs::write(
        directory.join("hard-box-cylinder.obj"),
        report::grouped_obj(hard, "public adaptive box-cylinder output"),
    )
    .expect("hard OBJ");
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[cfg(test)]
mod tests {
    use exedra_isosurface::analytic::{BoxField, TaggedField};
    use exedra_isosurface::{DualContourParams, EdgeSearchParams, dual_contour_semi_analytic};
    use exedra_qef::QefParams;
    use exedra_spatial::Aabb;

    use crate::fixture::BOX_A_REGION;
    use crate::{assert_counter_partition, reduction_gate_passes, report};

    #[test]
    fn reduction_gate_requires_tenfold_vertices_and_faces() {
        assert!(reduction_gate_passes(100, 200, 10, 20));
        assert!(!reduction_gate_passes(99, 200, 10, 20));
        assert!(!reduction_gate_passes(100, 199, 10, 20));
        assert!(!reduction_gate_passes(usize::MAX, 200, usize::MAX, 20));
        assert!(!reduction_gate_passes(100, 200, 0, 20));
        assert!(!reduction_gate_passes(100, 200, 10, 0));
    }

    #[test]
    fn tagged_single_box_exercises_successful_surface_projection() {
        let field = TaggedField {
            field: BoxField {
                center: [0.137, -0.219, 0.083],
                half_extents: [0.713, 0.581, 0.467],
            },
            provenance: BOX_A_REGION,
        };
        let params = DualContourParams {
            root_bounds: Aabb::new([-1.3; 3], [1.4; 3]).expect("root"),
            max_depth: 4,
            cell_budget: None,
            edge_search: EdgeSearchParams {
                bisection_steps: 10,
            },
            qef: QefParams::default(),
        };
        let result = dual_contour_semi_analytic(&field, &params).expect("single box extraction");
        assert_counter_partition(&result);
        assert_eq!(
            result.semi_analytic.surface_projections,
            result.stats.active_cells
        );
        assert_eq!(result.semi_analytic.feature_snaps, 0);
        assert!(report::topology(&result.mesh).is_closed_clean());
    }
}
