// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Full-section housed bearings for compression struts and rafter heads.

use exedra_math::{add, dot, scale, sub};
use joiner::{
    Anchor, Applicability, ContactMeaning, ContactPatch, Observation, PartEdit, Rejection,
    RejectionReason, Rule, RuleContext, RuleError, RuleOutput, TransferEdge, TransferKind,
    TransferTarget, Vec3,
};

use crate::length::default_millimeters;
use crate::participants::{EndpointPair, MemberEnd, resolve_endpoint_pair};
use crate::tool::{nominal_rect, profile_tool_world, receiving_profile, world_to_local};
use crate::{FitClass, Length};

/// Stable identity for a housed strut-to-king-post application.
pub const STRUT_KING_POST_RULE_KEY: &str = "joiner_timber:housed-strut-to-king-post@1";
/// Stable identity for a housed strut-to-rafter application.
pub const STRUT_RAFTER_RULE_KEY: &str = "joiner_timber:housed-strut-to-rafter@1";
/// Stable identity for a housed principal-rafter-to-king-post application.
pub const RAFTER_KING_POST_RULE_KEY: &str = "joiner_timber:housed-rafter-to-king-post@1";

const STRUT_ROLE: &str = "strut";
const KING_POST_ROLE: &str = "king-post";
const RAFTER_ROLE: &str = "principal-rafter";
const FRAME_EPSILON: f64 = 1.0e-9;

/// Parameters shared by full-section housed timber bearings.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct HousedBearingParams {
    /// Fit allowance applied only to the carrier's housing.
    pub fit: FitClass,
    /// Distance from the internal bearing shoulder to the exposed face.
    pub housing_depth: Length,
    /// Least carrier material around the bearing shoulder.
    ///
    /// This rejects visibly fragile geometry; it does not size the joint for
    /// a design force or replace the carrier's block-shear check.
    pub minimum_carrier_relish: Length,
}

impl Default for HousedBearingParams {
    fn default() -> Self {
        Self {
            fit: FitClass::CLOSE,
            housing_depth: default_millimeters(20),
            minimum_carrier_relish: default_millimeters(15),
        }
    }
}

impl HousedBearingParams {
    /// Lowers all exact dimensions together at the recipe-building boundary.
    fn lower_to_geometry(self) -> HousedBearingGeometry {
        HousedBearingGeometry {
            fit: self.fit,
            housing_depth: self.housing_depth.as_meters(),
            minimum_carrier_relish: self.minimum_carrier_relish.as_meters(),
        }
    }
}

/// Floating-point dimensions used only while fitting construction geometry.
#[derive(Copy, Clone)]
struct HousedBearingGeometry {
    fit: FitClass,
    housing_depth: f64,
    minimum_carrier_relish: f64,
}

/// Fits the lower end of a strut into a housing above the king-post foot.
///
/// The contact and transfer follow the compression strut into the king post.
#[derive(Copy, Clone, Debug, Default)]
pub struct StrutToKingPostRule;

/// Fits the upper end of a strut into a housing on a principal rafter.
///
/// Geometry still treats the terminating strut as the carried member, while
/// the contact records the loaded rafter bearing into that strut. No directed
/// transfer is added: this bearing closes the rafter/strut/king triangle, and
/// `joiner`'s acyclic support graph is not a truss-force solver.
#[derive(Copy, Clone, Debug, Default)]
pub struct StrutToRafterRule;

/// Fits one principal rafter into a deep bearing housing at the king-post head.
///
/// The contact routes the king post's tie-suspension path into the rafter; it
/// does not claim that roof gravity load terminates in the hanging post.
#[derive(Copy, Clone, Debug, Default)]
pub struct RafterToKingPostRule;

impl Rule for StrutToKingPostRule {
    type Params = HousedBearingParams;

    fn key(&self) -> &str {
        STRUT_KING_POST_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        assess(ctx, STRUT_ROLE, KING_POST_ROLE)
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        instantiate(
            ctx,
            params,
            STRUT_ROLE,
            KING_POST_ROLE,
            "strut-to-king-post",
            LoadRoute::CarriedIntoCarrier,
        )
    }
}

impl Rule for StrutToRafterRule {
    type Params = HousedBearingParams;

    fn key(&self) -> &str {
        STRUT_RAFTER_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        assess(ctx, STRUT_ROLE, RAFTER_ROLE)
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        instantiate(
            ctx,
            params,
            STRUT_ROLE,
            RAFTER_ROLE,
            "strut-to-rafter",
            LoadRoute::CarrierIntoCarriedContactOnly,
        )
    }
}

