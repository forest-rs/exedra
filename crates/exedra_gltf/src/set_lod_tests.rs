// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_assembly::{CompilePolicy, LodLevel, PartCompiler};
use exedra_mesh::{BuildParams, Mesh};

fn triangle(size: f32) -> Mesh {
    Mesh::from_indexed_triangles(
        &[[0.0, 0.0, 0.0], [size, 0.0, 0.0], [0.0, size, 0.0]],
        &[[0, 1, 2]],
        &BuildParams::default(),
    )
    .unwrap()
}

fn glb(assembly: &Assembly, options: GltfExportOptions<'_>) -> GlbExport {
    let compiled = PartCompiler::new()
        .compile_parts(assembly, &CompilePolicy::default())
        .unwrap();
    export_glb_with_options(assembly, &compiled, options).unwrap()
}

fn gpu() -> GltfExportOptions<'static> {
    GltfExportOptions::default().with_instancing(GltfInstancing::GpuInstancing)
}

fn msft_lod() -> GltfExportOptions<'static> {
    GltfExportOptions::default().with_lods(GltfLods::MsftLod)
}

/// A scatter of three trees under a frame; the second placement mirrors.
fn scatter() -> Assembly {
    let mut assembly = Assembly::new();
    let tree = assembly.add_baked_part("tree", triangle(1.0), &[]).unwrap();
    let frame = assembly
        .add_frame(None, "forest", Placement3::translate(10.0, 0.0, 0.0))
        .unwrap();
    let mut mirrored = Placement3::translate(3.0, 0.0, 0.0);
    mirrored.rows[0][0] = -1.0;
    let set = assembly
        .add_placement_set(
            Some(frame),
            "oaks",
            tree,
            vec![
                Placement3::IDENTITY,
                mirrored,
                Placement3::translate(0.0, 5.0, 0.0),
            ],
        )
        .unwrap();
    assembly
        .set_placement_seeds(set, vec![1, u64::MAX, 0x0001_0002_0003_0004])
        .unwrap();
    assembly
        .set_placement_tints(
            set,
            vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ],
        )
        .unwrap();
    assembly
}

#[test]
fn placement_sets_become_addressed_nodes_by_default() {
    let export = glb(&scatter(), GltfExportOptions::default());
    assert_eq!(export.stats.placement_nodes, 3);
    assert_eq!(export.stats.batched_placements, 0);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        doc.node_names(),
        ["forest", "forest/oaks#0", "forest/oaks#1", "forest/oaks#2"]
    );
    let j = doc.json();
    assert_eq!(j["nodes"][0]["children"], json!([1, 2, 3]));
    assert!(j.get("extensionsUsed").is_none());
    let extras = doc.node_extras("forest/oaks#1").unwrap();
    assert_eq!(extras["instancePath"], json!("forest/oaks#1"));
    assert_eq!(extras["partKey"], json!("tree"));
    // Seeds are decimal strings: JSON numbers cannot hold every u64.
    assert_eq!(extras["seed"], json!(u64::MAX.to_string()));
    assert_eq!(extras["tint"], json!([0.0, 1.0, 0.0, 1.0]));
    // The identity placement writes no matrix; the others keep theirs.
    assert!(j["nodes"][1].get("matrix").is_none());
    assert_eq!(j["nodes"][3]["matrix"][13], json!(5.0));
    // Every placement node draws the one shared mesh.
    for node in 1..=3 {
        assert_eq!(j["nodes"][node]["mesh"], json!(0));
    }
}

