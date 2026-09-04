// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Independent fixed scenarios for opt-in semi-analytic field extraction.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use exedra_isosurface::analytic::{
    BoxField, CylinderField, Difference, Intersection, TaggedField, Union,
};
use exedra_isosurface::{
    DualContourParams, EdgeSearchParams, SemiAnalyticContourResult, SemiAnalyticField, Translate,
    UniformScale, dual_contour, dual_contour_semi_analytic,
};
use exedra_math::{add, dot, scale, sub};
use exedra_mesh::{ExtractParams, Mesh, VertexId, attr};
use exedra_qef::QefParams;
use exedra_spatial::Aabb;

const BOX_REGION: u32 = 10;
const CYLINDER_REGION: u32 = 20;
const SCALES: [f32; 3] = [1.0e-3, 1.0, 1.0e4];

type BoxLeaf = TaggedField<BoxField, u32>;
type CylinderLeaf = TaggedField<CylinderField, u32>;

/// Stable measurements from one fixed semi-analytic scenario.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FeatureReport {
    pub(crate) name: &'static str,
    pub(crate) scale: f32,
    pub(crate) depth: u8,
    pub(crate) signature: u64,
    pub(crate) topology_issues: usize,
    pub(crate) active_cells: usize,
    pub(crate) box_faces: usize,
    pub(crate) cylinder_faces: usize,
    pub(crate) unexpected_faces: usize,
    pub(crate) seam_candidates: usize,
    pub(crate) max_feature_implicit_residual: f32,
    pub(crate) surface_projections: usize,
    pub(crate) feature_snaps: usize,
    pub(crate) unsupported_fallbacks: usize,
    pub(crate) ambiguous_fallbacks: usize,
    pub(crate) tangent_fallbacks: usize,
    pub(crate) coincident_fallbacks: usize,
    pub(crate) over_budget_fallbacks: usize,
    pub(crate) invalid_fallbacks: usize,
}

#[derive(Copy, Clone)]
struct Geometry {
    box_center: [f32; 3],
    box_half_extents: [f32; 3],
    cylinder_center: [f32; 3],
    cylinder_axis: [f32; 3],
    cylinder_radius: f32,
    cylinder_half_height: f32,
}

/// Runs the fixed typed suite. These expression trees deliberately remain
/// generic: erasing them behind `dyn ScalarField` would erase the optional
/// semi-analytic capability under test.
#[must_use]
pub(crate) fn run_suite() -> Vec<FeatureReport> {
    let mut reports = Vec::new();
    for scale in SCALES {
        reports.push(run_difference(scale, 5).0);
        reports.push(run_intersection(scale, 5));
        reports.push(run_union(scale, 5));
    }
    reports.push(run_translated_difference(5));
    reports.push(run_uniform_scale_difference(5));
    reports.push(run_rotated_fallback(1.0, 5));
    reports
}

