// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{CompilePolicy, PartCompiler};
use exedra_mesh::{Mesh, MeshBuilder, op};

// A complete 1x1 PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5, 1, 1,
    39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// Resolves one material per key; every texture index is the same image.
struct Resolver(Value);

impl MaterialResolver for Resolver {
    fn resolve(&self, _key: &str) -> Option<Value> {
        Some(self.0.clone())
    }
    fn resolve_texture(&self, _index: u32) -> Option<Texture<'_>> {
        Some(Texture {
            image: PNG,
            mime_type: "image/png",
            sampler: None,
        })
    }
}

fn uv_triangle() -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2]).unwrap();
    let mut mesh = builder.build().unwrap().mesh;
    let corners: Vec<_> = mesh.face_loop(mesh.faces().next().unwrap()).collect();
    let mut edit = mesh.edit();
    for corner in corners {
        op::set_corner_uv(&mut edit, corner, [0.5, 0.5]).unwrap();
    }
    let _: () = edit.finish();
    mesh
}

fn export(material: Value, options: GltfExportOptions<'_>) -> Result<GlbExport, GltfError> {
    let mut assembly = Assembly::new();
    let part = assembly
        .add_baked_part("leaf", uv_triangle(), &["surface"])
        .unwrap();
    assembly.set_default_slot(part, "surface").unwrap();
    assembly.set_part_material(part, "surface", "leaf").unwrap();
    assembly
        .add_instance(None, "leaf", part, Placement3::IDENTITY)
        .unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    export_glb_with_materials(&assembly, &compiled, &Resolver(material), options)
}

fn leaf() -> Value {
    json!({
        "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}},
        "doubleSided": true,
        "extensions": {
            "KHR_materials_diffuse_transmission": {
                "diffuseTransmissionFactor": 0.4,
                "diffuseTransmissionColorFactor": [0.6, 0.9, 0.2],
                "diffuseTransmissionTexture": {
                    "index": 1,
                    "extensions": {"KHR_texture_transform": {
                        "offset": [0.5, 0.0], "rotation": 0.25, "scale": [2.0, 2.0]
                    }}
                },
                "diffuseTransmissionColorTexture": {"index": 2}
            },
            "KHR_materials_specular": {"specularFactor": 0.5},
            "KHR_materials_ior": {"ior": 1.4}
        }
    })
}

#[test]
fn allowlisted_extensions_export_with_their_textures() {
    let export = export(leaf(), GltfExportOptions::default()).unwrap();
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    let j = doc.json();
    assert_eq!(
        j["extensionsUsed"],
        json!([
            "KHR_materials_diffuse_transmission",
            "KHR_materials_ior",
            "KHR_materials_specular",
            "KHR_texture_transform"
        ])
    );
    assert!(j.get("extensionsRequired").is_none());
    let transmission = &j["materials"][0]["extensions"]["KHR_materials_diffuse_transmission"];
    // Three caller indices resolve to one shared image and texture.
    assert_eq!(transmission["diffuseTransmissionTexture"]["index"], 0);
    assert_eq!(transmission["diffuseTransmissionColorTexture"]["index"], 0);
    assert_eq!(
        transmission["diffuseTransmissionTexture"]["extensions"]["KHR_texture_transform"]["rotation"],
        0.25
    );
    assert_eq!((export.stats.images, export.stats.textures), (1, 1));
}

#[test]
fn texture_transform_can_be_required() {
    let options = GltfExportOptions::default().with_required_texture_transform(true);
    let export = export(leaf(), options).unwrap();
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        doc.json()["extensionsRequired"],
        json!(["KHR_texture_transform"])
    );
}

#[test]
fn extension_textures_and_transforms_are_checked_per_primitive() {
    // The transform's texCoord overrides the reference's own set 0.
    let mut material = leaf();
    material["extensions"]["KHR_materials_diffuse_transmission"]["diffuseTransmissionTexture"]["extensions"]
        ["KHR_texture_transform"]["texCoord"] = json!(1);
    assert_eq!(
        export(material, GltfExportOptions::default()).unwrap_err(),
        GltfError::MissingTextureCoordinateSet {
            material: "leaf".into(),
            texture: "extensions.KHR_materials_diffuse_transmission.diffuseTransmissionTexture",
            set: 1,
            part: 0,
            body: 0,
            region: 0,
        }
    );
}

#[test]
fn a_transformed_reference_needs_its_own_set_too() {
    // A viewer without KHR_texture_transform samples the base set 1, so it
    // must be exported even though the transform overrides it with set 0.
    let mut material = leaf();
    material["pbrMetallicRoughness"]["baseColorTexture"] = json!({
        "index": 0,
        "texCoord": 1,
        "extensions": {"KHR_texture_transform": {"texCoord": 0}}
    });
    assert_eq!(
        export(material, GltfExportOptions::default()).unwrap_err(),
        GltfError::MissingTextureCoordinateSet {
            material: "leaf".into(),
            texture: "pbrMetallicRoughness.baseColorTexture",
            set: 1,
            part: 0,
            body: 0,
            region: 0,
        }
    );
}

#[test]
fn core_slots_take_transforms_and_extension_bodies_keep_extras() {
    let mut material = leaf();
    material["pbrMetallicRoughness"]["baseColorTexture"] = json!({
        "index": 0,
        "extensions": {"KHR_texture_transform": {"scale": [4.0, 4.0]}}
    });
    material["extensions"]["KHR_materials_specular"]["extras"] = json!({"source": "oak"});
    let export = export(material, GltfExportOptions::default()).unwrap();
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    let written = &doc.json()["materials"][0];
    assert_eq!(
        written["pbrMetallicRoughness"]["baseColorTexture"]["extensions"]["KHR_texture_transform"]
            ["scale"],
        json!([4.0, 4.0])
    );
    assert_eq!(
        written["extensions"]["KHR_materials_specular"]["extras"],
        json!({"source": "oak"})
    );
}

#[test]
fn clearcoat_normals_count_without_tangents() {
    let material = json!({"extensions": {"KHR_materials_clearcoat": {
        "clearcoatFactor": 1.0,
        "clearcoatNormalTexture": {"index": 0, "scale": 0.5}
    }}});
    let export = export(material, GltfExportOptions::default()).unwrap();
    assert_eq!(export.stats.normal_maps_without_tangents, 1);
}
