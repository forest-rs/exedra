// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Check the interfaces between the independently authored timber and roof groups.

use exedra_assembly::{CompiledPart, PartCompiler, flatten};
use exedra_constructive::ir::Placement3;
use joiner::{
    Anchor, ContactMeaning, ContactPatch, Element, OrientedBox, measure_contact_geometry,
};

use super::solid::Solid;
use crate::layout::CROSS_BEAM_WIDTH;
use crate::{Result, check_reports, compile_policy, joinery, pavilion};

struct Placed<'a> {
    part: &'a CompiledPart,
    placement: Placement3,
    solid: Solid,
}

impl Placed<'_> {
    fn extent(&self) -> OrientedBox {
        let bounds = self.part.bounds().expect("real part");
        assert!(bounds.min.into_iter().all(|v| v.abs() < 1e-6));
        OrientedBox {
            origin: self.placement.rows.map(|row| row[3]),
            axes: core::array::from_fn(|i| self.placement.rows.map(|row| row[i])),
            // Bounds come from f32 render vertices. Enclose their rounding
            // error without changing the actual part's placement.
            size: bounds.max.map(|value| value + 1e-6),
        }
    }
}

fn covered(
    carried: &Placed<'_>,
    carrier: &Placed<'_>,
    at: [f64; 3],
    normal: [f64; 3],
    tangents: [[f64; 3]; 2],
    size: [f64; 2],
    meaning: ContactMeaning,
) -> Result<()> {
    let (mut construction, evidence) = joinery::construction()?;
    let a = carried.extent();
    let b = carrier.extent();
    for (key, extent) in [("carried", &a), ("carrier", &b)] {
        construction.add_element(Element::new(
            key,
            "test",
            "timber",
            extent.clone(),
            evidence.clone(),
        ))?;
    }
    let contact = ContactPatch::new(
        "timber-junction",
        Anchor::new("carried", a.local_point(at)),
        Anchor::new("carrier", b.local_point(at)),
        normal,
        tangents,
        meaning,
        evidence,
    )
    .with_footprint_meters(size);
    let measurement =
        measure_contact_geometry(&construction, &contact, carried.part, carrier.part, 1e-5)
            .unwrap_or_else(|error| panic!("{error}: {contact:?}; carried={a:?}; carrier={b:?}"));
    assert!(
        measurement.is_covered(),
        "missing contact at {at:?}: {measurement:?}"
    );
    Ok(())
}

fn disjoint(a: &Placed<'_>, b: &Placed<'_>, context: &str) {
    // For these butt/bearing interfaces, separated emitted-geometry bounds
    // prove no volume overlap. The separate coverage check proves they meet.
    assert!(
        (0..3).any(
            |axis| a.solid.bounds.max[axis] <= b.solid.bounds.min[axis] + 1e-6
                || b.solid.bounds.max[axis] <= a.solid.bounds.min[axis] + 1e-6
        ),
        "interpenetrating timber bounds: {context}"
    );
}

#[test]
fn cross_beams_bear_on_brackets_and_longitudinal_beams_butt_against_them() -> Result<()> {
    for (bays, span_mm, depth_mm) in [(1, 2_400, 5_400), (1, 3_600, 3_600), (5, 5_400, 2_400)] {
        let assembly = pavilion(bays, span_mm, depth_mm)?;
        let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
        check_reports(&assembly, &compiled)?;
        let draw = flatten(&assembly, &compiled);
        let placed = |suffix: &str| {
            let item = draw
                .items
                .iter()
                .find(|i| i.path.to_string().ends_with(suffix))
                .expect(suffix);
            let part = compiled.part(item.part).unwrap();
            Placed {
                part,
                placement: item.world,
                solid: Solid::new(part, item.world),
            }
        };
        for frame in 0..=bays {
            let cross = placed(&format!("/fitted-roof-support-1-{frame}"));
            for (side, direction) in [("front", -1.0), ("back", 1.0)] {
                let arm = placed(&format!("/frame-{frame}-{side}-upper-arm"));
                let center = core::array::from_fn::<_, 3, _>(|i| {
                    (arm.solid.bounds.min[i] + arm.solid.bounds.max[i]) * 0.5
                });
                disjoint(&cross, &arm, "cross beam / upper arm");
                covered(
                    &cross,
                    &arm,
                    [
                        center[0],
                        center[1] - direction * 0.3,
                        cross.placement.rows[2][3],
                    ],
                    [0.0, 0.0, 1.0],
                    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    [0.14, 0.20],
                    ContactMeaning::Bearing,
                )?;
                for (bay, facing) in [
                    (frame.checked_sub(1), -1.0),
                    ((frame < bays).then_some(frame), 1.0),
                ] {
                    let Some(bay) = bay else { continue };
                    let longitudinal = placed(&format!("/bay-{bay}-{side}-beam"));
                    disjoint(&longitudinal, &arm, "longitudinal beam / upper arm");
                    disjoint(&longitudinal, &cross, "longitudinal / cross beam");
                    let tip = placed(&format!(
                        "/frame-{frame}-{side}-tip-{}",
                        if facing > 0.0 { 1 } else { -1 }
                    ));
                    disjoint(&longitudinal, &tip, "longitudinal beam / tip block");
                    covered(
                        &longitudinal,
                        &tip,
                        [
                            (tip.solid.bounds.min[0] + tip.solid.bounds.max[0]) * 0.5,
                            center[1],
                            longitudinal.placement.rows[2][3],
                        ],
                        [0.0, 0.0, 1.0],
                        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                        [0.20, 0.20],
                        ContactMeaning::Bearing,
                    )?;
                    let bottom = cross.solid.bounds.min[2].max(longitudinal.solid.bounds.min[2]);
                    let top = cross.solid.bounds.max[2].min(longitudinal.solid.bounds.max[2]);
                    assert!(top - bottom > 0.1);
                    let x = if facing > 0.0 {
                        longitudinal.placement.rows[0][3]
                    } else {
                        cross.placement.rows[0][3] - CROSS_BEAM_WIDTH
                    };
                    covered(
                        &longitudinal,
                        &cross,
                        [x, center[1] - direction * 0.06, (bottom + top) * 0.5],
                        [facing, 0.0, 0.0],
                        [[0.0, facing, 0.0], [0.0, 0.0, 1.0]],
                        [0.08, top - bottom - 0.002],
                        ContactMeaning::SideFit,
                    )?;
                }
            }
        }
    }
    Ok(())
}
