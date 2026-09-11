// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{format, vec};
use core::f64::consts::TAU;

use crate::Length;
use exedra_constructive::builders::circle;
use exedra_constructive::ir::{
    CapMode, CsgOp, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder,
};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use joiner::{Element, OrientedBox, Part, RuleContext, RuleError, RuleOutput};

/// A circular opening with a concentric, segmented masonry surround.
///
/// Both circles must fit inside the logical wall extent. The wall may extend
/// below finished ground so the lower arc can be buried under a level path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircularOpening {
    /// Circle center in wall-local X/Z coordinates, in meters.
    pub center: [f64; 2],
    /// Clear passage radius.
    pub radius: Length,
    /// Radial thickness of the masonry surround.
    pub surround: Length,
    /// Number of equal-angle surround units, between 8 and 256.
    pub segments: u32,
}

impl CircularOpening {
    pub(crate) fn validate(self, width: f64, height: f64, gap: f64) -> Result<(), RuleError> {
        let outer = self.radius.as_meters() + self.surround.as_meters() + gap;
        if !(8..=256).contains(&self.segments)
            || !self.center.iter().all(|v| v.is_finite())
            || self.center[0] - outer < 0.0
            || self.center[0] + outer > width
            || self.center[1] - outer < 0.0
            || self.center[1] + outer > height
            || gap >= TAU * self.radius.as_meters() / f64::from(self.segments)
        {
            return Err(RuleError::InvalidParameter {
                what: "circular surround and joints must fit inside the wall",
            });
        }
        Ok(())
    }
}

pub(crate) fn brick_recipe(
    [x0, z0, x1, z1]: [f64; 4],
    size: [f64; 3],
    opening: Option<CircularOpening>,
    gap: f64,
) -> Result<Option<Recipe>, RuleError> {
    let depth = size[1];
    let mut cut = None;
    if let Some(opening) = opening {
        let [cx, cz] = opening.center;
        let radius = opening.radius.as_meters() + opening.surround.as_meters() + gap;
        let far_x = (x0 - cx).abs().max((x1 - cx).abs());
        let far_z = (z0 - cz).abs().max((z1 - cz).abs());
        if far_x * far_x + far_z * far_z <= radius * radius {
            return Ok(None);
        }
        let near_x = cx.clamp(x0, x1) - cx;
        let near_z = cz.clamp(z0, z1) - cz;
        if near_x * near_x + near_z * near_z < radius * radius {
            cut = Some((cx - x0, cz - z0, radius));
        }
    }
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let mut root = builder.with_material(slot).add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size },
        placement: Placement3::IDENTITY,
    })?;
    if let Some((cx, cz, radius)) = cut {
        let profile =
            builder.add_profile(circle(radius).map_err(|_| RuleError::InvalidParameter {
                what: "opening circle",
            })?);
        let cutter = builder.with_material(slot).add(NodeKind::Extrude {
            profile,
            height: depth + 0.02,
            caps: CapMode::Both,
            placement: Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0],
                [cx, depth + 0.01, cz],
            ),
        })?;
        root = builder.add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![root, cutter],
        })?;
    }
    Ok(Some(builder.finish(root)?))
}

pub(crate) fn surround(
    ctx: &RuleContext<'_>,
    wall: &Element,
    opening: CircularOpening,
    gap: f64,
    out: &mut RuleOutput,
) -> Result<(), RuleError> {
    let inner = opening.radius.as_meters();
    let outer = inner + opening.surround.as_meters();
    let step = TAU / f64::from(opening.segments);
    let half = (step - gap / inner) * 0.5;
    let c = libm::cos(half);
    let s = libm::sin(half);
    let bulge = libm::sin(half * 0.5) / libm::cos(half * 0.5);
    // The local sector points right; its bounding box begins at inner*cos.
    let x0 = inner * c;
    let z0 = -outer * s;
    let profile = Profile2::simple(
        Loop2::new(vec![
            Seg2::line((outer * c - x0, -outer * s - z0)),
            Seg2::arc((outer * c - x0, outer * s - z0), bulge),
            Seg2::line((inner * c - x0, inner * s - z0)),
            Seg2::arc((inner * c - x0, -inner * s - z0), -bulge),
        ])
        .map_err(|_| RuleError::InvalidParameter {
            what: "surround sector",
        })?,
    )
    .map_err(|_| RuleError::InvalidParameter {
        what: "surround profile",
    })?;
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let profile = builder.add_profile(profile);
    let root = builder.with_material(slot).add(NodeKind::Extrude {
        profile,
        height: wall.extent.size[1],
        caps: CapMode::Both,
        placement: Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [0.0, wall.extent.size[1], 0.0],
        ),
    })?;
    let recipe = builder.finish(root)?;
    for i in 0..opening.segments {
        let angle = f64::from(i) * step;
        let [c, s] = [libm::cos(angle), libm::sin(angle)];
        let axis = |x, z| {
            core::array::from_fn(|k| wall.extent.axes[0][k] * x + wall.extent.axes[2][k] * z)
        };
        let extent = OrientedBox {
            origin: wall.extent.anchor([
                opening.center[0] + c * x0 - s * z0,
                0.0,
                opening.center[1] + s * x0 + c * z0,
            ]),
            axes: [axis(c, s), wall.extent.axes[1], axis(-s, c)],
            size: [outer - x0, wall.extent.size[1], -2.0 * z0],
        };
        out.generate(
            Element::new(
                &format!("{}-surround-{i}", ctx.relation().key),
                "masonry-surround",
                &wall.material,
                extent,
                ctx.relation().evidence.clone(),
            )
            .with_part(Part::new(recipe.clone())),
        );
    }
    Ok(())
}
