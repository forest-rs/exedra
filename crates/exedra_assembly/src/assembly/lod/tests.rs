// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::compile::{CompilePolicy, PartCompiler, assembly_fingerprint};
use crate::flatten::{FlattenOptions, LodEmission, flatten, flatten_with};
use crate::{InstancePath, RenderItem};
use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder};

/// A slotted prism standing in for one level of a tree.
fn tree(width: u32) -> Recipe {
    let mut b = RecipeBuilder::new();
    let bark = b.material_slot("bark");
    let profile = b.add_profile(builders::rect_from_corner(f64::from(width), 1.0).unwrap());
    let node = b
        .with_material(bark)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 10.0,
            caps: CapMode::Both,
        })
        .unwrap();
    b.finish(node).unwrap()
}

/// An oak with three levels, one placed instance and one set of two.
fn grove() -> (Assembly, [PartId; 3]) {
    let mut asm = Assembly::new();
    let fine = asm.add_recipe_part("oak", tree(16)).unwrap();
    let mid = asm.add_recipe_part("oak-lod1", tree(8)).unwrap();
    let card = asm.add_recipe_part("oak-lod2", tree(3)).unwrap();
    asm.set_part_material(mid, "bark", "grey").unwrap();
    asm.set_part_lods(
        fine,
        vec![
            LodLevel::new(fine, 0.25).with_crossfade(0.05),
            LodLevel::new(mid, 0.05),
            LodLevel::new(card, 0.0),
        ],
    )
    .unwrap();
    let hero = asm
        .add_instance(None, "hero", fine, Placement3::IDENTITY)
        .unwrap();
    asm.bind_material(hero, "bark", "mossy").unwrap();
    asm.add_placement_set(
        None,
        "grove",
        fine,
        vec![Placement3::translate(5.0, 0.0, 0.0); 2],
    )
    .unwrap();
    (asm, [fine, mid, card])
}

fn invalid(asm: &mut Assembly, part: PartId, levels: Vec<LodLevel>) -> &'static str {
    match asm.set_part_lods(part, levels) {
        Err(AssemblyError::InvalidLods { reason, .. }) => reason,
        other => panic!("expected an invalid chain, got {other:?}"),
    }
}

#[test]
fn chains_are_validated() {
    let mut asm = Assembly::new();
    let a = asm.add_recipe_part("a", tree(8)).unwrap();
    let b = asm.add_recipe_part("b", tree(4)).unwrap();
    let c = asm.add_recipe_part("c", tree(3)).unwrap();
    assert_eq!(
        invalid(&mut asm, a, vec![LodLevel::new(b, 0.5)]),
        "level 0 must be the part itself"
    );
    assert_eq!(
        invalid(
            &mut asm,
            a,
            vec![LodLevel::new(a, 0.1), LodLevel::new(b, 0.2)]
        ),
        "thresholds must strictly decrease"
    );
    assert_eq!(
        invalid(
            &mut asm,
            a,
            vec![LodLevel::new(a, 0.5), LodLevel::new(a, 0.1)]
        ),
        "a part appears twice in the chain"
    );
    assert_eq!(
        invalid(
            &mut asm,
            a,
            vec![
                LodLevel::new(a, 0.2).with_crossfade(0.15),
                LodLevel::new(b, 0.1)
            ]
        ),
        "a crossfade band reaches past the next threshold"
    );
    assert_eq!(
        invalid(&mut asm, a, vec![LodLevel::new(a, f32::NAN)]),
        "thresholds must be finite and non-negative"
    );
    asm.set_part_lods(a, vec![LodLevel::new(a, 0.5), LodLevel::new(b, 0.1)])
        .unwrap();
    // No nesting in either direction.
    assert_eq!(
        invalid(&mut asm, b, vec![LodLevel::new(b, 0.5)]),
        "the part is a lower level of another chain"
    );
    assert_eq!(
        invalid(
            &mut asm,
            c,
            vec![LodLevel::new(c, 0.5), LodLevel::new(a, 0.1)]
        ),
        "a lower-level part has its own chain"
    );
    // An empty chain clears.
    asm.set_part_lods(a, vec![]).unwrap();
    assert!(asm.part(a).unwrap().lods().is_empty());
}

#[test]
fn flatten_emits_the_base_level_unless_asked_for_all() {
    let (asm, [fine, mid, card]) = grove();
    let compiled = PartCompiler::new()
        .compile_parts(&asm, &CompilePolicy::default())
        .unwrap();

    let base = flatten(&asm, &compiled);
    assert_eq!(base.items.len(), 1);
    assert_eq!(base.batches.len(), 1);
    assert_eq!(base.items[0].part, fine);
    let tag = base.items[0].lod.expect("chained part is tagged");
    assert_eq!((tag.level, tag.levels), (0, 3));
    assert_eq!((tag.min_coverage, tag.max_coverage), (0.25, None));
    assert_eq!(tag.crossfade, 0.05);

    let all = flatten_with(
        &asm,
        &compiled,
        &FlattenOptions::default().with_lods(LodEmission::AllLevels),
    );
    let parts: Vec<_> = all.items.iter().map(|item| item.part).collect();
    assert_eq!(parts, [fine, mid, card]);
    assert!(
        all.items
            .iter()
            .all(|item| item.path == InstancePath::from_segments(&["hero"]))
    );
    let mid_tag = all.items[1].lod.unwrap();
    assert_eq!(
        (mid_tag.level, mid_tag.min_coverage, mid_tag.max_coverage),
        (1, 0.05, Some(0.25))
    );
    assert_eq!(all.batches.len(), 3);
    assert_eq!(all.batches[2].lod.unwrap().level, 2);
    assert_eq!(all.batches[2].len(), 2);
}

