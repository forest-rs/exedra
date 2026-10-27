// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared presentation for millimeter-scale geometry studies.

use exedra::{
    Mesh,
    compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource},
    constructive::ir::Placement3,
    gltf::{
        GltfExportOptions, GltfMesh, GltfNode, GltfScene, MaterialResolver, Texture,
        export_glb_with_materials,
    },
};
use serde_json::{Value, json};
use std::error::Error;

pub(super) struct Exhibit<'a> {
    pub name: &'a str,
    pub mesh: Mesh,
    pub placement: Placement3,
    pub color: [f64; 4],
    pub cut_regions: &'a [u32],
}

pub(super) fn export(
    path: &str,
    title: &str,
    exhibits: Vec<Exhibit<'_>>,
) -> Result<(), Box<dyn Error>> {
    let mut sources = Vec::with_capacity(exhibits.len());
    let mut specs = Vec::with_capacity(exhibits.len());
    for e in exhibits {
        assert!(
            e.mesh.validate_deep().is_empty(),
            "{}: deeply valid mesh",
            e.name
        );
        assert!(
            e.mesh.boundary_loops()?.is_empty(),
            "{}: closed mesh",
            e.name
        );
        sources.push(PartSource::Baked(e.mesh));
        specs.push((e.name, e.placement, e.color, e.cut_regions));
    }
    let inputs: Vec<_> = specs
        .iter()
        .zip(&sources)
        .map(|(e, s)| PartInput::new(e.0, s))
        .collect();
    let compiled = PartCompiler::new().compile_parts(&inputs, &CompilePolicy::default())?;
    let mut root = GltfNode::new(title);
    root.transform = Placement3::from_axes(
        [0.001, 0.0, 0.0],
        [0.0, 0.001, 0.0],
        [0.0, 0.0, 0.001],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    for (i, (name, placement, _, cut_regions)) in specs.iter().enumerate() {
        let id = PartId(u32::try_from(i)?);
        let mut mesh = GltfMesh::new(id, *name, &compiled)?;
        mesh.materials = compiled
            .part(id)
            .expect("compiled exhibit")
            .bodies
            .iter()
            .map(|b| {
                b.regions
                    .iter()
                    .map(|r| {
                        Some(if cut_regions.contains(&r.region) {
                            "cut".to_owned()
                        } else {
                            format!("part-{i}")
                        })
                    })
                    .collect()
            })
            .collect();
        let mut node = GltfNode::new(*name);
        node.parent = Some(0);
        node.transform = *placement;
        node.geometry = Some(mesh.into());
        nodes.push(node);
    }
    let colors: Vec<_> = specs.iter().map(|e| e.2).collect();
    let output = export_glb_with_materials(
        &GltfScene {
            nodes,
            ..GltfScene::default()
        },
        &compiled,
        &Materials { colors: &colors },
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(path, output.bytes)?;
    println!("Wrote {path}");
    Ok(())
}

struct Materials<'a> {
    colors: &'a [[f64; 4]],
}
impl MaterialResolver for Materials<'_> {
    fn resolve(&self, key: &str) -> Option<Value> {
        let color = if key == "cut" {
            [0.65, 0.3, 0.1, 1.0]
        } else {
            *self
                .colors
                .get(key.strip_prefix("part-")?.parse::<usize>().ok()?)?
        };
        let mut material = json!({"name":key,"pbrMetallicRoughness":{"baseColorFactor":color,"metallicFactor":0.35,"roughnessFactor":0.32}});
        if key != "cut" {
            material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":0});
        }
        Some(material)
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        (index == 0).then_some(Texture {
            image: include_bytes!("../../../material_gallery/assets/unit-grid.png"),
            mime_type: "image/png",
            sampler: Some(json!({"wrapS":10497,"wrapT":10497,"magFilter":9729,"minFilter":9987})),
        })
    }
}
