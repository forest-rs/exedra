// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Round purlins and their actual bearing pads/beams, fitted at setout datums.

use exedra_assembly::{Assembly, CompiledParts};
use exedra_constructive::builders::circle;
use exedra_constructive::ir::Placement3;
use joiner::{
    Construction, Element, Node, OrientedBox, Part, Relation, RelationKind,
    measure_contact_geometry,
};
use joiner_timber::{RoundPurlinSeatParams, RoundPurlinSeatRule};

use crate::{Result, geometry, joinery, layout::Layout};

pub(crate) struct RoofSeats {
    pub(crate) construction: Construction,
    /// Element key and shared geometry key. Roof levels with equal sections
    /// and the same seat stations deliberately instance one fitted part.
    pub(crate) instances: Vec<(String, String)>,
}

pub(crate) fn build(l: &Layout) -> Result<RoofSeats> {
    build_at_stations(l, l.width + 1.2, &l.frames)
}

pub(crate) fn study(l: &Layout) -> Result<RoofSeats> {
    build_at_stations(l, 0.65, &[0.0])
}

fn build_at_stations(l: &Layout, length: f64, stations: &[f64]) -> Result<RoofSeats> {
    let (mut construction, evidence) = joinery::construction()?;
    let mut instances = Vec::new();
    let radius = 0.095;
    let round = geometry::extrude(
        circle(radius)?,
        length,
        Placement3::from_axes(
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.0, radius, radius],
        ),
    )?;
    let params = RoundPurlinSeatParams {
        seat_depth: l.seat_depth,
        ..RoundPurlinSeatParams::default()
    };
    for (level, [run, _]) in l.roof.iter().copied().enumerate() {
        let bearing = l.bearing_height(level);
        let (support_width, support_length, support_height) = match level {
            0 => (0.15, 0.20, 0.02 + l.seat_depth.as_meters()),
            1 => (0.24, 2.0 * run + 0.28, 0.30),
            _ => (0.20, 2.0 * run + 0.40, 0.10),
        };
        let support_size = [support_length, support_width, support_height];
        let support_recipe = geometry::block(support_size)?;
        let sides: &[i32] = if level == 4 { &[1] } else { &[-1, 1] };
        for &side in sides {
            let y = f64::from(side) * run;
            let key = format!("roof-purlin-{level}-{side}");
            joinery::member(
                &mut construction,
                Element::new(
                    &key,
                    "round-purlin",
                    "timber.dark",
                    OrientedBox::axis_aligned(
                        [-length * 0.5, y - radius, l.purlin_bottom(level)],
                        [length, radius * 2.0, radius * 2.0],
                    ),
                    evidence.clone(),
                )
                .with_part(Part::new(round.clone())),
            )?;
            let family = match level {
                0 => "eave",
                1 => "cross",
                _ => "stack",
            };
            instances.push((key.clone(), format!("seated-purlin-{family}")));
            for (frame, &x) in stations.iter().enumerate() {
                let support = if level == 0 {
                    format!("eave-pad-{frame}-{side}")
                } else {
                    format!("roof-support-{level}-{frame}")
                };
                if construction.element(&support).is_none() {
                    let centre_y = if level == 0 { y } else { 0.0 };
                    let extent = OrientedBox {
                        origin: [
                            x + support_width * 0.5,
                            centre_y - support_length * 0.5,
                            bearing - support_height,
                        ],
                        axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                        size: support_size,
                    };
                    joinery::member(
                        &mut construction,
                        Element::new(
                            &support,
                            "purlin-support",
                            if level == 0 { "timber.end" } else { "timber" },
                            extent,
                            evidence.clone(),
                        )
                        .with_part(Part::new(support_recipe.clone())),
                    )?;
                    instances.push((support.clone(), format!("roof-support-beam-{level}")));
                }
                let relation = format!("seat-{level}-{side}-{frame}");
                construction.add_node(Node::new(&relation, [x, y, bearing]))?;
                construction.add_relation(Relation::new(
                    &relation,
                    RelationKind::member_member(&relation, &[&key, &support]),
                    "flat-round-seat",
                    evidence.clone(),
                ))?;
                construction.apply_rule(&relation, &relation, &RoundPurlinSeatRule, &params)?;
            }
        }
    }
    Ok(RoofSeats {
        construction,
        instances,
    })
}

/// Checks the same shared compiled parts that the scene exports.
pub(crate) fn verify(
    seats: &RoofSeats,
    assembly: &Assembly,
    compiled: &CompiledParts,
) -> Result<f64> {
    let mut area = 0.0;
    for contact in seats.construction.contacts() {
        let part = |key: &str| -> Result<_> {
            let (_, family) = seats
                .instances
                .iter()
                .find(|(element, _)| element == key)
                .ok_or("missing fitted roof instance")?;
            let id = assembly
                .part_by_key(family)
                .ok_or("missing fitted roof part")?;
            Ok(compiled.part(id).ok_or("missing compiled roof part")?)
        };
        // The analytic chord is narrowed by 2 mm at each edge. For this
        // 190 mm section and 15 mm seat that covers the sideways boundary
        // error of the 1 mm chord tessellation, plus f32 output quantization.
        // The returned area records precisely how much surface was checked.
        let measured = measure_contact_geometry(
            &seats.construction,
            contact,
            part(&contact.carried.element)?,
            part(&contact.carrier.element)?,
            0.002,
        )?;
        if !measured.is_covered() {
            return Err(format!("{} has an uncovered bearing: {measured:?}", contact.key).into());
        }
        area += measured.checked_area;
    }
    Ok(area)
}
