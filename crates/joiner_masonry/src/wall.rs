// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{format, vec};

use joiner::{
    Applicability, Element, OrientedBox, Part, Rejection, RejectionReason, RelationKind, Rule,
    RuleContext, RuleError, RuleOutput,
};

use crate::Length;
use crate::opening::{CircularOpening, brick_recipe, surround};

/// Dimensions of running-bond units; wall thickness comes from its extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunningBondParams {
    /// Nominal unit length, excluding the head joint.
    pub unit_length: Length,
    /// Nominal unit height, excluding the bed joint.
    pub unit_height: Length,
    /// Open mortar gap between units, in both course directions.
    pub joint: Length,
    /// Minimum rectangular end closure or final course before any opening cut.
    /// Small remainders are redistributed across the last two cut units.
    /// Must fit the unit height and the initial half unit, `(length-joint)/2`.
    pub minimum_closure: Length,
    /// Optional circular passage and masonry surround, in wall-local X/Z.
    pub opening: Option<CircularOpening>,
}

/// Generates a single thickness of running-bond units and an optional surround.
///
/// The relation must be an empty `ElementUnits` relation whose present whole
/// has no part. Local X runs along the wall, Y through its thickness and Z up.
/// The wall's finite orthonormal extent supplies all placements. Courses begin
/// at local Z=0; odd courses start with half a module. Units are named by
/// relation/course/column, including skipped cells inside the opening.
///
/// Generated units inherit the wall's opaque material key and the relation's
/// evidence. Surround units have role `masonry-surround`, ordinary units
/// `masonry-unit`; callers can bind materials by those roles after applying.
/// No mortar solid, contact coverage or capacity is inferred across open gaps.
/// Layouts above 100,000 estimated cells or without a legal rectangular closure
/// are refused before returning any generated elements.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunningBondRule;

impl Rule for RunningBondRule {
    type Params = RunningBondParams;

    fn key(&self) -> &str {
        "joiner_masonry:running-bond@1"
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        match wall(ctx) {
            Ok(_) => Applicability::suitable(),
            Err(reason) => Applicability::Unsuitable(vec![reason]),
        }
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        let wall = wall(ctx).map_err(|reason| RuleError::NotApplicable(vec![reason]))?;
        let [width, depth, height] = wall.extent.size;
        let length = params.unit_length.as_meters();
        let unit_height = params.unit_height.as_meters();
        let gap = params.joint.as_meters();
        let minimum = params.minimum_closure.as_meters();
        if gap >= length.min(unit_height)
            || minimum > ((length - gap) * 0.5).min(unit_height)
            || width < minimum
            || height < minimum
        {
            return Err(RuleError::InvalidParameter {
                what: "joint and minimum closure must fit the units and wall",
            });
        }
        // Bound authored work before allocating: a malformed dimension must
        // not request billions of generated elements.
        if (width / (length + gap) + 2.0) * (height / (unit_height + gap) + 1.0) > 100_000.0 {
            return Err(RuleError::InvalidParameter {
                what: "wall exceeds 100000 unit cells",
            });
        }
        if let Some(opening) = params.opening {
            opening.validate(width, height, gap)?;
        }
        let mut out = RuleOutput::new();
        let mut z = 0.0;
        let mut course = 0_u32;
        while z < height {
            let top = closure_end(z, unit_height, gap, height, minimum)?;
            let mut x = 0.0;
            let mut column = 0_u32;
            while x < width {
                let stock = if column == 0 && !course.is_multiple_of(2) {
                    (length - gap) * 0.5
                } else {
                    length
                };
                let end = closure_end(x, stock, gap, width, minimum)?;
                let start = x;
                let bounds = [start, z, end, top];
                let size = [
                    if end == x + length {
                        length
                    } else {
                        end - start
                    },
                    depth,
                    if top == z + unit_height {
                        unit_height
                    } else {
                        top - z
                    },
                ];
                if let Some(recipe) = brick_recipe(bounds, size, params.opening, gap)? {
                    let extent = OrientedBox {
                        origin: wall.extent.anchor([start, 0.0, z]),
                        axes: wall.extent.axes,
                        size,
                    };
                    out.generate(
                        Element::new(
                            &format!("{}-course-{course}-unit-{column}", ctx.relation().key),
                            "masonry-unit",
                            &wall.material,
                            extent,
                            ctx.relation().evidence.clone(),
                        )
                        .with_part(Part::new(recipe)),
                    );
                }
                x = end + gap;
                column += 1;
            }
            z = top + gap;
            course += 1;
        }
        if let Some(opening) = params.opening {
            surround(ctx, wall, opening, gap, &mut out)?;
        }
        Ok(out)
    }
}

fn closure_end(
    start: f64,
    size: f64,
    gap: f64,
    limit: f64,
    minimum: f64,
) -> Result<f64, RuleError> {
    let end = if start + size >= limit {
        limit
    } else if limit - (start + size) - gap < minimum {
        // Two shorter cuts keep the stock-size bound and the mortar gap.
        start + (limit - start - gap) * 0.5
    } else {
        start + size
    };
    if end - start < minimum {
        return Err(RuleError::InvalidParameter {
            what: "wall cannot close with the requested minimum cut and joint",
        });
    }
    Ok(end)
}

fn wall<'a>(ctx: &'a RuleContext<'_>) -> Result<&'a Element, Rejection> {
    let reject = |what| {
        Rejection::new(
            &ctx.relation().key,
            RejectionReason::MissingCapability { what },
        )
    };
    let RelationKind::ElementUnits { whole, units } = &ctx.relation().kind else {
        return Err(Rejection::new(
            &ctx.relation().key,
            RejectionReason::WrongRelationKind {
                expected: "element/units",
                found: ctx.relation().kind.label(),
            },
        ));
    };
    if !units.is_empty() {
        return Err(reject("an unexpanded relation"));
    }
    let wall = ctx
        .element(whole)
        .ok_or_else(|| reject("a registered whole wall"))?;
    if !wall.present || wall.part.is_some() {
        return Err(reject("a present wall without a geometry part"));
    }
    if !wall.extent.is_well_formed() {
        return Err(reject("a finite orthonormal wall extent"));
    }
    Ok(wall)
}
