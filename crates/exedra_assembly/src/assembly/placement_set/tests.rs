// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::compile::assembly_fingerprint;
use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, NodeKind, Recipe, RecipeBuilder};

fn slotted_recipe() -> Recipe {
    let mut b = RecipeBuilder::new();
    let _ = b.material_slot("bark");
    let profile = b.add_profile(builders::rect_from_corner(2.0, 2.0).unwrap());
    let node = b
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 4.0,
            caps: CapMode::Both,
        })
        .unwrap();
    b.finish(node).unwrap()
}

fn row(count: usize) -> Vec<Placement3> {
    (0..count)
        .map(|i| Placement3::translate(i as f64 * 10.0, 0.0, 0.0))
        .collect()
}

fn forest() -> (Assembly, PartId, InstanceId, PlacementSetId) {
    let mut asm = Assembly::new();
    let part = asm.add_recipe_part("oak", slotted_recipe()).unwrap();
    let frame = asm
        .add_frame(None, "forest", Placement3::translate(0.0, 100.0, 0.0))
        .unwrap();
    let set = asm
        .add_placement_set(Some(frame), "oaks", part, row(3))
        .unwrap();
    (asm, part, frame, set)
}

#[test]
fn sets_share_the_sibling_key_namespace() {
    let (mut asm, part, frame, _) = forest();
    assert_eq!(
        asm.add_instance(Some(frame), "oaks", part, Placement3::IDENTITY),
        Err(AssemblyError::DuplicateChildKey {
            parent: Some(frame),
            key: "oaks".into(),
        })
    );
    assert!(
        asm.add_instance(Some(frame), "lone", part, Placement3::IDENTITY)
            .is_ok()
    );
    assert_eq!(
        asm.add_placement_set(Some(frame), "lone", part, row(1)),
        Err(AssemblyError::DuplicateChildKey {
            parent: Some(frame),
            key: "lone".into(),
        })
    );
    // The same key is free under another parent.
    assert!(asm.add_placement_set(None, "oaks", part, row(1)).is_ok());
}

#[test]
fn keys_reserve_the_placement_separator() {
    let (mut asm, part, _, _) = forest();
    assert_eq!(
        asm.add_instance(None, "oak#1", part, Placement3::IDENTITY),
        Err(AssemblyError::InvalidKey("oak#1".into()))
    );
    assert_eq!(
        asm.add_placement_set(None, "a/b", part, row(1)),
        Err(AssemblyError::InvalidKey("a/b".into()))
    );
}

#[test]
fn per_placement_data_matches_the_placement_count() {
    let (mut asm, _, _, set) = forest();
    assert_eq!(
        asm.set_placement_seeds(set, vec![1, 2]),
        Err(AssemblyError::PlacementDataLength {
            expected: 3,
            found: 2
        })
    );
    assert_eq!(
        asm.set_placement_tints(set, vec![[1.0, f32::NAN, 1.0, 1.0]; 3]),
        Err(AssemblyError::NonFiniteTint)
    );
    asm.set_placement_seeds(set, vec![7, 8, 9]).unwrap();
    asm.set_placement_tints(set, vec![[1.0; 4]; 3]).unwrap();
    let stored = asm.placement_set(set).unwrap();
    assert_eq!(stored.seeds(), Some(&[7, 8, 9][..]));
    assert_eq!(stored.tints().map(<[_]>::len), Some(3));
    assert_eq!(
        asm.set_placement_seeds(PlacementSetId(9), vec![]),
        Err(AssemblyError::UnknownPlacementSet(PlacementSetId(9)))
    );
}

#[test]
fn placements_are_addressed_by_set_path_and_index() {
    let (asm, _, _, set) = forest();
    let path = PlacementPath {
        set: asm.placement_set_path(set).unwrap(),
        index: 2,
    };
    assert_eq!(alloc::format!("{path}"), "forest/oaks#2");
    assert_eq!(asm.resolve_placement(&path), Some((set, 2)));
    let past_end = PlacementPath {
        index: 3,
        ..path.clone()
    };
    assert_eq!(asm.resolve_placement(&past_end), None);
    // A set is not an instance.
    assert_eq!(asm.resolve_path(&path.set), None);
}

