// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Numerical GLB regressions for imported normals and their corner attributes.

use super::*;
use exedra_assembly::{CompilePolicy, NormalsSource, PartCompiler, flatten};
use exedra_constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};
use exedra_mesh::{Mesh, MeshBuilder, op};

fn authored_quad(partial: bool) -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2]).unwrap();
    builder.add_face(&[0, 2, 3]).unwrap();
    let mut mesh = builder.build().unwrap().mesh;
    let faces: Vec<_> = mesh.faces().collect();
    for (i, face) in faces.into_iter().enumerate() {
        let corners: Vec<_> = mesh
            .face_loop(face)
            .map(|h| {
                (
                    h,
                    *mesh.vertex_position(mesh.to_vertex(h).unwrap()).unwrap(),
                )
            })
            .collect();
        let mut edit = mesh.edit();
        op::set_face_region(&mut edit, face, if i == 0 { 10 } else { 20 }).unwrap();
        for (corner, position) in corners {
            op::set_corner_uv(
                &mut edit,
                corner,
                [0.125 + 0.5 * position[0], 0.25 + 0.25 * position[1]],
            )
            .unwrap();
            if !partial || i == 0 {
                op::set_corner_normal_override(&mut edit, corner, Some([0.6, 0.0, 0.8])).unwrap();
            }
        }
        let _: () = edit.finish();
    }
    mesh
}

pub(super) fn read_floats<const N: usize>(
    document: &GlbDocument,
    accessor: &Value,
    index: usize,
) -> [f32; N] {
    assert_eq!(accessor["componentType"], 5126);
    let view = &document.json()["bufferViews"]
        [usize::try_from(accessor["bufferView"].as_u64().unwrap()).unwrap()];
    let offset = usize::try_from(view["byteOffset"].as_u64().unwrap_or(0)).unwrap()
        + usize::try_from(accessor["byteOffset"].as_u64().unwrap_or(0)).unwrap()
        + index * N * 4;
    core::array::from_fn(|i| {
        f32::from_le_bytes(
            document.bin()[offset + i * 4..offset + (i + 1) * 4]
                .try_into()
                .unwrap(),
        )
    })
}

pub(super) fn read_indices(document: &GlbDocument, primitive: &Value) -> Vec<usize> {
    let accessor = &document.json()["accessors"]
        [usize::try_from(primitive["indices"].as_u64().unwrap()).unwrap()];
    assert_eq!(accessor["componentType"], 5125);
    let view = &document.json()["bufferViews"]
        [usize::try_from(accessor["bufferView"].as_u64().unwrap()).unwrap()];
    let offset = usize::try_from(view["byteOffset"].as_u64().unwrap_or(0)).unwrap()
        + usize::try_from(accessor["byteOffset"].as_u64().unwrap_or(0)).unwrap();
    (0..usize::try_from(accessor["count"].as_u64().unwrap()).unwrap())
        .map(|i| {
            u32::from_le_bytes(
                document.bin()[offset + i * 4..offset + (i + 1) * 4]
                    .try_into()
                    .unwrap(),
            ) as usize
        })
        .collect()
}

pub(super) fn attribute<'a>(document: &'a GlbDocument, primitive: &Value, name: &str) -> &'a Value {
    &document.json()["accessors"]
        [usize::try_from(primitive["attributes"][name].as_u64().unwrap()).unwrap()]
}

fn close<const N: usize>(actual: [f32; N], expected: [f64; N]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (f64::from(actual) - expected).abs() < 1.0e-6,
            "actual={actual} expected={expected}"
        );
    }
}

#[test]
fn baked_authored_normals_are_opt_in_and_reach_glb() {
    let mut assembly = Assembly::new();
    let part = assembly
        .add_baked_part("authored", authored_quad(false), &[])
        .unwrap();
    assembly
        .add_instance(None, "instance", part, Placement3::IDENTITY)
        .unwrap();
    let mut compiler = PartCompiler::new();
    for normals in [NormalsSource::Derived, NormalsSource::CustomOrDerived] {
        let policy = CompilePolicy {
            normals,
            ..CompilePolicy::default()
        };
        let compiled = compiler.compile_parts(&assembly, &policy).unwrap();
        let export = export_glb(&assembly, &compiled).unwrap();
        let document = GlbDocument::parse(&export.bytes).unwrap();
        let primitive = &document.json()["meshes"][0]["primitives"][0];
        let actual = read_floats::<3>(&document, attribute(&document, primitive, "NORMAL"), 0);
        close(
            actual,
            if normals == NormalsSource::Derived {
                [0.0, 0.0, 1.0]
            } else {
                [0.6, 0.0, 0.8]
            },
        );
    }
    assert_eq!(compiler.counters().parts_compiled, 2);
}

