// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Common-rafter-to-purlin underside-seat rule.

use exedra_math::{scale, sub};
use joiner::{
    Applicability, Observation, PartEdit, Rule, RuleContext, RuleError, RuleOutput, TransferEdge,
    TransferKind, TransferTarget,
};

use super::crossing::{
    CrossingFootprint, FRAME_EPSILON, bearing_contact, crossing_footprint, cutter_overrun,
    not_applicable, unsuitable_from_error, validate_bearing_size, validate_complete_crossing,
    validate_cut_depth,
};
use super::principal_trench::PURLIN_ROLE;
use crate::length::default_millimeters;
use crate::participants::{ParticipantPair, resolve_pair};
use crate::tool::{nominal_rect, profile_tool_world, receiving_profile, world_to_local};
use crate::{FitClass, Length};

/// Stable identity for a common rafter seated over a purlin.
pub const COMMON_RAFTER_PURLIN_SEAT_RULE_KEY: &str = "joiner_timber:common-rafter-to-purlin-seat@1";

pub(super) const COMMON_RAFTER_ROLE: &str = "common-rafter";

/// Parameters for a common rafter's underside seat over one purlin.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CommonRafterPurlinSeatParams {
    /// Fit allowance applied only to the common rafter's receiving seat.
    pub fit: FitClass,
    /// Depth cut upward from the common rafter's lower face.
    pub seat_depth: Length,
    /// Least uncut common-rafter depth above the seat.
    ///
    /// This is a geometric safeguard, not a capacity check.
    pub minimum_remaining_depth: Length,
    /// Least clear timber between the seat and either common-rafter end.
    pub minimum_end_relish: Length,
    /// Least bearing dimension in either direction of the crossing footprint.
    pub minimum_bearing: Length,
}

impl Default for CommonRafterPurlinSeatParams {
    fn default() -> Self {
        Self {
            fit: FitClass::CLOSE,
            seat_depth: default_millimeters(20),
            minimum_remaining_depth: default_millimeters(100),
            minimum_end_relish: default_millimeters(80),
            minimum_bearing: default_millimeters(80),
        }
    }
}

impl CommonRafterPurlinSeatParams {
    /// Lowers this rule's exact dimensions together for construction geometry.
    fn lower_to_geometry(self) -> CommonRafterPurlinSeatGeometry {
        CommonRafterPurlinSeatGeometry {
            fit: self.fit,
            seat_depth: self.seat_depth.as_meters(),
            minimum_remaining_depth: self.minimum_remaining_depth.as_meters(),
            minimum_end_relish: self.minimum_end_relish.as_meters(),
            minimum_bearing: self.minimum_bearing.as_meters(),
        }
    }
}

#[derive(Copy, Clone)]
struct CommonRafterPurlinSeatGeometry {
    fit: FitClass,
    seat_depth: f64,
    minimum_remaining_depth: f64,
    minimum_end_relish: f64,
    minimum_bearing: f64,
}

/// Seats a common rafter over a full-section purlin.
///
/// The relation node is the centre of the finished bearing plane. Setout must
/// place it [`CommonRafterPurlinSeatParams::seat_depth`] above the common
/// rafter's uncut lower face and on the purlin's upper face. The rule removes
/// only the overlapping underside of the common rafter; the purlin remains
/// continuous.
#[derive(Copy, Clone, Debug, Default)]
pub struct CommonRafterToPurlinSeatRule;

impl Rule for CommonRafterToPurlinSeatRule {
    type Params = CommonRafterPurlinSeatParams;

