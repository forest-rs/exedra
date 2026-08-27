// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! End-to-end Addressable tour over the Basilica assembly.

use std::string::String;

use addressable::{
    Endpoint, Guard, Locator, Pinned, Query, Resolution, SpaceId, Transaction, TransactionMode,
};
use basilica_ruin::{BasilicaParams, build_basilica_assembly, names};
use exedra_assembly::{
    AssemblyAxis, AssemblyPredicate, AssemblySpace, AssemblyView, BindMaterial, EditCapability,
    MaterialReason, MaterialSlot,
};

/// Observable milestones from [`run_tour`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tour {
    /// Canonical address selected by the query.
    pub address: String,
    /// Stable part referent selected at that address.
    pub part: String,
    /// Effective material before the edit.
    pub material_before: String,
    /// Why the original material won.
    pub reason_before: MaterialReason,
    /// Number of changes predicted by the dry run.
    pub preview_changes: usize,
    /// Effective material after the applied transaction.
    pub material_after: String,
    /// Wrapper revision after the applied transaction.
    pub revision_after: u64,
}

/// Runs resolution, query, handle, explanation, and guarded-edit workflows.
///
/// The function uses stable Basilica vocabulary rather than runtime Exedra
/// indices. Assertions are part of the tour: if any Addressable representation
/// disagrees with the underlying assembly identity contract, it stops at the
/// boundary where the disagreement becomes visible.
#[must_use]
pub fn run_tour() -> Tour {
    let assembly = build_basilica_assembly(&BasilicaParams::default());
    let mut space = assembly.into_addressable(SpaceId::<AssemblySpace>::new(1));

    let dome_query = Query::one(space.root_locator())
        .traverse(AssemblyAxis::Descendants)
        .filter(AssemblyPredicate::part(names::parts::CROSSING_DOME));
    let dome = space
        .query_one(&dome_query)
        .expect("the Basilica contains exactly one crossing dome")
        .into_value();

    let relative_path = dome.address().relative_to(
        space
            .resolve(&space.root_locator())
            .resolved()
            .expect("synthetic root resolves")
            .address(),
    );
    let relative = Locator::relative(
        space.id(),
        AssemblyView::Instances,
        addressable::AbsoluteAddress::root(),
        relative_path,
    );
    let Resolution::Resolved(relative_dome) = space.resolve(&relative) else {
        panic!("derived relative locator resolves");
    };
    assert_eq!(relative_dome, dome, "exact and relative resolution agree");

    let pin = Pinned::new(relative.clone(), dome.referent().clone(), dome.revision());
    assert!(
        matches!(space.resolve_pinned(&pin), Resolution::Resolved(_)),
        "fresh pin resolves without qualification"
    );

    let handle = space
        .resolved_handle(&dome)
        .expect("concrete occurrence has a runtime handle");
    let instance = space
        .instance_by_handle(&handle)
        .expect("fresh handle reads an instance");
    assert_eq!(
        instance.key(),
        names::instances::CROSSING_DOME,
        "runtime handle retains the selected occurrence"
    );

    let endpoint = Endpoint::new(dome.clone(), MaterialSlot::new("surface"));
    let explanation = space
        .read_material(&endpoint)
        .expect("declared material slot reads")
        .expect("Basilica dome has an effective material");
    let material_before = String::from(explanation.value().as_ref());
    let reason_before = *explanation.reason();
    let guard = Guard::new(
        dome.referent().clone(),
        dome.revision(),
        Some(explanation.value().clone()),
        EditCapability::BindMaterial,
    );
    let edit = BindMaterial::new(endpoint, "restored-copper", guard);

    let preview = space
        .transact(Transaction::dry_run(dome.revision(), [edit.clone()]))
        .expect("dry run validates");
    assert_eq!(
        preview.mode(),
        TransactionMode::DryRun,
        "preview reports dry-run mode"
    );
    assert_eq!(space.revision(), dome.revision(), "dry run does not mutate");

    let applied = space
        .transact(Transaction::apply(dome.revision(), [edit]))
        .expect("guarded binding applies");
    assert_eq!(
        applied.mode(),
        TransactionMode::Apply,
        "committed transaction reports apply mode"
    );
    assert_eq!(
        applied.changes(),
        preview.changes(),
        "dry run and apply report the same changes"
    );
    assert_eq!(
        applied.undo(),
        preview.undo(),
        "dry run and apply retain the same undo evidence"
    );
    assert_eq!(applied.undo().len(), 1, "one prior binding is retained");
    assert!(
        space.instance_by_handle(&handle).is_err(),
        "the old runtime handle is revision-stale"
    );
    assert!(
        matches!(space.resolve_pinned(&pin), Resolution::StaleRevision { .. }),
        "pre-edit pin reports its stale revision"
    );

    let Resolution::Resolved(fresh_dome) = space.resolve(&relative) else {
        panic!("durable locator resolves after mutation");
    };
    let fresh_endpoint = Endpoint::new(fresh_dome, MaterialSlot::new("surface"));
    let material_after = space
        .read_material(&fresh_endpoint)
        .expect("fresh endpoint reads")
        .expect("applied binding is effective")
        .value()
        .as_ref()
        .to_owned();

    Tour {
        address: dome.address().to_string(),
        part: dome
            .referent()
            .as_part()
            .expect("dome is a concrete part")
            .as_str()
            .to_owned(),
        material_before,
        reason_before,
        preview_changes: preview.changes().len(),
        material_after,
        revision_after: applied.revision_after().get(),
    }
}