impl Rule for RafterToKingPostRule {
    type Params = HousedBearingParams;

    fn key(&self) -> &str {
        RAFTER_KING_POST_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        assess(ctx, RAFTER_ROLE, KING_POST_ROLE)
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        instantiate(
            ctx,
            params,
            RAFTER_ROLE,
            KING_POST_ROLE,
            "rafter-to-king-post-head",
            LoadRoute::CarrierIntoCarried,
        )
    }
}

/// How physical bearing ownership maps into the acyclic support explanation.
///
/// The first two variants also emit the corresponding directed transfer. The
/// contact-only form is for a real bearing that closes a structural triangle:
/// forcing its internal force into the support DAG would create a cycle and
/// falsely make that DAG look like a statics model.
#[derive(Copy, Clone)]
enum LoadRoute {
    CarriedIntoCarrier,
    CarrierIntoCarried,
    CarrierIntoCarriedContactOnly,
}

fn assess(
    ctx: &RuleContext<'_>,
    carried_role: &'static str,
    carrier_role: &'static str,
) -> Applicability {
    let pair = match resolve_endpoint_pair(ctx, carried_role, carrier_role) {
        Ok(pair) => pair,
        Err(rejection) => return Applicability::Unsuitable(alloc::vec![rejection]),
    };
    if let Err(what) = check_endpoint_section(&pair) {
        return Applicability::unsuitable(
            &ctx.relation().key,
            RejectionReason::Unsupported { what },
        );
    }
    Applicability::Suitable(alloc::vec![Observation::new(
        "housed-bearing-shoulder",
        &pair.carried.key,
        match pair.carried_end {
            MemberEnd::Start => "carried timber starts on a full-section bearing shoulder",
            MemberEnd::End => "carried timber ends on a full-section bearing shoulder",
        },
    )])
}

fn instantiate(
    ctx: &RuleContext<'_>,
    params: &HousedBearingParams,
    carried_role: &'static str,
    carrier_role: &'static str,
    detail: &'static str,
    load_route: LoadRoute,
) -> Result<RuleOutput, RuleError> {
    let pair = resolve_endpoint_pair(ctx, carried_role, carrier_role)
        .map_err(|rejection| RuleError::NotApplicable(alloc::vec![rejection]))?;
    check_endpoint_section(&pair).map_err(|what| {
        RuleError::NotApplicable(alloc::vec![Rejection::new(
            &ctx.relation().key,
            RejectionReason::Unsupported { what },
        )])
    })?;
    let params = params.lower_to_geometry();
    let outward = validate_params(&pair, &params)?;

    let relation = &ctx.relation().key;
    let evidence = ctx.relation().evidence.clone();
    let across = pair.carried.extent.axes[1];
    let depth = pair.carried.extent.axes[2];
    let full = [pair.carried.extent.size[1], pair.carried.extent.size[2]];
    // The tool only has to clear the carrier's exposed face. Extending it by
    // a member length makes unrelated housings intersect far outside the
    // timber, which changes the n-ary cutter topology despite leaving the
    // requested pocket unchanged. A fit-scale tail is explicit, sufficient,
    // and local to the joint being authored.
    let overrun = params.fit.allowance_meters().max(FRAME_EPSILON * 16.0);

    // The carried member already begins or ends on the internal shoulder.
    // Only the carrier is cut: a nominal full-section profile, offset for fit,
    // runs from that shoulder through the exposed face and beyond it.
    let nominal = nominal_rect(full[0], full[1])?;
    let receiving = receiving_profile(&nominal, params.fit)?;
    let carrier_housing_origin = sub(
        sub(pair.node.point, scale(across, full[0] * 0.5)),
        scale(depth, full[1] * 0.5),
    );
    let carrier_housing = profile_tool_world(
        &alloc::format!("{relation}-carrier-housing"),
        receiving,
        params.housing_depth + overrun,
        carrier_housing_origin,
        [across, depth, outward],
        &pair.carrier.extent,
    )?;

    // Geometry calls the member ending in the housing "carried", but that
    // does not always describe physical bearing ownership. At the strut head,
    // roof load bears from the rafter into the compression strut; at the king
    // head, the post suspends the tie from the rafters. The contact is reversed
    // in both cases while the role-selected cutter orientation stays fixed.
    let (bearing_source, bearing_support, bearing_normal) = match load_route {
        LoadRoute::CarriedIntoCarrier => (pair.carried, pair.carrier, outward),
        LoadRoute::CarrierIntoCarried | LoadRoute::CarrierIntoCarriedContactOnly => {
            (pair.carrier, pair.carried, scale(outward, -1.0))
        }
    };

    let open_carrier_housing =
        PartEdit::remove(&pair.carrier.key, carrier_housing, evidence.clone());
    let bearing_contact = ContactPatch::new(
        &alloc::format!("contact-{relation}"),
        Anchor::new(
            &bearing_source.key,
            world_to_local(&bearing_source.extent, pair.node.point),
        ),
        Anchor::new(
            &bearing_support.key,
            world_to_local(&bearing_support.extent, pair.node.point),
        ),
        bearing_normal,
        [across, depth],
        ContactMeaning::Bearing,
        evidence,
    )
    .with_minimum_overlap([full[0] * 0.8, full[1] * 0.8])
    .with_detail(detail);
    let load_to_support =
        (!matches!(load_route, LoadRoute::CarrierIntoCarriedContactOnly)).then(|| {
            TransferEdge::new(
                &alloc::format!("load-{}-through-{relation}", bearing_source.key),
                &bearing_source.key,
                TransferTarget::element(&bearing_support.key),
                TransferKind::Contact,
            )
        });

    let mut output = RuleOutput::new();
    output.edit(open_carrier_housing).contact(bearing_contact);
    if let Some(load_to_support) = load_to_support {
        output.transfer(load_to_support);
    }
    Ok(output)
}

