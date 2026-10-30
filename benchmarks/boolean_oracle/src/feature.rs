// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Independent fixed scenarios for sharp, region-tagged field meshing.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use exedra_implicit::{
    Aabb, Cuboid, Cylinder, Difference, Formula, Intersection, Scale, Tagged, Translate, Union,
};
use exedra_isosurface::{DelaunayParams, DelaunayResult, delaunay_mesh};
use exedra_math::{add, dot, scale, sub};

const BOX_REGION: u32 = 10;
const CYLINDER_REGION: u32 = 20;
/// The scales the suite runs at.
pub(crate) const SCALES: [f64; 3] = [1.0e-3, 1.0, 1.0e4];
/// Cells across the meshed bounds, which are three times the scale.
const CELLS: f64 = 32.0;

/// Stable measurements from one fixed scenario.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FeatureReport {
    pub(crate) name: &'static str,
    pub(crate) scale: f64,
    pub(crate) signature: u64,
    pub(crate) complete: bool,
    pub(crate) topology_issues: usize,
    pub(crate) crease_lines: usize,
    pub(crate) corners: usize,
    pub(crate) box_faces: usize,
    pub(crate) cylinder_faces: usize,
    pub(crate) unexpected_faces: usize,
    pub(crate) seam_vertices: usize,
    pub(crate) max_seam_residual: f64,
}

#[derive(Copy, Clone)]
struct Geometry {
    box_center: [f64; 3],
    box_half_extents: [f64; 3],
    cylinder_center: [f64; 3],
    cylinder_axis: [f64; 3],
    cylinder_radius: f64,
    cylinder_half_height: f64,
}

/// Runs the fixed suite, the CSG operators at each of `scales`.
#[must_use]
pub(crate) fn run_suite(scales: &[f64]) -> Vec<FeatureReport> {
    let mut reports = Vec::new();
    for &scale in scales {
        reports.push(run_difference(scale).0);
        reports.push(run_intersection(scale));
        reports.push(run_union(scale));
    }
    reports.push(run_translated_difference());
    reports.push(run_scaled_difference());
    reports.push(run_rotated_difference());
    reports
}

/// Panics with scenario-specific diagnostics when the fixed suite violates
/// its completeness, topology, attribution or feature contracts.
pub(crate) fn assert_suite(reports: &[FeatureReport]) {
    for report in reports {
        let (name, scale) = (report.name, report.scale);
        assert!(report.complete, "{name} scale {scale} did not finish");
        assert_eq!(
            report.topology_issues, 0,
            "{name} scale {scale} has topology findings"
        );
        assert!(
            report.box_faces > 0 && report.cylinder_faces > 0,
            "{name} scale {scale} did not attribute both primitives: {report:?}"
        );
        assert_eq!(
            report.unexpected_faces, 0,
            "{name} scale {scale} emitted unexpected primitive regions"
        );
        assert!(
            report.crease_lines > 0 && report.seam_vertices > 0,
            "{name} scale {scale} found no seam: {report:?}"
        );
        assert!(
            report.max_seam_residual <= feature_tolerance(scale),
            "{name} scale {scale} seam residual {} exceeded {}",
            report.max_seam_residual,
            feature_tolerance(scale)
        );
    }
}

pub(crate) fn print_suite(reports: &[FeatureReport]) {
    for report in reports {
        let prefix = format!(
            "feature.{}.scale_{:016x}",
            report.name,
            report.scale.to_bits()
        );
        println!("{prefix}.signature={:016x}", report.signature);
        println!("{prefix}.complete={}", report.complete);
        println!("{prefix}.topology_issues={}", report.topology_issues);
        println!("{prefix}.crease_lines={}", report.crease_lines);
        println!("{prefix}.corners={}", report.corners);
        println!("{prefix}.box_faces={}", report.box_faces);
        println!("{prefix}.cylinder_faces={}", report.cylinder_faces);
        println!("{prefix}.unexpected_faces={}", report.unexpected_faces);
        println!("{prefix}.seam_vertices={}", report.seam_vertices);
        println!("{prefix}.max_seam_residual={}", report.max_seam_residual);
    }
}

