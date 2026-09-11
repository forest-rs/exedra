// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Common construction authoring for the pavilion's explicit joint families.

use std::collections::HashMap;

use exedra_assembly::{
    Assembly, CompiledParts, InstanceId, PartFingerprint, compose as compose_placements,
};
use exedra_constructive::ir::{Placement3, Recipe};
use joiner::{
    Construction, Element, Evidence, EvidenceClass, EvidenceSource, Member, Node, OrientedBox,
    PartEdit, RuleApplication, RuleOutput, ToolSolid, compose, lower_shared,
    measure_contact_geometry,
};

use crate::{Result, geometry};

pub(crate) struct FittedConstruction {
    pub(crate) construction: Construction,
    /// Element key and preferred part family. Only identical composed recipes
    /// share geometry; exceptional cuts or sources get a distinct variant.
    pub(crate) families: Vec<(String, String)>,
}

impl FittedConstruction {
    pub(crate) fn lower(&self) -> Result<Assembly> {
        let families: HashMap<_, _> = self
            .families
            .iter()
            .map(|(key, family)| (key.as_str(), family.as_str()))
            .collect();
        Ok(lower_shared(&self.construction, |element| {
            families
                .get(element.key.as_str())
                .copied()
                .unwrap_or(&element.key)
                .to_owned()
        })?)
    }

    /// Checks exported world placements against the authored construction coordinates.
    /// The caller supplies the element-to-instance mapping retained during assembly composition.
    pub(crate) fn verify(
        &self,
        assembly: &Assembly,
        compiled: &CompiledParts,
        find_instance: impl Fn(&str) -> Option<InstanceId>,
    ) -> Result<f64> {
        let mut parts = HashMap::new();
        for element in self
            .construction
            .elements()
            .iter()
            .filter(|e| e.present && e.part.is_some())
        {
            let id = find_instance(&element.key)
                .ok_or_else(|| format!("missing fitted instance: {}", element.key))?;
            let instance = assembly
                .instance(id)
                .ok_or_else(|| format!("missing fitted instance: {}", element.key))?;
            let mut world = *instance.placement();
            let mut parent = instance.parent();
            while let Some(id) = parent {
                let ancestor = assembly.instance(id).ok_or("missing fitted ancestor")?;
                world = compose_placements(ancestor.placement(), &world);
                parent = ancestor.parent();
            }
            if world != element.extent.placement() {
                return Err(format!("{} has a different exported placement", element.key).into());
            }
            let part = compiled
                .part(instance.part().ok_or("fitted instance has no geometry")?)
                .ok_or("missing compiled fitted part")?;
            if part.fingerprint
                != PartFingerprint(compose(&self.construction, element)?.recipe_fingerprint().0)
            {
                return Err(format!("{} has different exported geometry", element.key).into());
            }
            parts.insert(element.key.as_str(), part.as_ref());
        }
        let mut area = 0.0;
        for contact in self.construction.contacts() {
            // Insets the analytic rectangle to cover the sideways chord error
            // of 1 mm circle tessellation. Report the area actually checked.
            let measured = measure_contact_geometry(
                &self.construction,
                contact,
                parts
                    .get(contact.carried.element.as_str())
                    .ok_or("missing carried geometry")?,
                parts
                    .get(contact.carrier.element.as_str())
                    .ok_or("missing carrier geometry")?,
                0.002,
            )?;
            if !measured.is_covered() {
                return Err(
                    format!("{} has an uncovered contact: {measured:?}", contact.key).into(),
                );
            }
            area += measured.checked_area;
        }
        Ok(area)
    }
}

/// A cutter in the element's local frame, independent of instance placement.
pub(crate) fn remove(
    output: &mut RuleOutput,
    element: &Element,
    key: &str,
    recipe: Recipe,
    placement: Placement3,
) {
    output.edit(PartEdit::remove(
        &element.key,
        ToolSolid::new(key, recipe, placement),
        element.evidence.clone(),
    ));
}

pub(crate) fn remove_box(
    output: &mut RuleOutput,
    element: &Element,
    key: &str,
    bounds: OrientedBox,
) -> Result<()> {
    remove(
        output,
        element,
        key,
        geometry::block(bounds.size)?,
        bounds.placement(),
    );
    Ok(())
}

pub(crate) fn apply(
    construction: &mut Construction,
    key: &str,
    relation: &str,
    output: RuleOutput,
) -> Result<()> {
    let evidence = construction
        .relation(relation)
        .ok_or("missing fitting relation")?
        .evidence
        .clone();
    construction.apply(RuleApplication::new(
        key,
        "pavilion:authored-roof-fit@1",
        relation,
        evidence,
        output,
    ))?;
    Ok(())
}

pub(crate) fn construction() -> Result<(Construction, Evidence)> {
    let mut construction = Construction::new();
    let evidence = Evidence::new("pavilion-design", EvidenceClass::ModernEngineeringInference);
    construction.add_evidence_source(EvidenceSource::new("pavilion-design", evidence.class,
        "https://link.springer.com/chapter/10.1007/978-3-031-81623-9_24",
        "Modern illustration; module and roof method informed by Yingzao Fashi, fits authored for this scene"))?;
    Ok((construction, evidence))
}

pub(crate) fn member(construction: &mut Construction, element: Element) -> Result<()> {
    let key = &element.key;
    let extent = &element.extent;
    let start = format!("{key}-start");
    let end = format!("{key}-end");
    for (name, station) in [(&start, 0.0), (&end, extent.size[0])] {
        construction.add_node(Node::new(
            name,
            extent.anchor([station, extent.size[1] * 0.5, extent.size[2] * 0.5]),
        ))?;
    }
    let member = Member::new(key, key, &start, &end, element.evidence.clone());
    construction.add_element(element.with_member())?;
    construction.add_member(member)?;
    Ok(())
}
