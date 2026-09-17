// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_mesh::{EdgeAttrPropagation, Mesh, PropagatePolicy, attr};

fn authored_quad() -> (Mesh, FaceId) {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap();
    let face = mesh.faces().next().unwrap();
    let corners: Vec<_> = mesh.face_loop(face).collect();
    let mut edit = mesh.edit();
    for (i, corner) in corners.into_iter().enumerate() {
        let point = *edit
            .mesh()
            .vertex_position(edit.mesh().to_vertex(corner).unwrap())
            .unwrap();
        op::set_corner_uv(&mut edit, corner, [point[0], point[1]]).unwrap();
        op::set_edge_seam(&mut edit, corner, i == 1).unwrap();
        op::set_edge_sharpness(&mut edit, corner, 2.0 + i as f32).unwrap();
    }
    let _: () = edit.finish();
    (mesh, face)
}
fn inherit() -> PropagatePolicy {
    PropagatePolicy {
        edge_attr: EdgeAttrPropagation::Inherit,
        ..Default::default()
    }
}
fn cut_params(face: FaceId) -> CutRectFaceParams {
    CutRectFaceParams {
        face,
        frame_origin: [0.0; 3],
        frame_u: [1.0, 0.0, 0.0],
        frame_v: [0.0, 1.0, 0.0],
        rect_min: [1.0, 1.0],
        rect_max: [3.0, 2.0],
    }
}

#[test]
fn reversed_cap_uvs_follow_source_vertices() {
    let (mut mesh, face) = authored_quad();
    let mut edit = mesh.edit();
    let (stats, output) = extrude_faces(
        &mut edit,
        &ExtrudeFacesParams {
            faces: vec![face],
            distance: 1.0,
            mode: ExtrudeMode::KeepSource,
        },
        &inherit(),
    )
    .unwrap();
    assert!(stats.reversed_winding);
    let _: () = edit.finish();
    for corner in mesh.face_loop(output.cap_faces[0]) {
        let point = mesh
            .vertex_position(mesh.to_vertex(corner).unwrap())
            .unwrap();
        assert_eq!(
            mesh.attrs()
                .sparse(attr::CORNER_UV)
                .unwrap()
                .get(corner.as_id()),
            Some(&[point[0], point[1]])
        );
    }
}

#[test]
fn reversed_cap_edge_tags_follow_source_edges() {
    let (mut mesh, face) = authored_quad();
    let original = mesh.clone();
    let mut edit = mesh.edit();
    let (_, output) = extrude_faces(
        &mut edit,
        &ExtrudeFacesParams {
            faces: vec![face],
            distance: 1.0,
            mode: ExtrudeMode::KeepSource,
        },
        &inherit(),
    )
    .unwrap();
    let _: () = edit.finish();
    for corner in mesh.face_loop(output.cap_faces[0]) {
        let a = mesh
            .vertex_position(mesh.from_vertex(corner).unwrap())
            .unwrap();
        let b = mesh
            .vertex_position(mesh.to_vertex(corner).unwrap())
            .unwrap();
        let from = original
            .face_loop(face)
            .find(|&edge| {
                let x = original
                    .vertex_position(original.from_vertex(edge).unwrap())
                    .unwrap();
                let y = original
                    .vertex_position(original.to_vertex(edge).unwrap())
                    .unwrap();
                (a[..2] == x[..2] && b[..2] == y[..2]) || (a[..2] == y[..2] && b[..2] == x[..2])
            })
            .unwrap();
        assert_eq!(mesh.edge_seam(corner), original.edge_seam(from));
    }
}

#[test]
fn rectangular_cut_outer_tags_stay_on_the_same_source_edges() {
    let (mut mesh, face) = authored_quad();
    let source: Vec<_> = mesh
        .face_loop(face)
        .map(|edge| {
            (
                mesh.from_vertex(edge).unwrap(),
                mesh.to_vertex(edge).unwrap(),
                mesh.edge_seam(edge),
                mesh.edge_sharpness(edge),
            )
        })
        .collect();
    let mut edit = mesh.edit();
    let (_, output) = cut_rect_face(&mut edit, &cut_params(face), &inherit()).unwrap();
    let _: () = edit.finish();
    for (a, b, seam, sharpness) in source {
        let edge = output
            .frame_faces
            .iter()
            .flat_map(|&face| mesh.face_loop(face))
            .find(|&edge| {
                let x = mesh.from_vertex(edge).unwrap();
                let y = mesh.to_vertex(edge).unwrap();
                (a == x && b == y) || (a == y && b == x)
            })
            .unwrap();
        assert_eq!(mesh.edge_seam(edge), seam);
        assert_eq!(mesh.edge_sharpness(edge), sharpness);
    }
}

#[test]
fn prepared_face_edits_reject_unfinished_edits_before_mutation() {
    for change_uv in [false, true] {
        let (mut mesh, face) = authored_quad();
        let inset = InsetFacesPlan::prepare(
            &mesh,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.2,
            },
        )
        .unwrap();
        let cut = CutRectFacePlan::prepare(&mesh, &cut_params(face)).unwrap();
        let corner = mesh.face_edge(face).unwrap();
        let vertex = mesh.to_vertex(corner).unwrap();
        let mut edit = mesh.edit();
        if change_uv {
            op::set_corner_uv(&mut edit, corner, [99.0, 99.0]).unwrap();
        } else {
            op::set_vertex_position(&mut edit, vertex, [0.1, 0.1, 0.0]).unwrap();
        }
        assert_eq!(
            inset.apply(&mut edit, &inherit()).unwrap_err(),
            FaceEditError::StalePreparation
        );
        assert_eq!(
            cut.apply(&mut edit, &inherit()).unwrap_err(),
            FaceEditError::StalePreparation
        );
        assert_eq!(edit.mesh().faces().collect::<Vec<_>>(), vec![face]);
    }
}