#[test]
fn placement_sets_batch_with_gpu_instancing() {
    let export = glb(&scatter(), gpu());
    assert_eq!(export.stats.batched_placements, 2);
    assert_eq!(export.stats.unbatched_mirrored_placements, 1);
    assert_eq!(export.stats.placement_nodes, 1);
    assert_eq!(export.stats.instanced_nodes, 1);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        doc.node_names(),
        ["forest", "forest/oaks [2 placements]", "forest/oaks#1"]
    );
    let j = doc.json();
    assert_eq!(j["extensionsRequired"], json!(["EXT_mesh_gpu_instancing"]));
    let name = "forest/oaks [2 placements]";
    let extras = doc.node_extras(name).unwrap();
    assert_eq!(extras["setPath"], json!("forest/oaks"));
    assert_eq!(extras["placementCount"], json!(2));
    assert_eq!(extras["placementIndices"], json!([0, 2]));
    assert_eq!(
        doc.instancing_components(name, "TRANSLATION").unwrap(),
        [0.0, 0.0, 0.0, 0.0, 5.0, 0.0]
    );
    // Seeds as four 16-bit words, least significant first.
    assert_eq!(
        doc.instancing_components(name, "_SEED").unwrap(),
        [1.0, 0.0, 0.0, 0.0, 4.0, 3.0, 2.0, 1.0]
    );
    assert_eq!(
        doc.instancing_components(name, "_TINT").unwrap(),
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0]
    );
    // The mirrored placement keeps its own addressed node and exact seed.
    assert_eq!(
        doc.node_extras("forest/oaks#1").unwrap()["seed"],
        json!(u64::MAX.to_string())
    );
}

#[test]
fn a_whole_set_batches_without_an_index_list() {
    let mut assembly = Assembly::new();
    let tree = assembly.add_baked_part("tree", triangle(1.0), &[]).unwrap();
    assembly
        .add_placement_set(
            None,
            "one",
            tree,
            vec![Placement3::translate(1.0, 2.0, 3.0)],
        )
        .unwrap();
    let export = glb(&assembly, gpu());
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(doc.node_names(), ["one [1 placements]"]);
    let extras = doc.node_extras("one [1 placements]").unwrap();
    assert_eq!(extras["placementCount"], json!(1));
    assert!(!extras.contains_key("placementIndices"));
    assert!(
        doc.instancing_components("one [1 placements]", "_SEED")
            .is_none()
    );
}

#[test]
fn instances_that_own_placement_sets_keep_their_node() {
    let mut assembly = Assembly::new();
    let tree = assembly.add_baked_part("tree", triangle(1.0), &[]).unwrap();
    let a = assembly
        .add_instance(None, "a", tree, Placement3::IDENTITY)
        .unwrap();
    assembly
        .add_instance(None, "b", tree, Placement3::translate(2.0, 0.0, 0.0))
        .unwrap();
    assembly
        .add_placement_set(Some(a), "seedlings", tree, vec![Placement3::IDENTITY; 2])
        .unwrap();
    let export = glb(&assembly, gpu());
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    // `a` owns a set, so only `b` could batch, alone: no instance batch.
    assert_eq!(export.stats.batched_instances, 0);
    assert_eq!(doc.node_names(), ["a", "b", "a/seedlings [2 placements]"]);
    assert_eq!(doc.json()["nodes"][0]["children"], json!([2]));
}

#[test]
fn set_bindings_resolve_set_materials() {
    let mut assembly = Assembly::new();
    let tree = assembly
        .add_baked_part("tree", triangle(1.0), &["bark"])
        .unwrap();
    assembly.bind_region_slot(tree, 0, "bark").unwrap();
    assembly.set_part_material(tree, "bark", "grey").unwrap();
    let set = assembly
        .add_placement_set(None, "oaks", tree, vec![Placement3::IDENTITY; 2])
        .unwrap();
    assembly
        .bind_placement_material(set, "bark", "brown")
        .unwrap();
    for options in [GltfExportOptions::default(), gpu()] {
        let export = glb(&assembly, options);
        let doc = GlbDocument::parse(&export.bytes).unwrap();
        assert_eq!(doc.material_names(), ["brown"]);
    }
}

/// A `tree` part with a two-level chain onto a coarser `card` part that names
/// the same `bark` slot.
fn chained() -> (Assembly, PartId) {
    let mut assembly = Assembly::new();
    let tree = assembly
        .add_baked_part("tree", triangle(1.0), &["bark"])
        .unwrap();
    let card = assembly
        .add_baked_part("card", triangle(2.0), &["bark"])
        .unwrap();
    for part in [tree, card] {
        assembly.bind_region_slot(part, 0, "bark").unwrap();
    }
    assembly.set_part_material(tree, "bark", "grey").unwrap();
    assembly
        .set_part_material(card, "bark", "card-default")
        .unwrap();
    assembly
        .set_part_lods(
            tree,
            vec![
                LodLevel::new(tree, 0.25).with_crossfade(0.05),
                LodLevel::new(card, 0.0),
            ],
        )
        .unwrap();
    (assembly, tree)
}