#[test]
fn bindings_reach_lower_levels_by_slot_name() {
    let (asm, _) = grove();
    let compiled = PartCompiler::new()
        .compile_parts(&asm, &CompilePolicy::default())
        .unwrap();
    let all = flatten_with(
        &asm,
        &compiled,
        &FlattenOptions::default().with_lods(LodEmission::AllLevels),
    );
    let bark = |item: &RenderItem| {
        item.regions
            .iter()
            .map(|r| r.material.clone())
            .collect::<Vec<_>>()
    };
    // The instance binding applies to every level.
    for item in &all.items {
        assert!(bark(item).iter().all(|m| m.as_deref() == Some("mossy")));
    }
    // The unbound set falls back to each level's own part default.
    let batch_materials: Vec<_> = all
        .batches
        .iter()
        .map(|b| b.regions[0].material.clone())
        .collect();
    assert_eq!(batch_materials, [None, Some("grey".into()), None]);
}

#[test]
fn resolved_level_material_is_the_flatten_rule() {
    let (mut asm, [fine, mid, card]) = grove();
    let hero = asm.roots()[0];
    let slot = |asm: &Assembly, part: PartId| asm.part(part).unwrap().slot_index("bark").unwrap();
    for level in [fine, mid, card] {
        assert_eq!(
            asm.resolved_level_material(Occurrence::Instance(hero), level, slot(&asm, level)),
            Some("mossy")
        );
    }
    fn set_bark(asm: &Assembly, level: PartId) -> Option<&str> {
        let slot = asm.part(level).unwrap().slot_index("bark").unwrap();
        asm.resolved_level_material(Occurrence::PlacementSet(PlacementSetId(0)), level, slot)
    }
    assert_eq!(set_bark(&asm, fine), None);
    assert_eq!(set_bark(&asm, mid), Some("grey"));
    asm.bind_placement_material(PlacementSetId(0), "bark", "wet")
        .unwrap();
    assert_eq!(set_bark(&asm, card), Some("wet"));
    // For the occurrence's own part it is the ordinary resolution.
    assert_eq!(
        asm.resolved_level_material(Occurrence::Instance(hero), fine, slot(&asm, fine)),
        asm.resolved_material(hero, slot(&asm, fine))
    );
    assert_eq!(
        asm.resolved_level_material(Occurrence::Instance(InstanceId(99)), fine, slot(&asm, fine)),
        None
    );
}

#[test]
fn owner_defaults_cascade_to_lower_levels() {
    let (mut asm, [fine, _, _]) = grove();
    asm.set_part_material(fine, "bark", "brown").unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&asm, &CompilePolicy::default())
        .unwrap();
    let all = flatten_with(
        &asm,
        &compiled,
        &FlattenOptions::default().with_lods(LodEmission::AllLevels),
    );
    // The owning part's default reaches every level before a level's own
    // default ("grey" on the middle level).
    let batch_materials: Vec<_> = all
        .batches
        .iter()
        .map(|b| b.regions[0].material.clone())
        .collect();
    assert_eq!(
        batch_materials,
        vec![Some(alloc::string::String::from("brown")); 3]
    );
}

#[test]
fn chains_may_share_a_lower_level() {
    let (mut asm, [_, _, card]) = grove();
    let birch = asm.add_recipe_part("birch", tree(12)).unwrap();
    asm.set_part_lods(
        birch,
        vec![LodLevel::new(birch, 0.1), LodLevel::new(card, 0.0)],
    )
    .unwrap();
    assert_eq!(asm.part(birch).unwrap().lods()[1].part, card);
}

#[test]
fn crossfade_bands_may_meet_the_next_threshold_exactly() {
    let mut asm = Assembly::new();
    let a = asm.add_recipe_part("a", tree(8)).unwrap();
    let b = asm.add_recipe_part("b", tree(4)).unwrap();
    asm.set_part_lods(
        a,
        vec![
            LodLevel::new(a, 0.3).with_crossfade(0.1),
            LodLevel::new(b, 0.2),
        ],
    )
    .unwrap();
}

#[test]
fn chains_are_identity_and_survive_interchange_and_append() {
    let (asm, [fine, mid, _]) = grove();
    let mut without = asm.clone();
    without.set_part_lods(fine, vec![]).unwrap();
    assert_ne!(assembly_fingerprint(&asm), assembly_fingerprint(&without));

    #[cfg(feature = "serde")]
    {
        let dto = crate::interchange::to_dto(&asm);
        assert_eq!(dto.version, crate::interchange::VERSION);
        assert_eq!(dto.parts[fine.0 as usize].lods.len(), 3);
        assert!(crate::interchange::round_trips(&asm));
    }

    // Appending an instance of the chained part brings its lower levels.
    let mut dest = Assembly::new();
    let map = dest
        .append_selected(None, &asm, "east", Placement3::IDENTITY, |_, inst| {
            inst.key() == "hero"
        })
        .unwrap();
    let copied = map.part(fine).unwrap();
    let chain = dest.part(copied).unwrap().lods();
    assert_eq!(chain.len(), 3);
    assert_eq!(chain[0].part, copied);
    assert_eq!(chain[1].part, map.part(mid).unwrap());
    assert_eq!(dest.part(chain[1].part).unwrap().key(), "east-oak-lod1");
}