#[test]
fn set_bindings_fall_back_to_part_defaults() {
    let (mut asm, part, _, set) = forest();
    asm.set_part_material(part, "bark", "grey").unwrap();
    let bark = asm.part(part).unwrap().slot_index("bark").unwrap();
    assert_eq!(asm.resolved_placement_material(set, bark), Some("grey"));
    asm.bind_placement_material(set, "bark", "mossy").unwrap();
    assert_eq!(asm.resolved_placement_material(set, bark), Some("mossy"));
    assert!(matches!(
        asm.bind_placement_material(set, "leaf", "green"),
        Err(AssemblyError::UnknownSlot { .. })
    ));
}

#[test]
fn fingerprints_cover_set_contents() {
    let (mut asm, _, _, set) = forest();
    let base = assembly_fingerprint(&asm);
    asm.set_placement_seeds(set, vec![1, 2, 3]).unwrap();
    let seeded = assembly_fingerprint(&asm);
    assert_ne!(base, seeded);
    asm.set_placement_set_metadata(set, "species", "quercus")
        .unwrap();
    assert_ne!(seeded, assembly_fingerprint(&asm));
}

#[test]
fn append_copies_sets_with_their_parents_and_prefixes_root_sets() {
    let (mut source, part, _, set) = forest();
    source.set_placement_seeds(set, vec![1, 2, 3]).unwrap();
    let root_set = source
        .add_placement_set(None, "shrubs", part, row(2))
        .unwrap();

    let mut dest = Assembly::new();
    let map = dest
        .append(None, &source, "west", Placement3::translate(0.0, 0.0, 5.0))
        .unwrap();
    let copied = map.placement_set(set).unwrap();
    let copied_root = map.placement_set(root_set).unwrap();

    let nested = dest.placement_set(copied).unwrap();
    assert_eq!(nested.seeds(), Some(&[1, 2, 3][..]));
    assert_eq!(
        nested.placements(),
        row(3).as_slice(),
        "nested sets keep locals"
    );
    assert_eq!(
        dest.placement_set_path(copied).unwrap(),
        InstancePath::from_segments(&["west-forest", "oaks"])
    );

    let root = dest.placement_set(copied_root).unwrap();
    assert_eq!(root.key(), "west-shrubs");
    assert_eq!(root.placements()[1].rows[0][3], 10.0);
    assert_eq!(root.placements()[1].rows[2][3], 5.0, "root sets compose");
    assert_eq!(dest.root_placement_sets(), &[copied_root]);

    // The copied keys are indexed: re-appending collides.
    assert!(matches!(
        dest.append(None, &source, "west", Placement3::IDENTITY),
        Err(AssemblyError::DuplicatePartKey(_) | AssemblyError::DuplicateChildKey { .. })
    ));
    let path = PlacementPath {
        set: InstancePath::from_segments(&["west-shrubs"]),
        index: 1,
    };
    assert_eq!(dest.resolve_placement(&path), Some((copied_root, 1)));
}

#[test]
fn empty_sets_are_refused() {
    let (mut asm, part, _, _) = forest();
    assert_eq!(
        asm.add_placement_set(None, "none", part, Vec::new()),
        Err(AssemblyError::EmptyPlacementSet)
    );
}

#[test]
fn batches_share_placements_seeds_and_tints() {
    use crate::compile::{CompilePolicy, PartCompiler};
    let (mut asm, _, _, set) = forest();
    asm.set_placement_seeds(set, vec![7, 8, 9]).unwrap();
    asm.set_placement_tints(set, vec![[0.5, 0.25, 1.0, 1.0]; 3])
        .unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&asm, &CompilePolicy::default())
        .unwrap();
    let list = crate::flatten::flatten(&asm, &compiled);
    let stored = asm.placement_set(set).unwrap();
    assert!(!list.batches.is_empty());
    for batch in &list.batches {
        assert_eq!(batch.world.len(), 3);
        assert_eq!(batch.seeds.as_deref(), Some(&[7, 8, 9][..]));
        assert_eq!(
            batch.tints.as_deref(),
            Some(&[[0.5, 0.25, 1.0, 1.0]; 3][..])
        );
        // Shared with the set, not copied per batch.
        assert!(core::ptr::eq(
            batch.seeds.as_deref().unwrap().as_ptr(),
            stored.seeds().unwrap().as_ptr()
        ));
    }
    let unseeded = {
        let (asm, _, _, _) = forest();
        let compiled = PartCompiler::new()
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        crate::flatten::flatten(&asm, &compiled)
    };
    assert!(
        unseeded
            .batches
            .iter()
            .all(|b| b.seeds.is_none() && b.tints.is_none())
    );
}