/// Panics with scenario-specific diagnostics when the fixed suite violates
/// its topology, attribution, feature, fallback, or determinism contracts.
pub(crate) fn assert_suite(reports: &[FeatureReport]) {
    for report in reports {
        assert_eq!(
            report.surface_projections
                + report.feature_snaps
                + report.unsupported_fallbacks
                + report.ambiguous_fallbacks
                + report.tangent_fallbacks
                + report.coincident_fallbacks
                + report.over_budget_fallbacks
                + report.invalid_fallbacks,
            report.active_cells,
            "{} scale {} did not classify every active cell exactly once",
            report.name,
            report.scale
        );
        assert_eq!(
            report.topology_issues, 0,
            "{} scale {} has topology findings",
            report.name, report.scale
        );
        assert!(
            report.box_faces > 0 && report.cylinder_faces > 0,
            "{} scale {} did not attribute both primitives: {report:?}",
            report.name,
            report.scale
        );
        assert_eq!(
            report.unexpected_faces, 0,
            "{} scale {} emitted unexpected primitive regions",
            report.name, report.scale
        );
        if report.name == "rotated_difference" {
            assert!(
                report.unsupported_fallbacks > 0,
                "rotated pair did not count unsupported QEF fallbacks: {report:?}"
            );
            assert_eq!(report.feature_snaps, 0, "rotated pair snapped features");
            assert_eq!(
                report.surface_projections, 0,
                "rotated pair must retain every QEF position"
            );
            assert_eq!(
                report.unsupported_fallbacks, report.active_cells,
                "rotated pair must classify every active cell as Unsupported"
            );
        } else {
            assert!(
                report.surface_projections > 0,
                "{} scale {} did not project primitive surfaces",
                report.name,
                report.scale
            );
            assert!(
                report.feature_snaps > 0,
                "{} scale {} did not snap a feature",
                report.name,
                report.scale
            );
            assert!(
                report.seam_candidates >= report.feature_snaps,
                "{} scale {} has fewer verified feature vertices than snaps: {report:?}",
                report.name,
                report.scale
            );
            assert_eq!(
                report.seam_candidates,
                report.feature_snaps + report.ambiguous_fallbacks,
                "{} scale {} seam candidates are not fully explained by snaps and ambiguous cells",
                report.name,
                report.scale
            );
            assert!(
                report.max_feature_implicit_residual.is_finite(),
                "{} scale {} selected a non-finite feature residual",
                report.name,
                report.scale
            );
            assert!(
                report.max_feature_implicit_residual <= feature_tolerance(report.scale),
                "{} scale {} residual {} exceeded {}",
                report.name,
                report.scale,
                report.max_feature_implicit_residual,
                feature_tolerance(report.scale)
            );
            let expected_ambiguous = usize::from(report.name == "union") * 4;
            assert_eq!(
                (
                    report.unsupported_fallbacks,
                    report.ambiguous_fallbacks,
                    report.tangent_fallbacks,
                    report.coincident_fallbacks,
                    report.over_budget_fallbacks,
                    report.invalid_fallbacks,
                ),
                (0, expected_ambiguous, 0, 0, 0, 0),
                "{} scale {} unexpectedly fell back: {report:?}",
                report.name,
                report.scale
            );
        }
    }
}

pub(crate) fn print_suite(reports: &[FeatureReport]) {
    for report in reports {
        let prefix = format!(
            "semi_analytic.{}.scale_{:08x}",
            report.name,
            report.scale.to_bits()
        );
        println!("{prefix}.signature={:016x}", report.signature);
        println!("{prefix}.depth={}", report.depth);
        println!("{prefix}.topology_issues={}", report.topology_issues);
        println!("{prefix}.box_faces={}", report.box_faces);
        println!("{prefix}.cylinder_faces={}", report.cylinder_faces);
        println!("{prefix}.unexpected_faces={}", report.unexpected_faces);
        println!("{prefix}.seam_candidates={}", report.seam_candidates);
        println!(
            "{prefix}.max_feature_implicit_residual={}",
            report.max_feature_implicit_residual
        );
        println!("{prefix}.feature_snaps={}", report.feature_snaps);
        println!(
            "{prefix}.unsupported_fallbacks={}",
            report.unsupported_fallbacks
        );
        println!(
            "{prefix}.ambiguous_fallbacks={}",
            report.ambiguous_fallbacks
        );
        println!("{prefix}.tangent_fallbacks={}", report.tangent_fallbacks);
        println!(
            "{prefix}.coincident_fallbacks={}",
            report.coincident_fallbacks
        );
        println!(
            "{prefix}.over_budget_fallbacks={}",
            report.over_budget_fallbacks
        );
        println!("{prefix}.invalid_fallbacks={}", report.invalid_fallbacks);
    }
}

/// Writes the unit-scale through-cut reference mesh below
/// `target/boolean_oracle`, grouped by primitive face region.
pub(crate) fn write_reference_obj() -> std::io::Result<PathBuf> {
    let (_, result) = run_difference(1.0, 5);
    let directory = PathBuf::from("target/boolean_oracle");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("semi_analytic_box_cylinder.obj");
    std::fs::write(&path, mesh_to_region_obj(&result.mesh))?;
    Ok(path)
}

fn run_difference(scale: f32, depth: u8) -> (FeatureReport, SemiAnalyticContourResult) {
    let geometry = through_cut_geometry(scale, [0.0, 0.0, 1.0]);
    let field = Difference::new(box_leaf(geometry), cylinder_leaf(geometry));
    run_typed("difference", scale, depth, geometry, &field)
}

fn run_intersection(scale: f32, depth: u8) -> FeatureReport {
    let geometry = Geometry {
        box_center: [0.0; 3],
        box_half_extents: [scale; 3],
        cylinder_center: [0.0; 3],
        cylinder_axis: [0.0, 0.0, 1.0],
        cylinder_radius: 0.72 * scale,
        cylinder_half_height: 1.35 * scale,
    };
    let field = Intersection::new(box_leaf(geometry), cylinder_leaf(geometry));
    run_typed("intersection", scale, depth, geometry, &field).0
}