fn check_endpoint_section(pair: &EndpointPair<'_>) -> Result<(), &'static str> {
    let local = world_to_local(&pair.carried.extent, pair.node.point);
    let expected = [
        match pair.carried_end {
            MemberEnd::Start => 0.0,
            MemberEnd::End => pair.carried.extent.size[0],
        },
        pair.carried.extent.size[1] * 0.5,
        pair.carried.extent.size[2] * 0.5,
    ];
    if local
        .iter()
        .zip(expected)
        .any(|(actual, expected)| (*actual - expected).abs() > FRAME_EPSILON)
    {
        return Err("carried-member endpoint node is not its section centre");
    }
    Ok(())
}

fn validate_params(
    pair: &EndpointPair<'_>,
    params: &HousedBearingGeometry,
) -> Result<Vec3, RuleError> {
    let clearance = params.fit.allowance_meters();

    let outward = match pair.carried_end {
        MemberEnd::Start => pair.carried.extent.axes[0],
        MemberEnd::End => scale(pair.carried.extent.axes[0], -1.0),
    };
    if !pair
        .carrier
        .extent
        .contains_point(pair.node.point, FRAME_EPSILON)
    {
        return Err(RuleError::InvalidParameter {
            what: "carried-member shoulder lies outside carrier material",
        });
    }
    let exit = distance_to_exit(&pair.carrier.extent, pair.node.point, outward).ok_or(
        RuleError::InvalidParameter {
            what: "housing does not open through one carrier face",
        },
    )?;
    if (exit - params.housing_depth).abs() > FRAME_EPSILON {
        return Err(RuleError::InvalidParameter {
            what: "authored carried-member endpoint does not match housing depth",
        });
    }

    // The expanded corners protect the complete internal bearing shoulder.
    // Checking only its centre would permit an oblique housing to break out
    // through a carrier side while still satisfying the axial depth.
    let across = pair.carried.extent.axes[1];
    let depth = pair.carried.extent.axes[2];
    let half = [
        pair.carried.extent.size[1] * 0.5 + clearance + params.minimum_carrier_relish,
        pair.carried.extent.size[2] * 0.5 + clearance + params.minimum_carrier_relish,
    ];
    for (across_sign, depth_sign) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = add(
            add(pair.node.point, scale(across, across_sign * half[0])),
            scale(depth, depth_sign * half[1]),
        );
        if !pair.carrier.extent.contains_point(corner, FRAME_EPSILON) {
            return Err(RuleError::Degenerate {
                what: "housed bearing needs carrier relish around its shoulder",
            });
        }
    }
    Ok(outward)
}

