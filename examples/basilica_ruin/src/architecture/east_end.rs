// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::{BuildContext, Layout};
use crate::BasilicaParams;
use crate::geometry::{
    apse_profile, apse_roof_recipe, east_chancel_profile, extruded_profile_recipe,
    transverse_wall_frame,
};
use crate::names;

pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let half_nave = layout.half_nave;
    // A transverse chancel wall closes the nave roof end while a broad
    // round-headed opening keeps the apse spatially continuous.
    let east_chancel = context.add_part(
        names::parts::EAST_CHANCEL_GABLE,
        extruded_profile_recipe(
            east_chancel_profile(p),
            0.5,
            transverse_wall_frame(),
            "basilica:east-chancel-gable",
        ),
        "limestone",
    );
    context.add_instance(
        names::instances::EAST_CHANCEL_GABLE,
        east_chancel,
        Placement3::translate(p.length - 0.25, -half_nave, 0.0),
        "chancel_gable",
    );

    let apse = context.add_part(
        "apse",
        extruded_profile_recipe(
            apse_profile(p.apse_radius),
            8.0,
            Placement3::IDENTITY,
            "basilica:east-apse",
        ),
        "limestone",
    );
    context.add_instance(
        names::instances::EAST_APSE,
        apse,
        Placement3::translate(p.length, 0.0, 0.0),
        "apse",
    );
    let apse_roof = context.add_part(
        "apse-roof",
        apse_roof_recipe(p.apse_radius + 0.18, 2.35),
        "aged-roof-tile",
    );
    context.add_instance(
        "east-apse-roof",
        apse_roof,
        Placement3::translate(p.length, 0.0, 8.0),
        "apse_roof",
    );
}
