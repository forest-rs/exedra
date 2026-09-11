// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small, reusable part recipes. All dimensions are meters at this boundary.

use exedra_constructive::builders::l_profile;
use exedra_constructive::edge_finish::{EdgeSelection, RoundPolicy};
use exedra_constructive::ir::{
    CapMode, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder,
};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};

use crate::Result;

pub(crate) fn block(size: [f64; 3]) -> Result<Recipe> {
    primitive(PrimitiveSpec::Box { size })
}

pub(crate) fn dressed_block(size: [f64; 3]) -> Result<Recipe> {
    let mut recipe = RecipeBuilder::new();
    let material = recipe.material_slot("surface");
    let child = recipe.with_material(material).add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size },
        placement: Placement3::IDENTITY,
    })?;
    let root = recipe.add(NodeKind::EdgeFinish {
        child,
        selection: EdgeSelection::SharpEdges,
        policy: RoundPolicy::chamfer(0.003),
    })?;
    Ok(recipe.finish(root)?)
}

pub(crate) fn cylinder(radius: f64, height: f64) -> Result<Recipe> {
    primitive(PrimitiveSpec::Cylinder {
        radius,
        height,
        segments: 32,
    })
}

fn primitive(spec: PrimitiveSpec) -> Result<Recipe> {
    let mut recipe = RecipeBuilder::new();
    let material = recipe.material_slot("surface");
    let root = recipe.with_material(material).add(NodeKind::Primitive {
        spec,
        placement: Placement3::IDENTITY,
    })?;
    Ok(recipe.finish(root)?)
}

pub(crate) fn extrude(profile: Profile2, height: f64, placement: Placement3) -> Result<Recipe> {
    let mut recipe = RecipeBuilder::new();
    let profile = recipe.add_profile(profile);
    let material = recipe.material_slot("surface");
    let root = recipe.with_material(material).add(NodeKind::Extrude {
        profile,
        height,
        placement,
        caps: CapMode::Both,
    })?;
    Ok(recipe.finish(root)?)
}

pub(crate) fn polygon(points: &[[f64; 2]]) -> Result<Profile2> {
    let outline = Loop2::new(points.iter().map(|p| Seg2::line((p[0], p[1]))).collect())?;
    Ok(Profile2::simple(if outline.signed_area() < 0.0 {
        outline.reversed()
    } else {
        outline
    })?)
}

/// A bracket arm with a deep central bearing and rising, curved undersides.
pub(crate) fn arm(length: f64, width: f64, depth: f64) -> Result<Recipe> {
    let l = length * 0.5;
    let profile = Profile2::simple(Loop2::new(vec![
        Seg2::line((-l, depth * 0.65)),
        Seg2::cubic((-width, 0.0), (-l * 0.6, depth * 0.60), (-l * 0.65, 0.0)),
        Seg2::line((width, 0.0)),
        Seg2::cubic((l, depth * 0.65), (l * 0.65, 0.0), (l * 0.6, depth * 0.60)),
        Seg2::line((l, depth)),
        Seg2::line((-l, depth)),
    ])?)?;
    extrude(
        profile,
        width,
        Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [l, width, 0.0],
        ),
    )
}

/// An isolated shoulder study: sharp, chamfered, or filleted into the void.
pub(crate) fn concave_shoulder(finish: Option<RoundPolicy>) -> Result<Recipe> {
    let mut builder = RecipeBuilder::new();
    let material = builder.material_slot("surface");
    let profile = builder.add_profile(l_profile(0.2, 0.2, 0.1, 0.1)?);
    let mut root = builder.with_material(material).add(NodeKind::Extrude {
        profile,
        placement: Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.2, 0.0],
        ),
        height: 0.4,
        caps: CapMode::Both,
    })?;
    if let Some(policy) = finish {
        // The two internal profile walls, selected by semantic region.
        root = builder.add(NodeKind::EdgeFinish {
            child: root,
            selection: EdgeSelection::RegionBoundaries(vec![[4, 5]]),
            policy,
        })?;
    }
    Ok(builder.finish(root)?)
}
