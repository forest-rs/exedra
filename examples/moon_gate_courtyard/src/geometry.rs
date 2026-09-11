// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Meter-space recipes used by the courtyard.

use crate::Result;
use exedra_constructive::builders::circle;
use exedra_constructive::edge_finish::{EdgeSelection, RoundPolicy};
use exedra_constructive::ir::{
    CapMode, CsgOp, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder,
};

pub(crate) fn block(size: [f64; 3], bevel: f64) -> Result<Recipe> {
    let mut b = RecipeBuilder::new();
    let slot = b.material_slot("surface");
    let mut root = b.with_material(slot).add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size },
        placement: Placement3::IDENTITY,
    })?;
    if bevel > 0.0 {
        root = b.add(NodeKind::EdgeFinish {
            child: root,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::chamfer(bevel),
        })?;
    }
    Ok(b.finish(root)?)
}
pub(crate) fn pierced_panel(size: [f64; 3], center: [f64; 2], radius: f64) -> Result<Recipe> {
    let mut b = RecipeBuilder::new();
    let slot = b.material_slot("surface");
    let stock = b.with_material(slot).add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size },
        placement: Placement3::IDENTITY,
    })?;
    let profile = b.add_profile(circle(radius)?);
    let tool = b.with_material(slot).add(NodeKind::Extrude {
        profile,
        height: size[1] + 0.04,
        caps: CapMode::Both,
        placement: Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [center[0], size[1] + 0.02, center[1]],
        ),
    })?;
    let root = b.add(NodeKind::Csg {
        op: CsgOp::Difference,
        operands: vec![stock, tool],
    })?;
    Ok(b.finish(root)?)
}