#[test]
fn degenerate_inset_and_nonfinite_attributes_fail_during_preparation() {
    let (mut mesh, face) = authored_quad();
    assert_eq!(
        InsetFacesPlan::prepare(
            &mesh,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.0
            }
        )
        .unwrap_err(),
        FaceEditError::InvalidInsetFactor
    );
    let corner = mesh.face_edge(face).unwrap();
    let mut edit = mesh.edit();
    op::set_corner_uv(&mut edit, corner, [f32::NAN, 0.0]).unwrap();
    assert_eq!(
        InsetFacesPlan::prepare(
            edit.mesh(),
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.2
            }
        )
        .unwrap_err(),
        FaceEditError::NumericLimit
    );
    assert_eq!(edit.mesh().faces().count(), 1);
}

#[test]
fn prepared_inset_compares_consumed_uv_bits_in_unfinished_edits() {
    let (mut mesh, face) = authored_quad();
    let corner = mesh
        .face_loop(face)
        .find(|&corner| {
            mesh.attrs()
                .sparse(attr::CORNER_UV)
                .unwrap()
                .get(corner.as_id())
                .unwrap()[0]
                == 0.0
        })
        .unwrap();
    let uv = *mesh
        .attrs()
        .sparse(attr::CORNER_UV)
        .unwrap()
        .get(corner.as_id())
        .unwrap();
    let plan = InsetFacesPlan::prepare(
        &mesh,
        &InsetFacesParams {
            faces: vec![face],
            factor: 0.2,
        },
    )
    .unwrap();
    let mut edit = mesh.edit();
    op::set_corner_uv(&mut edit, corner, [-0.0, uv[1]]).unwrap();
    assert_eq!(
        plan.apply(&mut edit, &inherit()).unwrap_err(),
        FaceEditError::StalePreparation
    );
}

#[test]
fn rectangular_cut_refuses_off_plane_origin_skew_axes_and_boundary_contact() {
    let (mesh, face) = authored_quad();
    let base = cut_params(face);
    for (params, expected) in [
        (
            CutRectFaceParams {
                frame_origin: [0.0, 0.0, 0.25],
                ..base.clone()
            },
            FaceEditError::RectangleOffPlane,
        ),
        (
            CutRectFaceParams {
                frame_v: [0.2, 1.0, 0.0],
                ..base.clone()
            },
            FaceEditError::InvalidRectangleFrame,
        ),
        (
            CutRectFaceParams {
                rect_min: [0.0, 0.0],
                ..base
            },
            FaceEditError::RectangleOutsideFace,
        ),
    ] {
        assert_eq!(
            CutRectFacePlan::prepare(&mesh, &params).unwrap_err(),
            expected,
            "wrong refusal for {params:?}"
        );
    }
}

#[test]
fn rectangular_cut_refuses_nonplanar_source_even_with_aligned_frame_axes() {
    let mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 3.0, 0.4],
            [0.0, 3.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap();
    let face = mesh.faces().next().unwrap();
    let vertices: Vec<_> = mesh
        .face_loop(face)
        .map(|corner| mesh.to_vertex(corner).unwrap())
        .collect();
    let normal = normalized_face_normal(&mesh, &vertices).unwrap();
    let u = normalize(sub([1.0, 0.0, 0.0], scale(normal, normal[0]))).unwrap();
    let params = CutRectFaceParams {
        face,
        frame_origin: [2.0, 1.5, 0.1],
        frame_u: u,
        frame_v: cross(normal, u),
        rect_min: [-0.5, -0.5],
        rect_max: [0.5, 0.5],
    };
    assert_eq!(
        CutRectFacePlan::prepare(&mesh, &params).unwrap_err(),
        FaceEditError::NonPlanarFace { face }
    );
}

#[test]
fn rectangular_cut_in_concave_face_preserves_area_and_winding() {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap();
    let face = mesh.faces().next().unwrap();
    let params = CutRectFaceParams {
        face,
        frame_origin: [0.0; 3],
        frame_u: [2.0, 0.0, 0.0],
        frame_v: [0.0, 3.0, 0.0],
        rect_min: [0.1, 0.1],
        rect_max: [0.3, 0.3],
    };
    let mut edit = mesh.edit();
    let (_, output) = cut_rect_face(&mut edit, &params, &inherit()).unwrap();
    let _: () = edit.finish();
    assert_eq!(output.frame_faces.len(), 4);
    assert!(mesh.validate_fast().is_empty());
    assert!(mesh.validate_deep().is_empty());
    let mut area = 0.0;
    for face in mesh.faces() {
        let points: Vec<_> = mesh
            .face_loop(face)
            .map(|corner| {
                let p = mesh
                    .vertex_position(mesh.to_vertex(corner).unwrap())
                    .unwrap();
                assert_eq!(p[2], 0.0);
                [f64::from(p[0]), f64::from(p[1])]
            })
            .collect();
        let triangles = exedra_triangulate::triangulate(
            &exedra_triangulate::PolygonInput {
                outer: &points,
                holes: &[],
            },
            &exedra_triangulate::TriParams::default(),
        )
        .unwrap();
        for triangle in triangles.triangles {
            let [a, b, c] = triangle.map(|index| points[index as usize]);
            let twice_area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert!(twice_area > 0.0);
            area += twice_area * 0.5;
        }
    }
    assert!((area - 3.5).abs() < 1e-6);
}
