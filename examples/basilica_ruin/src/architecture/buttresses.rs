// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::{BuildContext, Layout};
use crate::BasilicaParams;
use crate::geometry::box_recipe;
use crate::names;

pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let half_total = layout.half_total;
    let buttress = context.add_part(
        names::parts::AISLE_BUTTRESS,
        box_recipe([0.72, 1.1, 5.5], "basilica:aisle-buttress"),
        "weathered-limestone",
    );
    let bay_pitch = (p.length - 4.0) / f64::from(p.arcade_bays);
    for side in ["north", "south"] {
        for bay in 0..=p.arcade_bays {
            let x = 1.7 + f64::from(bay) * bay_pitch;
            let y = if side == "north" {
                half_total - 0.18
            } else {
                -half_total - 0.92
            };
            context.add_instance(
                &format!("buttress-{side}-{bay:02}"),
                buttress,
                Placement3::translate(x, y, 0.0),
                names::roles::AISLE_BUTTRESS,
            );
        }
    }
}
