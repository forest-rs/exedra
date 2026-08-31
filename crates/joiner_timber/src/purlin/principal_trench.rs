// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Purlin-to-principal through-trench rule.

use joiner::{
    Applicability, Observation, PartEdit, Rule, RuleContext, RuleError, RuleOutput, TransferEdge,
    TransferKind, TransferTarget,
};

use super::crossing::{
    CrossingFootprint, FRAME_EPSILON, bearing_contact, crossing_footprint, cutter_overrun,
    not_applicable, unsuitable_from_error, validate_bearing_size, validate_complete_crossing,
    validate_cut_depth,
};
use crate::length::default_millimeters;
use crate::participants::{ParticipantPair, resolve_pair};
use crate::tool::{nominal_rect, profile_tool_world, receiving_profile, world_to_local};
use crate::{FitClass, Length};

/// Stable identity for a purlin trenched into a principal rafter.
pub const PURLIN_PRINCIPAL_TRENCH_RULE_KEY: &str = "joiner_timber:purlin-to-principal-trench@1";

pub(super) const PURLIN_ROLE: &str = "purlin";
pub(super) const PRINCIPAL_RAFTER_ROLE: &str = "principal-rafter";

/// Parameters for a through trench carrying one purlin.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PurlinPrincipalTrenchParams {
    /// Fit allowance applied only to the principal rafter's trench.
    pub fit: FitClass,
    /// Depth cut into the principal rafter's upper face.
    pub trench_depth: Length,
    /// Least uncut principal-rafter depth below the trench.
    ///
    /// This is a geometric safeguard, not a capacity check.
    pub minimum_remaining_depth: Length,
    /// Least clear timber between the trench and either principal-rafter end.
    pub minimum_end_relish: Length,
    /// Least bearing dimension in either direction of the crossing footprint.
    pub minimum_bearing: Length,
}

impl Default for PurlinPrincipalTrenchParams {
    fn default() -> Self {
        Self {
            fit: FitClass::CLOSE,
            trench_depth: default_millimeters(30),
            minimum_remaining_depth: default_millimeters(120),
            minimum_end_relish: default_millimeters(80),
            minimum_bearing: default_millimeters(80),
        }
    }
}

impl PurlinPrincipalTrenchParams {
    /// Lowers this rule's exact dimensions together for construction geometry.
    fn lower_to_geometry(self) -> PurlinPrincipalTrenchGeometry {
        PurlinPrincipalTrenchGeometry {
            fit: self.fit,
            trench_depth: self.trench_depth.as_meters(),
            minimum_remaining_depth: self.minimum_remaining_depth.as_meters(),
            minimum_end_relish: self.minimum_end_relish.as_meters(),
            minimum_bearing: self.minimum_bearing.as_meters(),
        }
    }
}

#[derive(Copy, Clone)]
struct PurlinPrincipalTrenchGeometry {
    fit: FitClass,
    trench_depth: f64,
    minimum_remaining_depth: f64,
    minimum_end_relish: f64,
    minimum_bearing: f64,
}

/// Trenches a principal rafter so a full-section purlin bears in it.
///
/// The relation node is the centre of the finished bearing plane. Setout must
/// place the purlin's lower face at that plane, overlapping the uncut
/// principal by [`PurlinPrincipalTrenchParams::trench_depth`]. The rule cuts
/// only the principal. This makes the authored overlap explicit and prevents
/// a rule from silently moving either member to manufacture contact.
#[derive(Copy, Clone, Debug, Default)]
pub struct PurlinToPrincipalTrenchRule;

impl Rule for PurlinToPrincipalTrenchRule {
    type Params = PurlinPrincipalTrenchParams;