/// Writes the unit-scale through-cut reference mesh below
/// `target/boolean_oracle`, grouped by primitive face region.
pub(crate) fn write_reference_obj() -> std::io::Result<PathBuf> {
    let (_, result) = run_difference(1.0);
    let directory = PathBuf::from("target/boolean_oracle");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("box_cylinder.obj");
    std::fs::write(&path, mesh_to_region_obj(&result))?;
    Ok(path)
}

fn run_difference(scale: f64) -> (FeatureReport, DelaunayResult) {
    let geometry = through_cut_geometry(scale, [0.0, 0.0, 1.0]);
    let field = Difference::new(box_leaf(geometry), cylinder_leaf(geometry));
    run(
        "difference",
        scale,
        geometry,
        centered_bounds(scale),
        &field,
    )
}

fn run_intersection(scale: f64) -> FeatureReport {
    let geometry = Geometry {
        box_center: [0.0; 3],
        box_half_extents: [scale; 3],
        cylinder_center: [0.0; 3],
        cylinder_axis: [0.0, 0.0, 1.0],
        cylinder_radius: 0.72 * scale,
        cylinder_half_height: 1.35 * scale,
    };
    let field = Intersection::new(box_leaf(geometry), cylinder_leaf(geometry));
    run(
        "intersection",
        scale,
        geometry,
        centered_bounds(scale),
        &field,
    )
    .0
}

fn run_union(scale: f64) -> FeatureReport {
    let geometry = Geometry {
        box_center: [0.0; 3],
        box_half_extents: [0.8 * scale; 3],
        cylinder_center: [0.65 * scale, 0.0, 0.0],
        cylinder_axis: [0.0, 0.0, 1.0],
        cylinder_radius: 0.55 * scale,
        cylinder_half_height: 0.6 * scale,
    };
    let field = Union::new(box_leaf(geometry), cylinder_leaf(geometry));
    run("union", scale, geometry, centered_bounds(scale), &field).0
}

/// A bore along a diagonal, so no crease lies along an axis.
fn run_rotated_difference() -> FeatureReport {
    let geometry = through_cut_geometry(1.0, [1.0, 1.0, 0.0]);
    let field = Difference::new(box_leaf(geometry), cylinder_leaf(geometry));
    run(
        "rotated_difference",
        1.0,
        geometry,
        centered_bounds(1.0),
        &field,
    )
    .0
}

fn run_translated_difference() -> FeatureReport {
    let offset = [2.5, -1.25, 0.5];
    let local_geometry = through_cut_geometry(1.0, [0.0, 0.0, 1.0]);
    let field = Translate::new(
        Difference::new(box_leaf(local_geometry), cylinder_leaf(local_geometry)),
        offset,
    )
    .expect("finite offset");
    let geometry = translate_geometry(local_geometry, offset);
    let bounds =
        Aabb::new(add(offset, [-1.5; 3]), add(offset, [1.5; 3])).expect("translated bounds");
    run("translated_difference", 1.0, geometry, bounds, &field).0
}

fn run_scaled_difference() -> FeatureReport {
    let factor = 3.0;
    let local_geometry = through_cut_geometry(1.0, [0.0, 0.0, 1.0]);
    let field = Scale::new(
        Difference::new(box_leaf(local_geometry), cylinder_leaf(local_geometry)),
        factor,
    )
    .expect("positive scale");
    let geometry = through_cut_geometry(factor, [0.0, 0.0, 1.0]);
    run(
        "scaled_difference",
        factor,
        geometry,
        centered_bounds(factor),
        &field,
    )
    .0
}