#[test]
fn lod_chains_write_only_the_base_level_by_default() {
    let (mut assembly, tree) = chained();
    assembly
        .add_instance(None, "oak", tree, Placement3::IDENTITY)
        .unwrap();
    let export = glb(&assembly, GltfExportOptions::default());
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(doc.node_names(), ["oak"]);
    assert!(doc.json().get("extensionsUsed").is_none());
    assert_eq!(export.stats.lod_nodes, 0);
}

#[test]
fn lod_chains_write_msft_lod_on_a_geometry_child() {
    let (mut assembly, tree) = chained();
    let oak = assembly
        .add_instance(None, "oak", tree, Placement3::translate(1.0, 0.0, 0.0))
        .unwrap();
    assembly.bind_material(oak, "bark", "brown").unwrap();
    let export = glb(&assembly, msft_lod());
    assert_eq!(export.stats.lod_nodes, 1);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(doc.node_names(), ["oak", "oak [lod 0]", "oak [lod 1]"]);
    assert_eq!(doc.lod_node_names("oak [lod 0]").unwrap(), ["oak [lod 1]"]);
    let j = doc.json();
    // The logical node keeps its transform; LOD nodes have none.
    assert!(j["nodes"][0].get("matrix").is_some());
    assert_eq!(j["nodes"][0]["children"], json!([1]));
    assert!(j["nodes"][1].get("matrix").is_none());
    assert!(j["nodes"][2].get("matrix").is_none());
    // The lower level is reachable only through MSFT_lod.
    assert_eq!(j["scenes"][0]["nodes"], json!([0]));
    assert!(j["nodes"][1].get("children").is_none());
    let extras = doc.node_extras("oak [lod 0]").unwrap();
    assert_eq!(extras["MSFT_screencoverage"], json!([0.25, 0.0]));
    assert_eq!(extras["exedraLodCrossfade"], json!([0.05, 0.0]));
    assert_eq!(
        doc.node_extras("oak [lod 1]").unwrap()["partKey"],
        json!("card")
    );
    // Used, not required: viewers without it draw level 0.
    assert_eq!(j["extensionsUsed"], json!(["MSFT_lod"]));
    assert!(j.get("extensionsRequired").is_none());
    // The instance binding reaches the card through the shared slot name.
    assert_eq!(doc.material_names(), ["brown"]);
    assert_eq!(doc.triangle_count(), 2);
}

#[test]
fn instanced_lod_levels_share_instance_transforms() {
    let (mut assembly, tree) = chained();
    for (key, x) in [("a", 0.0), ("b", 4.0)] {
        assembly
            .add_instance(None, key, tree, Placement3::translate(x, 0.0, 0.0))
            .unwrap();
    }
    let options = gpu().with_lods(GltfLods::MsftLod);
    let export = glb(&assembly, options);
    assert_eq!(export.stats.batched_instances, 2);
    assert_eq!(export.stats.instanced_nodes, 2);
    assert_eq!(export.stats.lod_nodes, 1);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    // Every level names its bodies, so no level group shares a name with
    // its own instanced body.
    assert_eq!(
        doc.node_names(),
        [
            "tree [2 instances]",
            "tree [2 instances] [lod 0] [body 0]",
            "tree [2 instances] [lod 1]",
            "tree [2 instances] [lod 1] [body 0]",
        ]
    );
    let j = doc.json();
    let group = &j["nodes"][0];
    assert_eq!(group["children"], json!([1]));
    assert!(group.get("mesh").is_none());
    assert_eq!(
        doc.lod_node_names("tree [2 instances]").unwrap(),
        ["tree [2 instances] [lod 1]"]
    );
    // Level 0 and level 1 instanced nodes reuse the same accessors, read by
    // name.
    let translations = |name: &str| doc.instancing_components(name, "TRANSLATION").unwrap();
    assert_eq!(
        translations("tree [2 instances] [lod 0] [body 0]"),
        translations("tree [2 instances] [lod 1] [body 0]")
    );
    assert!(
        doc.instancing_components("tree [2 instances] [lod 1]", "TRANSLATION")
            .is_none()
    );
    assert_eq!(j["nodes"][2]["children"], json!([3]));
    let used = j["extensionsUsed"].as_array().unwrap();
    assert_eq!(used, &[json!("EXT_mesh_gpu_instancing"), json!("MSFT_lod")]);
    assert_eq!(j["extensionsRequired"], json!(["EXT_mesh_gpu_instancing"]));
}

