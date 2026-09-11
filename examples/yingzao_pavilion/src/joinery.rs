// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Common construction authoring for the pavilion's explicit joint families.

use exedra_assembly::{Assembly, CompiledParts};
use exedra_constructive::ir::{Placement3, Recipe};
use joiner::{
    Construction, Element, Evidence, EvidenceClass, EvidenceSource, Member, Node, OrientedBox,
    PartEdit, RuleApplication, RuleOutput, ToolSolid, measure_contact_geometry,
};

use crate::{Result, geometry};

pub(crate) struct FittedConstruction {
    pub(crate) construction: Construction,
    /// Element key and shared geometry key. Repetition changes placements,
    /// while only different cuts or sections require another compiled part.
    pub(crate) instances: Vec<(String, String)>,
}

impl FittedConstruction {
    /// Checks the same shared compiled parts that the scene exports.
    pub(crate) fn verify(&self, assembly: &Assembly, compiled: &CompiledParts) -> Result<f64> {
        let mut area = 0.0;
        for contact in self.construction.contacts() {
            let part = |key: &str| -> Result<_> {
                let (_, family) = self
                    .instances
                    .iter()
                    .find(|(element, _)| element == key)
                    .ok_or("missing fitted instance")?;
                let id = assembly.part_by_key(family).ok_or("missing fitted part")?;
                Ok(compiled.part(id).ok_or("missing compiled fitted part")?)
            };
            // Insets the analytic rectangle to cover the sideways chord error
            // of 1 mm circle tessellation. Report the area actually checked.
            let measured = measure_contact_geometry(
                &self.construction,
                contact,
                part(&contact.carried.element)?,
                part(&contact.carrier.element)?,
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

/// A cutter authored in world coordinates, converted exactly once at the part boundary.
pub(crate) fn remove(
    output: &mut RuleOutput,
    element: &Element,
    key: &str,
    recipe: Recipe,
    placement: Placement3,
) {
    output.edit(PartEdit::remove(
        &element.key,
        ToolSolid::new(key, recipe, element.extent.local_placement(placement)),
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