#[cfg(test)]
mod tests {
    use addressable::{Endpoint, Guard, Pinned, Query, Resolution, SpaceId, Transaction};
    use basilica_ruin::{BasilicaParams, build_basilica_assembly, names};
    use exedra_assembly::{
        AssemblyAxis, AssemblyPredicate, AssemblyReferent, AssemblySpace, AssemblyView,
        BindMaterial, EditCapability, MaterialSlot, TransactionConflict,
    };

    use super::run_tour;

    #[test]
    fn tour_reaches_the_applied_material() {
        let tour = run_tour();
        assert_eq!(tour.address, "/crossing-dome", "query selected the dome");
        assert_eq!(
            tour.part,
            names::parts::CROSSING_DOME,
            "location retains the stable part referent"
        );
        assert_eq!(tour.preview_changes, 1, "dry run predicts one edit");
        assert_eq!(
            tour.material_after, "restored-copper",
            "applied binding becomes effective"
        );
        assert_eq!(tour.revision_after, 1, "one applied batch advances once");
    }

    #[test]
    fn root_children_and_descendants_preserve_the_assembly_forest() {
        let assembly = build_basilica_assembly(&BasilicaParams::default());
        let expected_roots = assembly.roots().len();
        let expected_instances = assembly.instances().len();
        let space = assembly.into_addressable(SpaceId::<AssemblySpace>::new(2));

        let children = space
            .query_many(&Query::many(space.root_locator()).traverse(AssemblyAxis::Children))
            .expect("root children query succeeds");
        let descendants = space
            .query_many(&Query::many(space.root_locator()).traverse(AssemblyAxis::Descendants))
            .expect("descendants query succeeds");

        assert_eq!(
            children.items().len(),
            expected_roots,
            "synthetic-root children are Exedra roots"
        );
        assert_eq!(
            descendants.items().len(),
            expected_instances,
            "synthetic-root descendants cover every instance"
        );
    }

    #[test]
    fn locator_and_pin_documents_round_trip() {
        let space = build_basilica_assembly(&BasilicaParams::default())
            .into_addressable(SpaceId::<AssemblySpace>::new(3));
        let dome = space
            .query_one(
                &Query::one(space.root_locator())
                    .traverse(AssemblyAxis::Descendants)
                    .filter(AssemblyPredicate::part(names::parts::CROSSING_DOME)),
            )
            .expect("one dome")
            .into_value();
        let locator = space
            .locator(
                dome.occurrence()
                    .as_instance()
                    .expect("dome is an instance"),
            )
            .expect("current path has a locator");
        let locator_text = locator.to_string();
        let parsed = locator_text
            .parse::<exedra_assembly::AssemblyLocator>()
            .expect("locator document parses");
        assert_eq!(parsed, locator, "locator serialization is lossless");

        let pin = Pinned::new(locator, dome.referent().clone(), dome.revision());
        let pin_text = pin.to_string();
        let parsed = pin_text
            .parse::<Pinned<AssemblySpace, AssemblyView, AssemblyReferent>>()
            .expect("pin document parses");
        assert_eq!(parsed, pin, "pin serialization is lossless");
        assert!(
            matches!(space.resolve_pinned(&parsed), Resolution::Resolved(_)),
            "round-tripped fresh pin resolves"
        );
    }

