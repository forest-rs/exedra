// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;
use exedra_mesh::{ChangeSetBuilder, attr, op};

const P: [[f32; 3]; 3] = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
const UV: [[f32; 2]; 3] = [[0.0; 2], [1.0, 0.0], [0.0, 1.0]];
fn close(a: f64, b: f64) {
    assert!(
        (a - b).abs() < 1e-10 * a.abs().max(b.abs()).max(1.0),
        "{a} != {b}"
    );
}
fn triangle_mesh() -> Mesh {
    let mut mesh = Mesh::from_polygons(&P, &[&[0, 1, 2]]).unwrap();
    let face = mesh.faces().next().unwrap();
    let corners: Vec<_> = mesh.face_loop(face).collect();
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    for corner in corners {
        let p = *edit
            .mesh()
            .vertex_position(edit.mesh().to_vertex(corner).unwrap())
            .unwrap();
        op::set_corner_uv(&mut edit, corner, [p[0], p[1]]).unwrap();
    }
    let _ = edit.finish();
    mesh
}
#[test]
fn analytic_scale_shear_resolution_and_rigid_invariance() {
    let m = measure(P, UV, [1024, 512]).unwrap();
    close(m.principal_density[0], 512.0);
    close(m.principal_density[1], 1024.0);
    close(m.anisotropy, 2.0);
    close(m.density, libm::sqrt(512.0 * 1024.0));
    close(m.surface_area, 0.5);
    close(m.texture_area, 262144.0);
    let shear = measure(P, [[0.0; 2], [1.0, 0.0], [1.0, 1.0]], [1; 2]).unwrap();
    close(shear.principal_density[1], (1.0 + libm::sqrt(5.0)) * 0.5);
    close(shear.density, 1.0);
    close(shear.principal_density[0] * shear.principal_density[1], 1.0);
    let transformed = P.map(|p| [3.0 - p[1], 7.0 + p[0], 2.0]);
    let translated = UV.map(|uv| [uv[0] + 16.0, uv[1] - 32.0]);
    let moved = measure(transformed, translated, [1024, 512]).unwrap();
    assert_eq!(m, moved);
    let cyclic = measure([P[1], P[2], P[0]], [UV[1], UV[2], UV[0]], [1024, 512]).unwrap();
    close(cyclic.anisotropy, m.anisotropy);
    close(cyclic.density, m.density);
    let scaled = measure(P.map(|p| p.map(|x| x * 2.0)), UV, [1024, 512]).unwrap();
    close(scaled.density, m.density * 0.5);
    close(scaled.anisotropy, m.anisotropy);
}
#[test]
fn thin_triangles_do_not_lose_the_small_principal_density() {
    let points = [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0e-12, 0.0]];
    let uv = [[0.0; 2], [1.0, 0.0], [1.0, 1.0e-12]];
    let m = measure(points, uv, [1; 2]).unwrap();
    close(m.anisotropy, 1.0);
    close(m.density, 1.0);
    let m = measure(P, [[0.0; 2], [1.0, 0.0], [0.0, 1.0e-12]], [1; 2]).unwrap();
    close(m.principal_density[0], f64::from(1.0e-12_f32));
    close(m.principal_density[1], 1.0);
}
#[test]
fn degenerate_and_nonfinite_inputs_are_refusals_not_infinite_metrics() {
    assert_eq!(
        measure([[0.0; 3]; 3], UV, [1; 2]),
        Err(UvQualityIssue::DegenerateGeometry)
    );
    assert_eq!(
        measure(P, [[1.0; 2]; 3], [1; 2]),
        Err(UvQualityIssue::DegenerateUv)
    );
    let mut p = P;
    p[0][0] = f32::NAN;
    assert_eq!(
        measure(p, UV, [1; 2]),
        Err(UvQualityIssue::NonfiniteGeometry)
    );
    let mut uv = UV;
    uv[0][0] = f32::INFINITY;
    assert_eq!(measure(P, uv, [1; 2]), Err(UvQualityIssue::NonfiniteUv));
}
#[test]
fn survey_and_streaming_agree_with_bounded_evidence_and_mirror_policy() {
    let mesh = triangle_mesh();
    let revision = mesh.revision();
    let mut params = UvQualityParams::default();
    params.limits.max_density = Some(0.5);
    params.witness_limit = 0;
    let mut scratch = UvQualityScratch::default();
    let a = inspect_uv_quality(&mesh, &params, &mut scratch).unwrap();
    let mut triangles = Vec::new();
    let b = visit_uv_quality(&mesh, &params, &mut scratch, |triangle| {
        triangles.push(triangle);
    })
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(a.triangles_outside_limits, 1);
    assert_eq!(a.witnesses_omitted, 1);
    assert!(a.witnesses.is_empty());
    assert_eq!(mesh.revision(), revision);
    assert!(!a.is_acceptable());
    assert!(triangles[0].violations.density_above);
    let mirrored = measure(P, UV.map(|uv| [-uv[0], uv[1]]), [1; 2]).unwrap();
    assert!(mirrored.mirrored);
    assert!(!UvQualityLimits::default().check(mirrored).any());
    assert!(
        UvQualityLimits {
            forbid_mirrored: true,
            ..UvQualityLimits::default()
        }
        .check(mirrored)
        .mirrored
    );
    let bytes = scratch.retained_bytes();
    inspect_uv_quality(&mesh, &params, &mut scratch).unwrap();
    assert_eq!(scratch.retained_bytes(), bytes);
}
#[test]
fn secondary_dense_channels_and_canonical_selection_have_exact_witnesses() {
    let mut mesh = triangle_mesh();
    mesh.define_dense_layer(attr::CORNER_UV1, [0.0; 2]).unwrap();
    let face = mesh.faces().next().unwrap();
    let corners: Vec<_> = mesh.face_loop(face).collect();
    let mut edit = mesh.edit();
    for &corner in &corners {
        let p = *edit
            .mesh()
            .vertex_position(edit.mesh().to_vertex(corner).unwrap())
            .unwrap();
        op::set_attribute(&mut edit, attr::CORNER_UV1, corner, [p[0] * 2.0, p[1]]).unwrap();
    }
    let _: () = edit.finish();
    let params = UvQualityParams {
        scope: UvScope::FaceSet(vec![face, face]),
        set: UvSet::Secondary,
        limits: UvQualityLimits {
            max_anisotropy: Some(1.5),
            ..UvQualityLimits::default()
        },
        ..UvQualityParams::default()
    };
    let report = inspect_uv_quality(&mesh, &params, &mut UvQualityScratch::default()).unwrap();
    assert_eq!(report.faces_examined, 1);
    assert!(report.selections_canonicalized);
    assert_eq!(report.witnesses[0].face, face);
    assert_eq!(
        report.witnesses[0].triangle.unwrap().corners,
        corners.as_slice()
    );
    close(report.max_anisotropy.unwrap().value, 2.0);
    close(report.min_density.unwrap().value, libm::sqrt(2.0));
}
#[test]
fn missing_coverage_is_counted_and_invalid_parameters_name_the_field() {
    let mesh = Mesh::from_polygons(&P, &[&[0, 1, 2]]).unwrap();
    let mut params = UvQualityParams::default();
    let report = inspect_uv_quality(&mesh, &params, &mut UvQualityScratch::default()).unwrap();
    assert_eq!(report.refusals.missing_uv, 1);
    assert_eq!(report.triangles_measured, 0);
    assert_eq!(report.witnesses[0].issue, UvQualityIssue::MissingUv);
    params.texture_size = [0, 1];
    assert_eq!(
        inspect_uv_quality(&mesh, &params, &mut UvQualityScratch::default()),
        Err(UvQualityError::InvalidParameter {
            field: "texture_size"
        })
    );
    params = UvQualityParams::default();
    params.scope = UvScope::FaceSet(vec![FaceId::OUTSIDE]);
    assert_eq!(
        inspect_uv_quality(&mesh, &params, &mut UvQualityScratch::default()),
        Err(UvQualityError::Mesh(UvError::InvalidFace {
            face: FaceId::OUTSIDE
        }))
    );
}
#[test]
fn robust_fallback_is_an_explicit_face_refusal() {
    let mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap();
    let params = UvQualityParams {
        triangulation: FaceTriangulation::Robust,
        ..UvQualityParams::default()
    };
    let mut calls = 0;
    let report = visit_uv_quality(&mesh, &params, &mut UvQualityScratch::default(), |_| {
        calls += 1;
    })
    .unwrap();
    assert_eq!(calls, 0);
    assert_eq!(report.triangulation_fallback_faces, 1);
    assert_eq!(
        report.witnesses[0].issue,
        UvQualityIssue::TriangulationFallback
    );
    assert!(!report.is_acceptable());
}