fn run_union(scale: f32, depth: u8) -> FeatureReport {
    let geometry = Geometry {
        box_center: [0.0; 3],
        box_half_extents: [0.8 * scale; 3],
        cylinder_center: [0.65 * scale, 0.0, 0.0],
        cylinder_axis: [0.0, 0.0, 1.0],
        cylinder_radius: 0.55 * scale,
        cylinder_half_height: 0.6 * scale,
    };
    let field = Union::new(box_leaf(geometry), cylinder_leaf(geometry));
    run_typed("union", scale, depth, geometry, &field).0
}

fn run_rotated_fallback(scale: f32, depth: u8) -> FeatureReport {
    let geometry = through_cut_geometry(scale, [1.0, 1.0, 0.0]);
    let field = Difference::new(box_leaf(geometry), cylinder_leaf(geometry));
    let (report, semi) = run_typed("rotated_difference", scale, depth, geometry, &field);
    let ordinary = dual_contour(&field, &params(centered_bounds(scale), depth))
        .expect("rotated ordinary QEF extraction");
    let (ordinary_tri, _) = ordinary.mesh.to_trimesh(&ExtractParams::default());
    let (semi_tri, _) = semi.mesh.to_trimesh(&ExtractParams::default());
    assert_eq!(
        ordinary_tri.positions, semi_tri.positions,
        "rotated Unsupported path moved QEF positions"
    );
    assert_eq!(
        ordinary_tri.indices, semi_tri.indices,
        "rotated Unsupported path changed QEF topology"
    );
    report
}

fn run_translated_difference(depth: u8) -> FeatureReport {
    let offset = [2.5, -1.25, 0.5];
    let local_geometry = through_cut_geometry(1.0, [0.0, 0.0, 1.0]);
    let field = Translate::new(
        Difference::new(box_leaf(local_geometry), cylinder_leaf(local_geometry)),
        offset,
    );
    let geometry = translate_geometry(local_geometry, offset);
    let root_bounds =
        Aabb::new(add(offset, [-1.5; 3]), add(offset, [1.5; 3])).expect("translated bounds");
    run_typed_in_bounds(
        "translated_difference",
        1.0,
        depth,
        geometry,
        root_bounds,
        &field,
    )
    .0
}

fn run_uniform_scale_difference(depth: u8) -> FeatureReport {
    let factor = 3.0;
    let local_geometry = through_cut_geometry(1.0, [0.0, 0.0, 1.0]);
    let field = UniformScale::new(
        Difference::new(box_leaf(local_geometry), cylinder_leaf(local_geometry)),
        factor,
    )
    .expect("positive uniform scale");
    let geometry = through_cut_geometry(factor, [0.0, 0.0, 1.0]);
    run_typed("uniform_scale_difference", factor, depth, geometry, &field).0
}

fn run_typed<F: SemiAnalyticField>(
    name: &'static str,
    scale: f32,
    depth: u8,
    geometry: Geometry,
    field: &F,
) -> (FeatureReport, SemiAnalyticContourResult) {
    run_typed_in_bounds(name, scale, depth, geometry, centered_bounds(scale), field)
}

