// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Coordinated openings where the two ground channels cross the entrance wall.

use crate::{Result, geometry};
use exedra_constructive::ir::Placement3;
use joiner::{Construction, PartEdit, RuleApplication, RuleOutput, ToolSolid};

pub(crate) const CHANNEL_X: [f64; 2] = [0.31, 4.78];
pub(crate) const CHANNEL_WIDTH: f64 = 0.19;

pub(crate) fn outlets(construction: &mut Construction) -> Result<()> {
    let evidence = construction
        .relation("gate-bond")
        .ok_or("missing gate bond")?
        .evidence
        .clone();
    let mut output = RuleOutput::new();
    for x in CHANNEL_X {
        let tool = geometry::block([CHANNEL_WIDTH, 0.46, 0.20], 0.0)?;
        let placement = Placement3::translate(x, -0.02, -0.24);
        for unit in construction.elements().iter().filter(|e| e.part.is_some()) {
            // Conservative world bounds include the rotated lower surround.
            let center = unit.extent.center();
            let overlaps = [(0, x, x + CHANNEL_WIDTH), (2, -0.24, -0.04)]
                .into_iter()
                .all(|(axis, low, high)| {
                    let radius = (0..3)
                        .map(|i| unit.extent.size[i] * unit.extent.axes[i][axis].abs() * 0.5)
                        .sum::<f64>();
                    center[axis] + radius > low && center[axis] - radius < high
                });
            if overlaps {
                output.edit(PartEdit::remove(
                    &unit.key,
                    ToolSolid::new(
                        "channel-outlet",
                        tool.clone(),
                        unit.extent.local_placement(placement),
                    ),
                    evidence.clone(),
                ));
            }
        }
    }
    construction.apply(RuleApplication::new(
        "open-drain-outlets",
        "courtyard:drain-outlets@1",
        "gate-bond",
        evidence,
        output,
    ))?;
    Ok(())
}