fn run<F: Formula>(
    name: &'static str,
    scale: f64,
    geometry: Geometry,
    bounds: Aabb,
    field: &F,
) -> (FeatureReport, DelaunayResult) {
    let mut params = DelaunayParams::new(bounds, 3.0 * scale / CELLS);
    params.regions = true;
    let result = delaunay_mesh(field, &params)
        .unwrap_or_else(|error| panic!("{name} scale {scale} meshing failed: {error}"));
    let repeated = delaunay_mesh(field, &params)
        .unwrap_or_else(|error| panic!("{name} scale {scale} repeat failed: {error}"));
    let signature = signature(&result);
    assert_eq!(
        signature,
        self::signature(&repeated),
        "{name} scale {scale} mesh signature changed"
    );
    assert_eq!(
        result.report, repeated.report,
        "{name} scale {scale} report changed"
    );

    let mut box_faces = 0;
    let mut cylinder_faces = 0;
    let mut unexpected_faces = 0;
    let mut incident_regions = vec![0_u8; result.positions.len()];
    for (triangle, &region) in result.triangles.iter().zip(&result.regions) {
        let mask = match region {
            BOX_REGION => {
                box_faces += 1;
                0b01
            }
            CYLINDER_REGION => {
                cylinder_faces += 1;
                0b10
            }
            _ => {
                unexpected_faces += 1;
                0
            }
        };
        for &vertex in triangle {
            incident_regions[vertex as usize] |= mask;
        }
    }

    // Every vertex where both primitives' faces meet must lie on both
    // surfaces: creases are sampled on the surface and kept sharp.
    let seam: Vec<f64> = incident_regions
        .iter()
        .zip(&result.positions)
        .filter(|(mask, _)| **mask == 0b11)
        .map(|(_, &point)| joint_implicit_residual(point, geometry))
        .collect();

    let topology_issues = result
        .mesh()
        .map_or(usize::MAX, |mesh| mesh.validate_deep().len());
    let report = FeatureReport {
        name,
        scale,
        signature,
        complete: result.report.is_complete(),
        topology_issues,
        crease_lines: result.report.crease_lines,
        corners: result.report.corners,
        box_faces,
        cylinder_faces,
        unexpected_faces,
        seam_vertices: seam.len(),
        max_seam_residual: seam.iter().copied().fold(0.0, f64::max),
    };
    (report, result)
}

fn box_leaf(geometry: Geometry) -> Tagged<Cuboid> {
    Tagged::new(
        Cuboid::new(geometry.box_center, geometry.box_half_extents).expect("valid box"),
        BOX_REGION,
    )
}

fn cylinder_leaf(geometry: Geometry) -> Tagged<Cylinder> {
    Tagged::new(
        Cylinder::new(
            geometry.cylinder_center,
            geometry.cylinder_axis,
            geometry.cylinder_radius,
            geometry.cylinder_half_height,
        )
        .expect("valid cylinder"),
        CYLINDER_REGION,
    )
}

fn through_cut_geometry(scale: f64, axis: [f64; 3]) -> Geometry {
    Geometry {
        box_center: [0.0; 3],
        box_half_extents: [scale; 3],
        cylinder_center: [0.0; 3],
        cylinder_axis: axis,
        cylinder_radius: 0.6 * scale,
        cylinder_half_height: 1.5 * scale,
    }
}

fn translate_geometry(mut geometry: Geometry, offset: [f64; 3]) -> Geometry {
    geometry.box_center = add(geometry.box_center, offset);
    geometry.cylinder_center = add(geometry.cylinder_center, offset);
    geometry
}

fn centered_bounds(scale: f64) -> Aabb {
    Aabb::new([-1.5 * scale; 3], [1.5 * scale; 3]).expect("valid centered bounds")
}

fn feature_tolerance(scale: f64) -> f64 {
    1.0e-6 * scale
}

fn box_implicit_residual(point: [f64; 3], geometry: Geometry) -> f64 {
    let mut residual = f64::NEG_INFINITY;
    for (axis, coordinate) in point.into_iter().enumerate() {
        residual = residual
            .max((coordinate - geometry.box_center[axis]).abs() - geometry.box_half_extents[axis]);
    }
    residual.abs()
}