fn run_typed_in_bounds<F: SemiAnalyticField>(
    name: &'static str,
    scale: f32,
    depth: u8,
    geometry: Geometry,
    root_bounds: Aabb,
    field: &F,
) -> (FeatureReport, SemiAnalyticContourResult) {
    let params = params(root_bounds, depth);
    let result = dual_contour_semi_analytic(field, &params)
        .unwrap_or_else(|error| panic!("{name} scale {scale} extraction failed: {error}"));
    let repeated = dual_contour_semi_analytic(field, &params)
        .unwrap_or_else(|error| panic!("{name} scale {scale} repeat failed: {error}"));
    let signature = mesh_signature(&result.mesh);
    assert_eq!(
        signature,
        mesh_signature(&repeated.mesh),
        "{name} scale {scale} mesh signature changed"
    );
    assert_eq!(
        result.stats, repeated.stats,
        "{name} scale {scale} ordinary extraction stats changed"
    );
    assert_eq!(
        result.semi_analytic, repeated.semi_analytic,
        "{name} scale {scale} semi-analytic stats changed"
    );

    let regions = result
        .mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .expect("semi-analytic extraction must emit FACE_REGION");
    let mut box_faces = 0;
    let mut cylinder_faces = 0;
    let mut unexpected_faces = 0;
    let mut incident_regions = BTreeMap::<VertexId, u8>::new();
    for face in result.mesh.faces() {
        let mask = match regions.get(face.as_id()).copied() {
            Some(BOX_REGION) => {
                box_faces += 1;
                0b01
            }
            Some(CYLINDER_REGION) => {
                cylinder_faces += 1;
                0b10
            }
            _ => {
                unexpected_faces += 1;
                0
            }
        };
        for corner in result.mesh.face_loop(face) {
            let vertex = result
                .mesh
                .from_vertex(corner)
                .expect("face-loop corner must have a source vertex");
            *incident_regions.entry(vertex).or_default() |= mask;
        }
    }

    // Region incidence identifies a topology-derived superset of pair-feature
    // vertices. Measure the best N of that fixed set, where N is the
    // extractor's claimed snap count; tolerance does not select the sample.
    let mut feature_residuals = Vec::new();
    for (vertex, mask) in incident_regions {
        if mask != 0b11 {
            continue;
        }
        let Some(&point) = result.mesh.vertex_position(vertex) else {
            continue;
        };
        let residual = joint_implicit_residual(point, geometry);
        feature_residuals.push(residual);
    }
    feature_residuals.sort_by(f32::total_cmp);
    let seam_candidates = feature_residuals.len();
    let max_feature_implicit_residual = if result.semi_analytic.feature_snaps == 0 {
        0.0
    } else {
        result
            .semi_analytic
            .feature_snaps
            .checked_sub(1)
            .and_then(|index| feature_residuals.get(index))
            .copied()
            .unwrap_or(f32::INFINITY)
    };

    let report = FeatureReport {
        name,
        scale,
        depth,
        signature,
        topology_issues: result.mesh.validate_deep().len(),
        active_cells: result.stats.active_cells,
        box_faces,
        cylinder_faces,
        unexpected_faces,
        seam_candidates,
        max_feature_implicit_residual,
        surface_projections: result.semi_analytic.surface_projections,
        feature_snaps: result.semi_analytic.feature_snaps,
        unsupported_fallbacks: result.semi_analytic.unsupported_fallbacks,
        ambiguous_fallbacks: result.semi_analytic.ambiguous_fallbacks,
        tangent_fallbacks: result.semi_analytic.tangent_fallbacks,
        coincident_fallbacks: result.semi_analytic.coincident_fallbacks,
        over_budget_fallbacks: result.semi_analytic.over_budget_fallbacks,
        invalid_fallbacks: result.semi_analytic.invalid_fallbacks,
    };
    (report, result)
}

fn box_leaf(geometry: Geometry) -> BoxLeaf {
    TaggedField {
        field: BoxField {
            center: geometry.box_center,
            half_extents: geometry.box_half_extents,
        },
        provenance: BOX_REGION,
    }
}

fn cylinder_leaf(geometry: Geometry) -> CylinderLeaf {
    TaggedField {
        field: CylinderField {
            center: geometry.cylinder_center,
            axis: geometry.cylinder_axis,
            radius: geometry.cylinder_radius,
            half_height: geometry.cylinder_half_height,
        },
        provenance: CYLINDER_REGION,
    }
}

fn through_cut_geometry(scale: f32, axis: [f32; 3]) -> Geometry {
    Geometry {
        box_center: [0.0; 3],
        box_half_extents: [scale; 3],
        cylinder_center: [0.0; 3],
        cylinder_axis: axis,
        cylinder_radius: 0.6 * scale,
        cylinder_half_height: 1.5 * scale,
    }
}

fn translate_geometry(mut geometry: Geometry, offset: [f32; 3]) -> Geometry {
    geometry.box_center = add(geometry.box_center, offset);
    geometry.cylinder_center = add(geometry.cylinder_center, offset);
    geometry
}

fn centered_bounds(scale: f32) -> Aabb {
    Aabb::new([-1.5 * scale; 3], [1.5 * scale; 3]).expect("valid centered bounds")
}