    #[test]
    fn failed_batch_has_no_partial_effect() {
        let mut space = build_basilica_assembly(&BasilicaParams::default())
            .into_addressable(SpaceId::<AssemblySpace>::new(4));
        let dome = space
            .query_one(
                &Query::one(space.root_locator())
                    .traverse(AssemblyAxis::Descendants)
                    .filter(AssemblyPredicate::part(names::parts::CROSSING_DOME)),
            )
            .expect("one dome")
            .into_value();
        let pendentives = space
            .query_many(
                &Query::many(space.root_locator())
                    .traverse(AssemblyAxis::Descendants)
                    .filter(AssemblyPredicate::part(names::parts::PENDENTIVE_WEB)),
            )
            .expect("pendentives query");
        let pendentive = pendentives.items()[0].clone();

        let dome_endpoint = Endpoint::new(dome.clone(), MaterialSlot::new("surface"));
        let dome_before = space
            .read_material(&dome_endpoint)
            .expect("dome slot reads")
            .expect("dome material exists")
            .value()
            .clone();
        let dome_edit = BindMaterial::new(
            dome_endpoint,
            "should-not-apply",
            Guard::new(
                dome.referent().clone(),
                dome.revision(),
                Some(dome_before.clone()),
                EditCapability::BindMaterial,
            ),
        );
        let pendentive_endpoint = Endpoint::new(pendentive.clone(), MaterialSlot::new("surface"));
        let stale_edit = BindMaterial::new(
            pendentive_endpoint,
            "also-not-applied",
            Guard::new(
                pendentive.referent().clone(),
                pendentive.revision(),
                Some("deliberately-wrong".into()),
                EditCapability::BindMaterial,
            ),
        );

        let result = space.transact(Transaction::apply(dome.revision(), [dome_edit, stale_edit]));
        assert!(
            matches!(
                result,
                Err(TransactionConflict::ValueMismatch { operation: 1, .. })
            ),
            "the stale second operation rejects the whole batch"
        );
        assert_eq!(
            space.revision(),
            dome.revision(),
            "failed atomic batch does not advance revision"
        );

        let after = space
            .read_material(&Endpoint::new(dome, MaterialSlot::new("surface")))
            .expect("original endpoint remains current")
            .expect("dome material still exists");
        assert_eq!(
            after.value(),
            &dome_before,
            "failed batch leaves the earlier target unchanged"
        );
    }

    #[test]
    fn applied_edit_stales_runtime_handle() {
        let mut space = build_basilica_assembly(&BasilicaParams::default())
            .into_addressable(SpaceId::<AssemblySpace>::new(5));
        let dome = space
            .query_one(
                &Query::one(space.root_locator())
                    .traverse(AssemblyAxis::Descendants)
                    .filter(AssemblyPredicate::part(names::parts::CROSSING_DOME)),
            )
            .expect("one dome")
            .into_value();
        let handle = space.resolved_handle(&dome).expect("instance handle");
        let endpoint = Endpoint::new(dome.clone(), MaterialSlot::new("surface"));
        let old = space
            .read_material(&endpoint)
            .expect("slot reads")
            .expect("material exists")
            .value()
            .clone();
        let edit = BindMaterial::new(
            endpoint,
            "new-material",
            Guard::new(
                dome.referent().clone(),
                dome.revision(),
                Some(old),
                EditCapability::BindMaterial,
            ),
        );
        space
            .transact(Transaction::apply(dome.revision(), [edit]))
            .expect("edit applies");

        assert!(
            space.instance_by_handle(&handle).is_err(),
            "applied edit invalidates the old runtime handle"
        );
    }

    #[test]
    fn committed_authoring_stales_handles_and_pins() {
        let mut space = build_basilica_assembly(&BasilicaParams::default())
            .into_addressable(SpaceId::<AssemblySpace>::new(6));
        let dome = space
            .query_one(
                &Query::one(space.root_locator())
                    .traverse(AssemblyAxis::Descendants)
                    .filter(AssemblyPredicate::part(names::parts::CROSSING_DOME)),
            )
            .expect("one dome")
            .into_value();
        let handle = space.resolved_handle(&dome).expect("instance handle");
        let pin = Pinned::new(
            space
                .locator(
                    dome.occurrence()
                        .as_instance()
                        .expect("instance occurrence"),
                )
                .expect("current locator"),
            dome.referent().clone(),
            dome.revision(),
        );
        let (_, authored) = space
            .commit(|assembly| assembly.set_metadata(*handle.handle(), "conservation", "planned"));
        authored.expect("metadata update applies");

        assert!(
            space.instance_by_handle(&handle).is_err(),
            "committed authoring invalidates the old runtime handle"
        );
        assert!(
            matches!(space.resolve_pinned(&pin), Resolution::StaleRevision { .. }),
            "committed authoring reports the old pin's stale revision"
        );
    }
}