fn cylinder_residual(point: [f64; 3], geometry: Geometry) -> f64 {
    let axis = exedra_math::normalize(geometry.cylinder_axis).unwrap_or([f64::NAN; 3]);
    let delta = sub(point, geometry.cylinder_center);
    let axial = dot(delta, axis);
    let radial_vector = sub(delta, scale(axis, axial));
    let radial = dot(radial_vector, radial_vector).sqrt();
    let side = radial - geometry.cylinder_radius;
    let cap = axial.abs() - geometry.cylinder_half_height;
    let outside = (side.max(0.0).powi(2) + cap.max(0.0).powi(2)).sqrt();
    (outside + side.max(cap).min(0.0)).abs()
}

fn joint_implicit_residual(point: [f64; 3], geometry: Geometry) -> f64 {
    box_implicit_residual(point, geometry).max(cylinder_residual(point, geometry))
}

fn signature(result: &DelaunayResult) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for position in &result.positions {
        for component in position {
            hash = fnv_bytes(hash, &component.to_bits().to_le_bytes());
        }
    }
    for triangle in &result.triangles {
        for index in triangle {
            hash = fnv_bytes(hash, &index.to_le_bytes());
        }
    }
    for region in &result.regions {
        hash = fnv_bytes(hash, &region.to_le_bytes());
    }
    hash
}

fn fnv_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn mesh_to_region_obj(result: &DelaunayResult) -> String {
    let mut output = String::from(
        "# deterministic box/cylinder reference\n# face groups are primitive identities\n",
    );
    for position in &result.positions {
        writeln!(
            output,
            "v {:.9} {:.9} {:.9}",
            position[0], position[1], position[2]
        )
        .expect("writing to String cannot fail");
    }
    let mut groups = BTreeMap::<u32, Vec<[u32; 3]>>::new();
    for (triangle, &region) in result.triangles.iter().zip(&result.regions) {
        groups.entry(region).or_default().push(*triangle);
    }
    for (region, faces) in groups {
        writeln!(output, "g primitive_{region}").expect("writing to String cannot fail");
        for [a, b, c] in faces {
            writeln!(output, "f {} {} {}", a + 1, b + 1, c + 1)
                .expect("writing to String cannot fail");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        Geometry, assert_suite, joint_implicit_residual, mesh_to_region_obj, run_difference,
        run_suite, through_cut_geometry,
    };

    /// Each scenario meshes twice and checks the two agree. Other scales
    /// run in the release binary.
    #[test]
    fn fixed_feature_suite_is_clean_and_deterministic() {
        assert_suite(&run_suite(&[1.0]));
    }

    #[test]
    fn reference_obj_is_grouped_by_primitive() {
        let (_, result) = run_difference(1.0);
        let first = mesh_to_region_obj(&result);
        let groups = first
            .lines()
            .filter(|line| line.starts_with("g "))
            .collect::<Vec<_>>();
        assert_eq!(groups, ["g primitive_10", "g primitive_20"]);
        assert!(first.lines().any(|line| line.starts_with("v ")));
        assert!(first.lines().any(|line| line.starts_with("f ")));
    }

    #[test]
    fn joint_residual_requires_both_primitive_boundaries() {
        let through_cut = through_cut_geometry(1.0, [0.0, 0.0, 1.0]);
        assert!(
            joint_implicit_residual([0.0, 0.0, 1.0], through_cut) > 0.4,
            "box-face point inside cylinder is not a pair feature"
        );
        assert!(joint_implicit_residual([0.6, 0.0, 1.0], through_cut) <= f64::EPSILON);

        let cap_line = Geometry {
            cylinder_radius: 1.2,
            cylinder_half_height: 0.75,
            ..through_cut
        };
        assert!(joint_implicit_residual([1.0, 0.0, 0.75], cap_line) <= f64::EPSILON);
    }
}