fn params(root_bounds: Aabb, max_depth: u8) -> DualContourParams {
    DualContourParams {
        root_bounds,
        max_depth,
        cell_budget: None,
        edge_search: EdgeSearchParams {
            bisection_steps: 10,
        },
        qef: QefParams::default(),
    }
}

fn feature_tolerance(scale: f32) -> f32 {
    2.0e-5 * scale
}

fn box_implicit_residual(point: [f32; 3], geometry: Geometry) -> f32 {
    let mut residual = f32::NEG_INFINITY;
    for (axis, coordinate) in point.into_iter().enumerate() {
        residual = residual
            .max((coordinate - geometry.box_center[axis]).abs() - geometry.box_half_extents[axis]);
    }
    residual.abs()
}

fn cylinder_residual(point: [f32; 3], geometry: Geometry) -> f32 {
    let axis = normalize(geometry.cylinder_axis);
    let delta = sub(point, geometry.cylinder_center);
    let axial = dot(delta, axis);
    let radial_vector = sub(delta, scale(axis, axial));
    let radial = dot(radial_vector, radial_vector).sqrt();
    let side = radial - geometry.cylinder_radius;
    let cap = axial.abs() - geometry.cylinder_half_height;
    let outside = (side.max(0.0).powi(2) + cap.max(0.0).powi(2)).sqrt();
    (outside + side.max(cap).min(0.0)).abs()
}

fn joint_implicit_residual(point: [f32; 3], geometry: Geometry) -> f32 {
    box_implicit_residual(point, geometry).max(cylinder_residual(point, geometry))
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    exedra_math::normalize(value).unwrap_or([f32::NAN; 3])
}

fn mesh_signature(mesh: &Mesh) -> u64 {
    let (triangles, _) = mesh.to_trimesh(&ExtractParams::default());
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for position in triangles.positions {
        for component in position {
            hash = fnv_bytes(hash, &component.to_bits().to_le_bytes());
        }
    }
    for index in triangles.indices {
        hash = fnv_bytes(hash, &index.to_le_bytes());
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

fn mesh_to_region_obj(mesh: &Mesh) -> String {
    let mut output = String::from(
        "# deterministic semi-analytic box/cylinder reference\n# face groups are primitive identities\n",
    );
    let mut obj_indices = BTreeMap::<VertexId, usize>::new();
    for (index, vertex) in mesh.vertices().enumerate() {
        let position = mesh
            .vertex_position(vertex)
            .expect("live vertex must have a position");
        writeln!(
            output,
            "v {:.9} {:.9} {:.9}",
            position[0], position[1], position[2]
        )
        .expect("writing to String cannot fail");
        obj_indices.insert(vertex, index + 1);
    }

    let regions = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .expect("semi-analytic mesh must have face regions");
    let mut groups = BTreeMap::<u32, Vec<Vec<usize>>>::new();
    for face in mesh.faces() {
        let region = regions.get(face.as_id()).copied().unwrap_or(0);
        let vertices = mesh
            .face_loop(face)
            .map(|corner| {
                let vertex = mesh
                    .from_vertex(corner)
                    .expect("face-loop corner must have a source vertex");
                obj_indices[&vertex]
            })
            .collect();
        groups.entry(region).or_default().push(vertices);
    }
    for (region, faces) in groups {
        writeln!(output, "g primitive_{region}").expect("writing to String cannot fail");
        for face in faces {
            output.push('f');
            for index in face {
                write!(output, " {index}").expect("writing to String cannot fail");
            }
            output.push('\n');
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

    #[test]
    fn fixed_semi_analytic_suite_is_clean_and_deterministic() {
        let first = run_suite();
        let second = run_suite();
        assert_suite(&first);
        assert_eq!(first, second);
    }

    #[test]
    fn reference_obj_is_stable_and_grouped_by_primitive() {
        let (_, first_result) = run_difference(1.0, 5);
        let (_, second_result) = run_difference(1.0, 5);
        let first = mesh_to_region_obj(&first_result.mesh);
        let second = mesh_to_region_obj(&second_result.mesh);
        assert_eq!(first, second);
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
        assert!(joint_implicit_residual([0.6, 0.0, 1.0], through_cut) <= f32::EPSILON);

        let cap_line = Geometry {
            cylinder_radius: 1.2,
            cylinder_half_height: 0.75,
            ..through_cut
        };
        assert!(joint_implicit_residual([1.0, 0.0, 0.75], cap_line) <= f32::EPSILON);
    }
}