fn distance_to_exit(extent: &joiner::OrientedBox, point: Vec3, direction: Vec3) -> Option<f64> {
    if !extent.contains_point(point, FRAME_EPSILON) {
        return None;
    }
    let local = world_to_local(extent, point);
    let local_direction = extent.axes.map(|axis| dot(direction, axis));
    let mut distance = f64::INFINITY;
    for axis in 0..3 {
        let candidate = if local_direction[axis] > FRAME_EPSILON {
            (extent.size[axis] - local[axis]) / local_direction[axis]
        } else if local_direction[axis] < -FRAME_EPSILON {
            -local[axis] / local_direction[axis]
        } else {
            continue;
        };
        if candidate >= -FRAME_EPSILON {
            distance = distance.min(candidate.max(0.0));
        }
    }
    // An exact housing Length cannot guarantee that a ray through arbitrary
    // floating-point extent frames finds a finite exit. Infinity here means
    // no axis supplied an outward intersection, so there is no housing path.
    distance.is_finite().then_some(distance)
}

#[cfg(test)]
mod tests {
    use exedra_constructive::evaluate::{Severity, evaluate};
    use exedra_constructive::tessellate::EvalPolicy;
    use joiner::{
        Construction, Element, Evidence, EvidenceClass, EvidenceSource, Member, Node, OrientedBox,
        Relation, RelationKind, RuleApplication, compose,
    };

    use super::*;

    fn fixture(carried_role: &str, carrier_role: &str, carried_end: MemberEnd) -> Construction {
        let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
        let mut construction = Construction::new();
        construction
            .add_evidence_source(EvidenceSource::new(
                "fixture",
                EvidenceClass::ModernEngineeringInference,
                "https://example.invalid/housed-strut",
                "Deterministic housed-strut test fixture",
            ))
            .unwrap();

        let (shoulder, other): (Vec3, Vec3) = match carried_end {
            MemberEnd::Start => ([0.18, 0.0, 0.0], [1.0, 0.0, 0.0]),
            MemberEnd::End => ([-0.18, 0.0, 0.0], [-1.0, 0.0, 0.0]),
        };
        let strut_origin_x = shoulder[0].min(other[0]);
        construction
            .add_element(
                Element::new(
                    "carrier",
                    carrier_role,
                    "oak",
                    OrientedBox::axis_aligned([-0.20, -0.20, -0.20], [0.40, 0.40, 0.40]),
                    evidence.clone(),
                )
                .with_member(),
            )
            .unwrap();
        construction
            .add_element(
                Element::new(
                    "strut",
                    carried_role,
                    "oak",
                    OrientedBox::axis_aligned(
                        [strut_origin_x, -0.09, -0.08],
                        [(shoulder[0] - other[0]).abs(), 0.18, 0.16],
                    ),
                    evidence.clone(),
                )
                .with_member(),
            )
            .unwrap();
        construction
            .add_node(Node::new("shoulder", shoulder))
            .unwrap();
        construction.add_node(Node::new("other", other)).unwrap();
        construction
            .add_node(Node::new("carrier-other", [0.0, 0.0, 0.15]))
            .unwrap();
        let (from, to) = match carried_end {
            MemberEnd::Start => ("shoulder", "other"),
            MemberEnd::End => ("other", "shoulder"),
        };
        construction
            .add_member(Member::new("strut", "strut", from, to, evidence.clone()))
            .unwrap();
        construction
            .add_member(Member::new(
                "carrier",
                "carrier",
                "shoulder",
                "carrier-other",
                evidence.clone(),
            ))
            .unwrap();
        construction
            .add_relation(Relation::new(
                "strut-joint",
                RelationKind::member_member("shoulder", &["carrier", "strut"]),
                "housed-bearing",
                evidence,
            ))
            .unwrap();
        construction
    }

    #[test]
    fn both_rules_orient_the_housing_from_shoulder_to_carrier_face() {
        // The king-post fit is at the strut start and the rafter fit is at its
        // far end; each must orient its cutter outward without reflecting
        // participant order into a geometric convention.
        for (role, end, rule) in [
            (KING_POST_ROLE, MemberEnd::Start, STRUT_KING_POST_RULE_KEY),
            (RAFTER_ROLE, MemberEnd::End, STRUT_RAFTER_RULE_KEY),
        ] {
            let construction = fixture(STRUT_ROLE, role, end);
            let ctx = RuleContext::new(&construction, "strut-joint").unwrap();
            let output = if rule == STRUT_KING_POST_RULE_KEY {
                StrutToKingPostRule.instantiate(&ctx, &HousedBearingParams::default())
            } else {
                StrutToRafterRule.instantiate(&ctx, &HousedBearingParams::default())
            }
            .unwrap();
            assert_eq!(output.part_edits.len(), 1, "{rule}");
            assert_eq!(output.contacts.len(), 1, "{rule}");
            assert_eq!(
                output.transfers.len(),
                usize::from(rule == STRUT_KING_POST_RULE_KEY),
                "only the strut foot contributes a directed support edge"
            );
        }
    }