    fn key(&self) -> &str {
        COMMON_RAFTER_PURLIN_SEAT_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        let pair = match resolve_pair(ctx, COMMON_RAFTER_ROLE, PURLIN_ROLE) {
            Ok(pair) => pair,
            Err(rejection) => return Applicability::Unsuitable(alloc::vec![rejection]),
        };
        let footprint = match assess_common_seat(&pair) {
            Ok(footprint) => footprint,
            Err(error) => return unsuitable_from_error(ctx, error),
        };
        Applicability::Suitable(alloc::vec![Observation::new(
            "common-rafter-seat",
            &pair.carried.key,
            &alloc::format!(
                "bearing footprint is {} m by {} m",
                footprint.size[0],
                footprint.size[1]
            ),
        )])
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        let pair = resolve_pair(ctx, COMMON_RAFTER_ROLE, PURLIN_ROLE)
            .map_err(|rejection| RuleError::NotApplicable(alloc::vec![rejection]))?;
        let params = params.lower_to_geometry();
        let footprint = validate_common_seat(&pair, &params)?;
        let nominal = nominal_rect(footprint.size[0], footprint.size[1])?;
        let receiving = receiving_profile(&nominal, params.fit)?;
        let overrun = cutter_overrun(params.fit);
        let relation = &ctx.relation().key;
        let evidence = ctx.relation().evidence.clone();

        // The tool rises from outside the common rafter to the purlin's top
        // face. Its upper cap is the only meaningful Boolean boundary; the
        // lower overrun keeps the uncut underside from coinciding with a
        // cutter face.
        let common_rafter_seat = profile_tool_world(
            &alloc::format!("{relation}-common-rafter-seat"),
            receiving,
            params.seat_depth + overrun,
            sub(
                footprint.origin,
                scale(footprint.normal, params.seat_depth + overrun),
            ),
            [
                footprint.tangents[0],
                footprint.tangents[1],
                footprint.normal,
            ],
            &pair.carried.extent,
        )?;

        let open_common_rafter_seat =
            PartEdit::remove(&pair.carried.key, common_rafter_seat, evidence.clone());
        let common_rafter_bears_on_purlin = bearing_contact(
            relation,
            &pair,
            &footprint,
            "seated-common-rafter",
            evidence,
        )
        .with_minimum_overlap_meters([footprint.size[0] * 0.8, footprint.size[1] * 0.8]);
        let route_common_rafter_load_to_purlin = TransferEdge::new(
            &alloc::format!("load-{}-through-{relation}", pair.carried.key),
            &pair.carried.key,
            TransferTarget::element(&pair.carrier.key),
            TransferKind::Contact,
        );

        let mut output = RuleOutput::new();
        output
            .edit(open_common_rafter_seat)
            .contact(common_rafter_bears_on_purlin)
            .transfer(route_common_rafter_load_to_purlin);
        Ok(output)
    }
}

/// Checks only the role and geometry that make a common-rafter seat possible.
///
/// The actual seat depth and strength safeguards belong to `instantiate`, so
/// custom parameters cannot be rejected merely for differing from defaults.
fn assess_common_seat(pair: &ParticipantPair<'_>) -> Result<CrossingFootprint, RuleError> {
    let footprint = crossing_footprint(pair)?;
    validate_complete_crossing(pair, &footprint)?;
    let common_node = world_to_local(&pair.carried.extent, pair.node.point);
    let purlin_node = world_to_local(&pair.carrier.extent, pair.node.point);
    if common_node[2] <= FRAME_EPSILON
        || common_node[2] >= pair.carried.extent.size[2] - FRAME_EPSILON
    {
        return Err(not_applicable(
            pair,
            "purlin does not overlap the common-rafter lower face by a usable depth",
        ));
    }
    if (purlin_node[2] - pair.carrier.extent.size[2]).abs() > FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "joint node is not on the purlin upper face",
        ));
    }
    Ok(footprint)
}

fn validate_common_seat(
    pair: &ParticipantPair<'_>,
    params: &CommonRafterPurlinSeatGeometry,
) -> Result<CrossingFootprint, RuleError> {
    validate_cut_depth(
        params.seat_depth,
        params.minimum_remaining_depth,
        pair.carried.extent.size[2],
        "seat depth",
    )?;
    let footprint = assess_common_seat(pair)?;
    validate_bearing_size(&footprint, params.minimum_bearing)?;

    let common_node = world_to_local(&pair.carried.extent, pair.node.point);
    let purlin_node = world_to_local(&pair.carrier.extent, pair.node.point);
    if (common_node[2] - params.seat_depth).abs() > FRAME_EPSILON {
        return Err(RuleError::InvalidParameter {
            what: "authored purlin overlap does not match common-rafter seat depth",
        });
    }
    if (purlin_node[2] - pair.carrier.extent.size[2]).abs() > FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "joint node is not on the purlin upper face",
        ));
    }

    // Tangent 1 follows the common rafter. Both ends need clear timber beyond
    // the fit-expanded notch so this remains an interior seat rather than an
    // accidental open-ended notch.
    let half_along = footprint.size[1] * 0.5 + params.fit.allowance_meters();
    let left = common_node[0] - half_along;
    let right = pair.carried.extent.size[0] - common_node[0] - half_along;
    if left < params.minimum_end_relish || right < params.minimum_end_relish {
        return Err(RuleError::Degenerate {
            what: "common-rafter seat needs timber beyond both ends",
        });
    }
    Ok(footprint)
}
