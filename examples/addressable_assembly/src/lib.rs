// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! End-to-end Addressable tour over the Basilica assembly.

use std::string::String;

use addressable::{Endpoint, Locator, Pinned, Query, Resolution, SpaceId};
use basilica_ruin::{BasilicaParams, build_basilica_assembly, names};
use exedra_assembly::{
    AssemblyAxis, AssemblyPredicate, AssemblySpace, AssemblyView, MaterialReason, MaterialSlot,
};

/// Observable milestones from [`run_tour`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tour {
    /// Canonical address selected by the query.
    pub address: String,
    /// Stable part referent selected at that address.
    pub part: String,
    /// Effective material selected for the resolved occurrence.
    pub material: String,
    /// Why that material won.
    pub reason: MaterialReason,
    /// Number of authored opinions retained as evidence.
    pub material_opinions: usize,
}

/// Runs resolution, query, handle, and material-explanation workflows.
///
/// The function uses stable Basilica vocabulary rather than runtime Exedra
/// indices. Assertions are part of the tour: if any Addressable representation
/// disagrees with the underlying assembly identity contract, it stops at the
/// boundary where the disagreement becomes visible.
#[must_use]
pub fn run_tour() -> Tour {
    let assembly = build_basilica_assembly(&BasilicaParams::default());
    let space = assembly.into_addressable(SpaceId::<AssemblySpace>::new(1));

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

    Tour {
        address: dome.address().to_string(),
        part: dome
            .referent()
            .as_part()
            .expect("dome is a concrete part")
            .as_str()
            .to_owned(),
        material: String::from(explanation.value().as_ref()),
        reason: *explanation.reason(),
        material_opinions: explanation.opinions().len(),
    }
}

#[cfg(test)]
mod tests {
    use addressable::{Pinned, Query, Resolution, SpaceId};
    use basilica_ruin::{BasilicaParams, build_basilica_assembly, names};
    use exedra_assembly::{
        AssemblyAxis, AssemblyPredicate, AssemblyReferent, AssemblySpace, AssemblyView,
    };

    use super::run_tour;

    #[test]
    fn tour_reaches_the_explained_material() {
        let tour = run_tour();
        assert_eq!(tour.address, "/crossing-dome", "query selected the dome");
        assert_eq!(
            tour.part,
            names::parts::CROSSING_DOME,
            "location retains the stable part referent"
        );
        assert!(!tour.material.is_empty(), "effective material is explained");
        assert!(tour.material_opinions > 0, "explanation retains evidence");
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
    fn committed_authoring_stales_handles_and_pins() {
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