    #[test]
    fn strut_head_records_bearing_without_closing_a_directed_cycle() {
        // The strut remains the geometric carried member because it ends in
        // the rafter housing, but physical bearing ownership runs from rafter
        // to strut. The contact is retained without pretending that one edge
        // can represent the cyclic internal force system of the whole truss.
        let construction = fixture(STRUT_ROLE, RAFTER_ROLE, MemberEnd::End);
        let ctx = RuleContext::new(&construction, "strut-joint").unwrap();
        let output = StrutToRafterRule
            .instantiate(&ctx, &HousedBearingParams::default())
            .unwrap();
        assert_eq!(output.contacts[0].carried.element, "carrier");
        assert_eq!(output.contacts[0].carrier.element, "strut");
        assert!(output.transfers.is_empty());
    }

    #[test]
    fn rafter_head_uses_the_same_explicit_bearing_contract() {
        // A principal rafter terminates at the far-end shoulder just like the
        // upper end of a strut, but the king post hangs from that rafter. Pin
        // both the distinct rule identity and the reversed load direction.
        let construction = fixture(RAFTER_ROLE, KING_POST_ROLE, MemberEnd::End);
        let ctx = RuleContext::new(&construction, "strut-joint").unwrap();
        let output = RafterToKingPostRule
            .instantiate(&ctx, &HousedBearingParams::default())
            .unwrap();
        assert_eq!(output.part_edits.len(), 1);
        assert_eq!(output.contacts[0].detail, "rafter-to-king-post-head");
        assert_eq!(output.transfers[0].from, "carrier");
        assert_eq!(output.transfers[0].to, TransferTarget::element("strut"));
        assert_eq!(RafterToKingPostRule.key(), RAFTER_KING_POST_RULE_KEY);
    }

    #[test]
    fn applied_housed_strut_compiles_both_members_without_diagnostics() {
        // This pins the actual top-open housing subtraction and confirms that
        // the untouched strut terminates exactly on its internal bearing face.
        let mut construction = fixture(STRUT_ROLE, KING_POST_ROLE, MemberEnd::Start);
        let ctx = RuleContext::new(&construction, "strut-joint").unwrap();
        let output = StrutToKingPostRule
            .instantiate(&ctx, &HousedBearingParams::default())
            .unwrap();
        construction
            .apply(RuleApplication::new(
                "fit-strut",
                STRUT_KING_POST_RULE_KEY,
                "strut-joint",
                Evidence::new("fixture", EvidenceClass::ModernEngineeringInference),
                output,
            ))
            .unwrap();
        for key in ["strut", "carrier"] {
            let recipe = compose(&construction, construction.element(key).unwrap()).unwrap();
            let evaluated = evaluate(&recipe, &EvalPolicy::default()).unwrap();
            assert_eq!(
                evaluated.bodies.len(),
                1,
                "{key}: {:?}",
                evaluated.report.diagnostics
            );
            assert!(evaluated.report.clean_at(Severity::Warning), "{key}");
            assert!(evaluated.bodies[0].body.mesh.validate_deep().is_empty());
        }
    }

    #[test]
    fn housing_depth_is_part_of_the_authored_setout_contract() {
        // Changing only the depth parameter would move the carrier face away
        // from the relation-derived shoulder, so the rule must refuse instead
        // of silently changing the strut's effective length.
        let construction = fixture(STRUT_ROLE, KING_POST_ROLE, MemberEnd::Start);
        let ctx = RuleContext::new(&construction, "strut-joint").unwrap();
        let params = HousedBearingParams {
            housing_depth: Length::millimeters(30).unwrap(),
            ..HousedBearingParams::default()
        };
        assert!(matches!(
            StrutToKingPostRule.instantiate(&ctx, &params),
            Err(RuleError::InvalidParameter {
                what: "authored carried-member endpoint does not match housing depth"
            })
        ));
    }
}