#[test]
fn set_placements_write_their_lod_chains() {
    let (mut assembly, tree) = chained();
    assembly
        .add_placement_set(None, "grove", tree, vec![Placement3::IDENTITY; 2])
        .unwrap();
    let export = glb(&assembly, msft_lod());
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(export.stats.lod_nodes, 2);
    assert_eq!(
        doc.lod_node_names("grove#1 [lod 0]").unwrap(),
        ["grove#1 [lod 1]"]
    );
    let batched = glb(&assembly, gpu().with_lods(GltfLods::MsftLod));
    let doc = GlbDocument::parse(&batched.bytes).unwrap();
    assert_eq!(
        doc.lod_node_names("grove [2 placements]").unwrap(),
        ["grove [2 placements] [lod 1]"]
    );
}

#[test]
fn exports_with_sets_and_lods_are_deterministic() {
    let (mut assembly, tree) = chained();
    assembly
        .add_placement_set(None, "grove", tree, vec![Placement3::IDENTITY; 3])
        .unwrap();
    let options = gpu().with_lods(GltfLods::MsftLod);
    assert_eq!(glb(&assembly, options).bytes, glb(&assembly, options).bytes);
}

/// A part of two separate boxes, so it compiles to two bodies.
fn two_body_part(assembly: &mut Assembly, key: &str) -> PartId {
    use exedra_constructive::ir::{NodeKind, PrimitiveSpec, RecipeBuilder};
    let mut builder = RecipeBuilder::new();
    let children = (0..2)
        .map(|i| {
            builder
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [0.5, 0.1, 0.5],
                    },
                    placement: Placement3::translate(0.0, f64::from(i) * 0.2, 0.0),
                })
                .unwrap()
        })
        .collect();
    let root = builder.add(NodeKind::Group { children }).unwrap();
    assembly
        .add_recipe_part(key, builder.finish(root).unwrap())
        .unwrap()
}

fn two_body_chain() -> Assembly {
    let mut assembly = Assembly::new();
    let pair = two_body_part(&mut assembly, "pair");
    let card = assembly.add_baked_part("card", triangle(1.0), &[]).unwrap();
    assembly
        .set_part_lods(
            pair,
            vec![LodLevel::new(pair, 0.25), LodLevel::new(card, 0.0)],
        )
        .unwrap();
    for (key, x) in [("a", 0.0), ("b", 2.0)] {
        assembly
            .add_instance(None, key, pair, Placement3::translate(x, 0.0, 0.0))
            .unwrap();
    }
    assembly
}

#[test]
fn multi_body_lod_chains_name_every_body() {
    let nodes = glb(&two_body_chain(), msft_lod());
    let doc = GlbDocument::parse(&nodes.bytes).unwrap();
    assert_eq!(
        doc.node_names(),
        [
            "a",
            "b",
            "a [lod 0]",
            "a [lod 0] [body 0]",
            "a [lod 0] [body 1]",
            "a [lod 1]",
            "b [lod 0]",
            "b [lod 0] [body 0]",
            "b [lod 0] [body 1]",
            "b [lod 1]",
        ]
    );
    assert_eq!(doc.lod_node_names("a [lod 0]").unwrap(), ["a [lod 1]"]);

    let batched = glb(&two_body_chain(), gpu().with_lods(GltfLods::MsftLod));
    let doc = GlbDocument::parse(&batched.bytes).unwrap();
    assert_eq!(
        doc.node_names(),
        [
            "pair [2 instances]",
            "pair [2 instances] [lod 0] [body 0]",
            "pair [2 instances] [lod 0] [body 1]",
            "pair [2 instances] [lod 1]",
            "pair [2 instances] [lod 1] [body 0]",
        ]
    );
    for body in ["[lod 0] [body 0]", "[lod 0] [body 1]", "[lod 1] [body 0]"] {
        assert!(
            doc.instancing_components(&format!("pair [2 instances] {body}"), "TRANSLATION")
                .is_some()
        );
    }
}