#[test]
fn threshold_area_is_weighted_by_surface_coverage_not_triangle_count() {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [10.0, 0.0, 1.0],
            [0.0, 10.0, 1.0],
        ],
        &[&[0, 1, 2], &[3, 4, 5]],
    )
    .unwrap();
    let corners: Vec<_> = mesh.faces().flat_map(|face| mesh.face_loop(face)).collect();
    let mut edit = mesh.edit();
    for corner in corners {
        let p = *edit
            .mesh()
            .vertex_position(edit.mesh().to_vertex(corner).unwrap())
            .unwrap();
        let scale = if p[2] == 0.0 { 1.0 } else { 0.1 };
        op::set_corner_uv(&mut edit, corner, [p[0] * scale, p[1] * scale]).unwrap();
    }
    let _: () = edit.finish();
    let params = UvQualityParams {
        limits: UvQualityLimits {
            min_density: Some(0.5),
            ..UvQualityLimits::default()
        },
        ..UvQualityParams::default()
    };
    let report = inspect_uv_quality(&mesh, &params, &mut UvQualityScratch::default()).unwrap();
    assert_eq!(report.triangles_outside_limits, 1);
    assert_eq!(report.triangles_measured, 2);
    close(report.measured_surface_area, 50.5);
    close(report.outside_limits_surface_area, 50.0);
    close(
        report.outside_limits_area_fraction().unwrap(),
        100.0 / 101.0,
    );
    assert_eq!(report.texture_size, [1, 1]);
    assert_eq!(report.limits, params.limits);
}
