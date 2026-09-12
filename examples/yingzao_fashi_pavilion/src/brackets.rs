// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One authored bracket template: a housed arm and a crossed, lapped arm.
//!
//! These dimensions describe this illustration. They are not a historical
//! joint prescription or a new general-purpose rule library.

use exedra_constructive::builders::rect;
use exedra_constructive::ir::{Placement3, Recipe};
use exedra_constructive::offset::CornerPolicy;
use joiner::{
    Anchor, Construction, ContactMeaning, ContactPatch, Element, Evidence, Node, OrientedBox, Part,
    PartEdit, Relation, RelationKind, RuleApplication, RuleOutput, ToolSolid, TransferEdge,
    TransferKind, TransferTarget, compose,
};
use joiner_timber::FitClass;

use crate::{Result, geometry, joinery, layout::Layout};

pub(crate) struct FittedPart {
    pub(crate) key: String,
    pub(crate) recipe: Recipe,
    pub(crate) extent: OrientedBox,
}

pub(crate) fn fitted(l: &Layout) -> Result<Vec<FittedPart>> {
    let construction = construction(l)?;
    construction
        .elements()
        .iter()
        .map(|element| {
            Ok(FittedPart {
                key: element.key.clone(),
                recipe: compose(&construction, element)?,
                extent: element.extent.clone(),
            })
        })
        .collect()
}

fn construction(l: &Layout) -> Result<Construction> {
    let (mut construction, evidence) = joinery::construction()?;
    let w = l.arm_width;
    let d = l.arm_depth;
    let block = OrientedBox::axis_aligned([-0.18, -0.18, 0.0], [0.36, 0.36, 0.18]);
    let lower = OrientedBox::axis_aligned([-0.51, -w * 0.5, 0.15], [1.02, w, d]);
    let upper = OrientedBox {
        origin: [w * 0.5, -0.85, 0.15 + d - 0.045],
        axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        size: [1.7, w, d],
    };
    for (key, extent, recipe) in [
        ("bearing-block", &block, geometry::block(block.size)?),
        ("lower-arm", &lower, geometry::arm(1.02, w, d)?),
        ("upper-arm", &upper, geometry::arm(1.7, w, d)?),
    ] {
        joinery::member(
            &mut construction,
            Element::new(key, key, "timber", extent.clone(), evidence.clone())
                .with_part(Part::new(recipe)),
        )?;
    }
    // One nominal width and the existing timber fit type drive all receiving
    // profiles. The cutter overruns external faces; the bearing plane stays exact.
    let allowance = FitClass::CLOSE.allowance_meters();
    let receiving = |a, b| -> Result<_> {
        Ok(rect(a, b)?.offset(allowance, CornerPolicy::Miter { limit: 2.0 })?)
    };
    let housing = ToolSolid::new(
        "arm-housing",
        geometry::extrude(receiving(0.362, w)?, 0.031, Placement3::IDENTITY)?,
        Placement3::translate(-0.001, 0.18 - w * 0.5, 0.15),
    );
    let mut output = RuleOutput::new();
    output.edit(PartEdit::remove("bearing-block", housing, evidence.clone()));
    fit_relation(
        &mut construction,
        "arm-housing",
        "lower-arm",
        "bearing-block",
        [0.0, 0.0, 0.15],
        &lower,
        &block,
        &evidence,
        output,
    )?;

    let lap_z = 0.15 + d - 0.0225;
    let mut output = RuleOutput::new();
    for (key, extent, start_z, height) in [
        ("lower-arm", &lower, lap_z, 0.0235),
        ("upper-arm", &upper, upper.origin[2] - 0.001, 0.0235),
    ] {
        // The square crossing is symmetric in the two local frames.
        let cutter = ToolSolid::new(
            &format!("{key}-lap"),
            geometry::extrude(receiving(w, w)?, height, Placement3::IDENTITY)?,
            Placement3::translate((extent.size[0] - w) * 0.5, 0.0, start_z - extent.origin[2]),
        );
        output.edit(PartEdit::remove(key, cutter, evidence.clone()));
    }
    fit_relation(
        &mut construction,
        "arm-cross-lap",
        "upper-arm",
        "lower-arm",
        [0.0, 0.0, lap_z],
        &upper,
        &lower,
        &evidence,
        output,
    )?;
    Ok(construction)
}

fn fit_relation(
    construction: &mut Construction,
    key: &str,
    carried: &str,
    carrier: &str,
    point: [f64; 3],
    a: &OrientedBox,
    b: &OrientedBox,
    evidence: &Evidence,
    mut output: RuleOutput,
) -> Result<()> {
    construction.add_node(Node::new(key, point))?;
    construction.add_relation(Relation::new(
        key,
        RelationKind::member_member(key, &[carried, carrier]),
        key,
        evidence.clone(),
    ))?;
    output.contact(
        ContactPatch::new(
            key,
            Anchor::new(carried, a.local_point(point)),
            Anchor::new(carrier, b.local_point(point)),
            [0.0, 0.0, 1.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            ContactMeaning::Bearing,
            evidence.clone(),
        )
        .with_minimum_overlap_meters([0.14, 0.14])
        .with_footprint_meters([0.14, 0.14]),
    );
    output.transfer(TransferEdge::new(
        key,
        carried,
        TransferTarget::element(carrier),
        TransferKind::Contact,
    ));
    construction.apply(RuleApplication::new(
        key,
        "pavilion:authored-bracket-fit@1",
        key,
        evidence.clone(),
        output,
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile_policy;
    use crate::layout::Parameters;
    use crate::tests::recipe_volume;
    use exedra_assembly::PartCompiler;
    use joiner::{lower, measure_contact, measure_contact_geometry, part_key};

    #[test]
    fn fitted_bracket_removes_the_housing_and_both_halves_of_the_lap() -> Result<()> {
        let l = Layout::resolve(Parameters::default())?;
        let construction = construction(&l)?;
        let assembly = lower(&construction)?;
        let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
        for contact in construction.contacts() {
            let measured =
                measure_contact(&construction, contact).ok_or("missing contact participant")?;
            assert!(measured.anchors_coincide(), "bearing anchors coincide");
            let part = |key: &str| {
                compiled
                    .part(assembly.part_by_key(&part_key(key)).unwrap())
                    .unwrap()
            };
            assert!(
                measure_contact_geometry(
                    &construction,
                    contact,
                    part(&contact.carried.element),
                    part(&contact.carrier.element),
                    0.000_01
                )?
                .is_covered()
            );
            assert!(
                measured.overlap.into_iter().all(|v| v >= 0.15 - 1.0e-12),
                "full bearing width"
            );
        }
        for element in construction.elements() {
            let base = recipe_volume(element.part.as_ref().unwrap().recipe.clone())?;
            let fitted = recipe_volume(compose(&construction, element)?)?;
            let removed = if element.key == "bearing-block" {
                0.36 * 0.151 * 0.03
            } else {
                0.151 * 0.15 * 0.0225
            };
            assert!(
                (base - fitted - removed).abs() < 1.0e-8,
                "{} removed {}, expected {removed}",
                element.key,
                base - fitted
            );
        }
        Ok(())
    }
}