#[test]
fn sheared_set_placements_are_counted_in_both_modes() {
    let mut assembly = Assembly::new();
    let tree = assembly.add_baked_part("tree", triangle(1.0), &[]).unwrap();
    let mut sheared = Placement3::translate(2.0, 0.0, 0.0);
    sheared.rows[0][1] = 0.5;
    assembly
        .add_placement_set(None, "oaks", tree, vec![Placement3::IDENTITY, sheared])
        .unwrap();
    let nodes = glb(&assembly, GltfExportOptions::default());
    assert_eq!(nodes.stats.non_trs_matrices, 1);
    assert_eq!(nodes.stats.unbatched_sheared_placements, 0);
    let batched = glb(&assembly, gpu());
    assert_eq!(batched.stats.non_trs_matrices, 1);
    assert_eq!(batched.stats.unbatched_sheared_placements, 1);
    assert_eq!(batched.stats.batched_placements, 1);
}

#[test]
fn sets_sit_under_the_conversion_root() {
    for options in [
        GltfExportOptions::z_up_to_y_up(),
        GltfExportOptions::z_up_to_y_up().with_instancing(GltfInstancing::GpuInstancing),
    ] {
        let export = glb(&scatter(), options);
        let doc = GlbDocument::parse(&export.bytes).unwrap();
        let j = doc.json();
        let scene = j["scenes"][0]["nodes"].as_array().unwrap();
        assert_eq!(scene.len(), 1);
        let root = &j["nodes"][usize::try_from(scene[0].as_u64().unwrap()).unwrap()];
        assert_eq!(root["name"], json!("Exedra Z-up to glTF Y-up"));
        // The frame holding the set (and, instanced, the batch under it) is
        // reached through the root.
        assert_eq!(root["children"], json!([0]));
    }
}

#[test]
fn placements_without_seeds_or_tints_write_neither() {
    let mut assembly = Assembly::new();
    let tree = assembly.add_baked_part("tree", triangle(1.0), &[]).unwrap();
    assembly
        .add_placement_set(
            None,
            "oaks",
            tree,
            vec![Placement3::IDENTITY, Placement3::translate(2.0, 0.0, 0.0)],
        )
        .unwrap();
    let export = glb(&assembly, GltfExportOptions::default());
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    let extras = doc.node_extras("oaks#1").unwrap();
    assert!(extras.get("seed").is_none());
    assert!(extras.get("tint").is_none());
    let batched = glb(&assembly, gpu());
    let doc = GlbDocument::parse(&batched.bytes).unwrap();
    assert!(
        doc.instancing_components("oaks [2 placements]", "_SEED")
            .is_none()
    );
    assert!(
        doc.instancing_components("oaks [2 placements]", "_TINT")
            .is_none()
    );
}

#[test]
fn tint_extras_use_shortest_decimals() {
    let mut assembly = Assembly::new();
    let tree = assembly.add_baked_part("tree", triangle(1.0), &[]).unwrap();
    let set = assembly
        .add_placement_set(None, "oaks", tree, vec![Placement3::IDENTITY])
        .unwrap();
    assembly
        .set_placement_tints(set, vec![[0.3, 0.1, 0.0, 1.0]])
        .unwrap();
    let export = glb(&assembly, GltfExportOptions::default());
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        doc.node_extras("oaks#0").unwrap()["tint"],
        json!([0.3, 0.1, 0.0, 1.0])
    );
}