    fn key(&self) -> &str {
        PURLIN_PRINCIPAL_TRENCH_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        let pair = match resolve_pair(ctx, PURLIN_ROLE, PRINCIPAL_RAFTER_ROLE) {
            Ok(pair) => pair,
            Err(rejection) => return Applicability::Unsuitable(alloc::vec![rejection]),
        };
        let footprint = match assess_purlin_trench(&pair) {
            Ok(footprint) => footprint,
            Err(error) => return unsuitable_from_error(ctx, error),
        };
        Applicability::Suitable(alloc::vec![Observation::new(
            "purlin-trench-bearing",
            &pair.carrier.key,
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
        let pair = resolve_pair(ctx, PURLIN_ROLE, PRINCIPAL_RAFTER_ROLE)
            .map_err(|rejection| RuleError::NotApplicable(alloc::vec![rejection]))?;
        let params = params.lower_to_geometry();
        let footprint = validate_purlin_trench(&pair, &params)?;
        let nominal = nominal_rect(footprint.size[0], footprint.size[1])?;
        let receiving = receiving_profile(&nominal, params.fit)?;
        let overrun = cutter_overrun(params.fit);
        let relation = &ctx.relation().key;
        let evidence = ctx.relation().evidence.clone();

        // The purlin already occupies the requested trench depth. Starting
        // the tool on its lower face makes that one plane survive as both the
        // trench bottom and the structural contact; only the irrelevant open
        // top is overrun for robust Boolean classification.
        let principal_trench = profile_tool_world(
            &alloc::format!("{relation}-principal-trench"),
            receiving,
            params.trench_depth + overrun,
            footprint.origin,
            [
                footprint.tangents[0],
                footprint.tangents[1],
                footprint.normal,
            ],
            &pair.carrier.extent,
        )?;

        let open_principal_trench =
            PartEdit::remove(&pair.carrier.key, principal_trench, evidence.clone());
        let purlin_bears_on_principal = bearing_contact(
            relation,
            &pair,
            &footprint,
            "trenched-purlin-bearing",
            evidence,
        )
        .with_minimum_overlap_meters([footprint.size[0] * 0.8, footprint.size[1] * 0.8]);
        let route_purlin_load_to_principal = TransferEdge::new(
            &alloc::format!("load-{}-through-{relation}", pair.carried.key),
            &pair.carried.key,
            TransferTarget::element(&pair.carrier.key),
            TransferKind::Contact,
        );

        let mut output = RuleOutput::new();
        output
            .edit(open_principal_trench)
            .contact(purlin_bears_on_principal)
            .transfer(route_purlin_load_to_principal);
        Ok(output)
    }
}

/// Checks only the role and geometry that make a purlin trench possible.
///
/// `Rule::assess` must not bake in [`PurlinPrincipalTrenchParams::default`]:
/// the overlap is an authored fact, while the caller supplies the matching cut
/// depth later to `instantiate`.
fn assess_purlin_trench(pair: &ParticipantPair<'_>) -> Result<CrossingFootprint, RuleError> {
    let footprint = crossing_footprint(pair)?;
    validate_complete_crossing(pair, &footprint)?;
    let purlin_node = world_to_local(&pair.carried.extent, pair.node.point);
    let principal_node = world_to_local(&pair.carrier.extent, pair.node.point);
    if purlin_node[2].abs() > FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "joint node is not on the purlin lower face",
        ));
    }
    let overlap = pair.carrier.extent.size[2] - principal_node[2];
    if overlap <= FRAME_EPSILON || overlap >= pair.carrier.extent.size[2] - FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "purlin does not overlap the principal upper face by a usable depth",
        ));
    }
    Ok(footprint)
}

fn validate_purlin_trench(
    pair: &ParticipantPair<'_>,
    params: &PurlinPrincipalTrenchGeometry,
) -> Result<CrossingFootprint, RuleError> {
    validate_cut_depth(
        params.trench_depth,
        params.minimum_remaining_depth,
        pair.carrier.extent.size[2],
        "trench depth",
    )?;
    let footprint = assess_purlin_trench(pair)?;
    validate_bearing_size(&footprint, params.minimum_bearing)?;

    let purlin_node = world_to_local(&pair.carried.extent, pair.node.point);
    let principal_node = world_to_local(&pair.carrier.extent, pair.node.point);
    if purlin_node[2].abs() > FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "joint node is not on the purlin lower face",
        ));
    }
    let expected_principal_depth = pair.carrier.extent.size[2] - params.trench_depth;
    if (principal_node[2] - expected_principal_depth).abs() > FRAME_EPSILON {
        return Err(RuleError::InvalidParameter {
            what: "authored purlin overlap does not match trench depth",
        });
    }

    // Tangent 0 follows the principal because the purlin section-width axis
    // is aligned to principal length. Protecting both ends keeps this an
    // internal through trench rather than an accidental open-ended notch.
    let half_along = footprint.size[0] * 0.5 + params.fit.allowance_meters();
    let left = principal_node[0] - half_along;
    let right = pair.carrier.extent.size[0] - principal_node[0] - half_along;
    if left < params.minimum_end_relish || right < params.minimum_end_relish {
        return Err(RuleError::Degenerate {
            what: "principal-rafter trench needs timber beyond both ends",
        });
    }
    Ok(footprint)
}
