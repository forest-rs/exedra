// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::cell::RefCell;

use super::*;
use exedra_assembly::{CompilePolicy, PartCompiler};
use exedra_mesh::{Mesh, MeshBuilder, UvSource, op};

// A complete 1x1 PNG; export must preserve these exact encoded bytes.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5, 1, 1,
    39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

struct Resources {
    calls: RefCell<Vec<u32>>,
    sampler: Option<Value>,
    image: &'static [u8],
    mime: &'static str,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            calls: RefCell::default(),
            sampler: Some(json!({"wrapS":10497,"wrapT":10497})),
            image: PNG,
            mime: "image/png",
        }
    }
}

impl MaterialResolver for Resources {
    fn resolve(&self, key: &str) -> Option<Value> {
        let index = match key {
            "a" => 17,
            "b" => 99,
            "missing" => 500,
            _ => return None,
        };
        Some(
            json!({"pbrMetallicRoughness": {"metallicFactor":0, "baseColorTexture":{"index":index,"texCoord":0}}}),
        )
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        self.calls.borrow_mut().push(index);
        (index != 500).then(|| Texture {
            image: self.image,
            mime_type: self.mime,
            sampler: self.sampler.clone(),
        })
    }
}

fn triangle(uv_count: usize, uv: [f32; 2]) -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2]).unwrap();
    let mut mesh = builder.build().unwrap().mesh;
    let corners: Vec<_> = mesh.face_loop(mesh.faces().next().unwrap()).collect();
    let mut edit = mesh.edit();
    for corner in corners.into_iter().take(uv_count) {
        op::set_corner_uv(&mut edit, corner, uv).unwrap();
    }
    let _: () = edit.finish();
    mesh
}

fn assembly(mesh: Mesh) -> Assembly {
    let mut assembly = Assembly::new();
    let part = assembly
        .add_baked_part("sample", mesh, &["surface"])
        .unwrap();
    assembly.set_default_slot(part, "surface").unwrap();
    assembly.set_part_material(part, "surface", "a").unwrap();
    assembly
        .add_instance(None, "first", part, Placement3::IDENTITY)
        .unwrap();
    let second = assembly
        .add_instance(None, "second", part, Placement3::translate(2., 0., 0.))
        .unwrap();
    assembly.bind_material(second, "surface", "b").unwrap();
    assembly
}