#[test]
fn imported_and_generated_normals_survive_placement_and_partial_coverage_in_glb() {
    for partial in [false, true] {
        for scale in [
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
            [2.0, 3.0, 4.0],
            [-2.0, 3.0, 4.0],
        ] {
            let placement = Placement3 {
                rows: [
                    [scale[0], 0.0, 0.0, 0.0],
                    [0.0, scale[1], 0.0, 0.0],
                    [0.0, 0.0, scale[2], 0.0],
                ],
            };
            let mut builder = RecipeBuilder::new();
            let import = builder.add_import(authored_quad(partial)).unwrap();
            let imported = builder
                .add(NodeKind::MeshImport { import, placement })
                .unwrap();
            let generated = builder
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box { size: [1.0; 3] },
                    placement: Placement3::translate(5.0, 0.0, 0.0),
                })
                .unwrap();
            let root = builder
                .add(NodeKind::Group {
                    children: vec![imported, generated],
                })
                .unwrap();
            let mut assembly = Assembly::new();
            let part = assembly
                .add_recipe_part("mixed", builder.finish(root).unwrap())
                .unwrap();
            assembly
                .add_instance(None, "instance", part, Placement3::IDENTITY)
                .unwrap();
            let compiled = PartCompiler::new()
                .compile_parts(
                    &assembly,
                    &CompilePolicy {
                        normals: NormalsSource::CustomOrDerived,
                        ..CompilePolicy::default()
                    },
                )
                .unwrap();
            let list = flatten(&assembly, &compiled);
            assert_eq!(list.items.len(), 2);
            let export = export_glb(&assembly, &compiled).unwrap();
            let document = GlbDocument::parse(&export.bytes).unwrap();
            check_imported(&document, scale, partial);
            check_generated(&document);
        }
    }
}

fn check_imported(document: &GlbDocument, scale: [f64; 3], partial: bool) {
    let mut expected = [0.6 / scale[0], 0.0, 0.8 / scale[2]];
    let length = (expected[0] * expected[0] + expected[2] * expected[2]).sqrt();
    expected = expected.map(|v| v / length);
    let primitives = document.json()["meshes"][0]["primitives"]
        .as_array()
        .unwrap();
    assert_eq!(primitives.len(), 2);
    for primitive in primitives {
        let derived = partial && primitive["extras"]["faceRegion"] == 20;
        let normal = if derived { [0.0, 0.0, 1.0] } else { expected };
        let indices = read_indices(document, primitive);
        let mut triangle = Vec::new();
        for index in indices {
            let position =
                read_floats::<3>(document, attribute(document, primitive, "POSITION"), index);
            let actual =
                read_floats::<3>(document, attribute(document, primitive, "NORMAL"), index);
            let uv = read_floats::<2>(
                document,
                attribute(document, primitive, "TEXCOORD_0"),
                index,
            );
            close(actual, normal);
            close(
                uv,
                [
                    0.125 + 0.5 * f64::from(position[0]) / scale[0],
                    0.25 + 0.25 * f64::from(position[1]) / scale[1],
                ],
            );
            triangle.push(position);
        }
        assert_eq!(triangle.len(), 3);
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        assert!(
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) > 0.0,
            "reflected winding remains outward"
        );
    }
}

fn check_generated(document: &GlbDocument) {
    let primitive = &document.json()["meshes"][1]["primitives"][0];
    let normals = attribute(document, primitive, "NORMAL");
    for index in 0..usize::try_from(normals["count"].as_u64().unwrap()).unwrap() {
        let normal = read_floats::<3>(document, normals, index);
        let axis_count = normal.into_iter().filter(|v| v.abs() == 1.0).count();
        assert_eq!(axis_count, 1, "box retains derived unit face normals");
        assert_eq!(normal.into_iter().filter(|v| *v == 0.0).count(), 2);
    }
}