#[test]
fn remaps_resources_and_preserves_embedded_bytes_and_shared_geometry() {
    let assembly = assembly(triangle(3, [0., 0.]));
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let resources = Resources::default();
    let options = GltfExportOptions::default();
    let export = export_glb_with_materials(&assembly, &compiled, &resources, options).unwrap();
    assert_eq!(*resources.calls.borrow(), [17, 99]);
    assert_eq!(
        (
            export.stats.images,
            export.stats.textures,
            export.stats.image_bytes
        ),
        (1, 1, PNG.len() as u64)
    );
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    let j = doc.json();
    assert_eq!(
        j["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
        0
    );
    assert_eq!(
        j["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
        0
    );
    assert_eq!(
        j["meshes"][0]["primitives"][0]["attributes"],
        j["meshes"][1]["primitives"][0]["attributes"]
    );
    let view =
        &j["bufferViews"][usize::try_from(j["images"][0]["bufferView"].as_u64().unwrap()).unwrap()];
    let start = usize::try_from(view["byteOffset"].as_u64().unwrap()).unwrap();
    assert_eq!(start % 4, 0);
    assert!(view.get("target").is_none());
    assert_eq!(view["byteLength"], PNG.len());
    assert_eq!(&doc.bin()[start..start + PNG.len()], PNG);
    let repeat = export_glb_with_materials(&assembly, &compiled, &resources, options).unwrap();
    assert_eq!(repeat.bytes, export.bytes);
    let text = export_gltf_with_materials(&assembly, &compiled, &resources, options).unwrap();
    let text: Value = serde_json::from_str(&text.json).unwrap();
    for field in ["images", "textures", "samplers", "materials", "bufferViews"] {
        assert_eq!(text[field], j[field]);
    }
    assert_eq!(compiler.counters().parts_compiled, 1);
}

#[test]
fn missing_partial_and_nonfinite_uvs_are_rejected_but_authored_zero_is_valid() {
    for (count, uv) in [
        (0, [0., 0.]),
        (2, [0., 0.]),
        (3, [f32::NAN, 0.]),
        (3, [f32::INFINITY, 0.]),
    ] {
        let assembly = assembly(triangle(count, uv));
        let compiled = PartCompiler::new()
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        assert!(!compiled.part(PartId(0)).unwrap().bodies[0].regions[0].has_uvs);
        assert!(matches!(
            export_glb_with_materials(
                &assembly,
                &compiled,
                &Resources::default(),
                GltfExportOptions::default()
            ),
            Err(GltfError::MissingTextureCoordinates {
                part: 0,
                body: 0,
                region: 0,
                ..
            })
        ));
    }
    let assembly = assembly(triangle(0, [0., 0.]));
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    assert!(
        export_glb(&assembly, &compiled).is_ok(),
        "untextured export needs no UVs"
    );
}

#[test]
fn box_projected_uvs_satisfy_textured_materials_without_authored_uvs() {
    let assembly = assembly(triangle(0, [0., 0.]));
    let mut compiler = PartCompiler::new();
    let custom_only = compiler
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    assert!(!custom_only.part(PartId(0)).unwrap().bodies[0].regions[0].has_uvs);
    assert!(matches!(
        export_glb_with_materials(
            &assembly,
            &custom_only,
            &Resources::default(),
            GltfExportOptions::default()
        ),
        Err(GltfError::MissingTextureCoordinates {
            part: 0,
            body: 0,
            region: 0,
            ..
        })
    ));

    let projected = compiler
        .compile_parts(
            &assembly,
            &CompilePolicy {
                uvs: UvSource::CustomOrBoxProjected { scale: 1.0 },
                ..CompilePolicy::default()
            },
        )
        .unwrap();
    assert_eq!(
        compiler.counters().parts_compiled,
        2,
        "the UV policy is a cache key"
    );
    let body = &projected.part(PartId(0)).unwrap().bodies[0];
    assert!(body.regions[0].has_uvs);
    // The triangle lies in z = 0 facing +Z, so the projection is its XY.
    for (position, uv) in body.tri.positions.iter().zip(&body.tri.uvs) {
        assert_eq!(*uv, [position[0], position[1]]);
    }
    let export = export_glb_with_materials(
        &assembly,
        &projected,
        &Resources::default(),
        GltfExportOptions::default(),
    )
    .unwrap();
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert!(
        doc.json()["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"].is_number(),
        "textured primitives carry projected texture coordinates"
    );
}

#[test]
fn rejects_missing_resources_invalid_encoding_and_sampler_fields() {
    let mut assembly = assembly(triangle(3, [0., 0.]));
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let options = GltfExportOptions::default();
    for resources in [
        Resources {
            image: b"invalid",
            ..Resources::default()
        },
        Resources {
            mime: "image/webp",
            ..Resources::default()
        },
        Resources {
            sampler: Some(json!({"wrapS":0})),
            ..Resources::default()
        },
        Resources {
            sampler: Some(json!({"minFilter":9728.5})),
            ..Resources::default()
        },
        Resources {
            sampler: Some(json!({"extensions":{}})),
            ..Resources::default()
        },
    ] {
        assert!(matches!(
            export_glb_with_materials(&assembly, &compiled, &resources, options),
            Err(GltfError::InvalidTexture { .. })
        ));
    }
    assembly
        .set_part_material(PartId(0), "surface", "missing")
        .unwrap();
    assert!(matches!(
        export_glb_with_materials(&assembly, &compiled, &Resources::default(), options),
        Err(GltfError::MissingTexture { index: 500 })
    ));
}

#[test]
fn texture_info_refuses_missing_indices_other_sets_and_extensions() {
    for info in [
        json!({}),
        json!({"index":-1}),
        json!({"index":1.5}),
        json!({"index":4294967296_u64}),
        json!({"index":0,"texCoord":1}),
        json!({"index":0,"extensions":{}}),
    ] {
        assert!(
            materials::validate(
                json!({"pbrMetallicRoughness":{"baseColorTexture":info}}),
                "a"
            )
            .is_err()
        );
    }
}

#[test]
fn untextured_regions_do_not_require_uvs_from_their_textured_neighbors() {
    let mut builder = MeshBuilder::new();
    for p in [
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [2., 0., 0.],
        [3., 0., 0.],
        [2., 1., 0.],
    ] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2]).unwrap();
    builder.add_face(&[3, 4, 5]).unwrap();
    let mut mesh = builder.build().unwrap().mesh;
    let faces: Vec<_> = mesh.faces().collect();
    let corners: Vec<_> = mesh.face_loop(faces[0]).collect();
    let mut edit = mesh.edit();
    op::set_face_region(&mut edit, faces[0], 10).unwrap();
    op::set_face_region(&mut edit, faces[1], 20).unwrap();
    for corner in corners {
        op::set_corner_uv(&mut edit, corner, [0., 0.]).unwrap();
    }
    let _: () = edit.finish();
    let mut assembly = Assembly::new();
    let part = assembly
        .add_baked_part("mixed", mesh, &["textured", "plain"])
        .unwrap();
    assembly.bind_region_slot(part, 10, "textured").unwrap();
    assembly.bind_region_slot(part, 20, "plain").unwrap();
    assembly.set_part_material(part, "textured", "a").unwrap();
    assembly
        .add_instance(None, "mixed", part, Placement3::IDENTITY)
        .unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let ranges = &compiled.part(part).unwrap().bodies[0].regions;
    assert_eq!(
        ranges
            .iter()
            .map(|r| (r.region, r.has_uvs))
            .collect::<Vec<_>>(),
        [(10, true), (20, false)]
    );
    let options = GltfExportOptions::default();
    assert!(
        export_glb_with_materials(&assembly, &compiled, &Resources::default(), options).is_ok()
    );
    assembly.set_part_material(part, "plain", "a").unwrap();
    assert!(matches!(
        export_glb_with_materials(&assembly, &compiled, &Resources::default(), options),
        Err(GltfError::MissingTextureCoordinates { region: 20, .. })
    ));
}

#[test]
fn different_samplers_share_the_image_but_keep_distinct_textures() {
    struct Sampling;
    impl MaterialResolver for Sampling {
        fn resolve(&self, key: &str) -> Option<Value> {
            Resources::default().resolve(key)
        }
        fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
            Some(Texture {
                image: PNG,
                mime_type: "image/png",
                sampler: Some(json!({"wrapS":if index == 17 {10497} else {33071}})),
            })
        }
    }
    let assembly = assembly(triangle(3, [0., 0.]));
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let export = export_glb_with_materials(
        &assembly,
        &compiled,
        &Sampling,
        GltfExportOptions::default(),
    )
    .unwrap();
    assert_eq!((export.stats.images, export.stats.textures), (1, 2));
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        doc.json()["textures"][0]["source"],
        doc.json()["textures"][1]["source"]
    );
    assert_ne!(
        doc.json()["textures"][0]["sampler"],
        doc.json()["textures"][1]["sampler"]
    );
}
