// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! King-post-to-tie rule: a through tenon secured by a transverse key.

use exedra_math::{add, dot, scale, sub};
use joiner::{
    Anchor, Applicability, ContactMeaning, ContactPatch, Element, Observation, PartEdit, Rejection,
    RejectionReason, Rule, RuleContext, RuleError, RuleOutput, TransferEdge, TransferKind,
    TransferTarget,
};

use crate::length::default_millimeters;
use crate::participants::{ParticipantPair, resolve_pair};
use crate::tool::{nominal_rect, profile_tool_world, receiving_profile, world_to_local};
use crate::{FitClass, Length};

/// Stable identity recorded on keyed king-post-to-tie applications.
pub const KING_POST_TIE_RULE_KEY: &str = "joiner_timber:keyed-king-post-tie@1";

const KING_POST_ROLE: &str = "king-post";
const TIE_ROLE: &str = "tie-beam";
const KEY_ROLE: &str = "king-post-key";
const FRAME_EPSILON: f64 = 1.0e-9;

/// Parameters for a keyed through tenon between a king post and tie beam.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct KingPostTieParams {
    /// Fit allowance applied to the through mortise and across the key slot.
    ///
    /// The key slot remains line-to-line in the post direction: its upper
    /// and lower faces are the two bearing interfaces that suspend the tie.
    /// Clearance there would make the rectangular key incapable of touching
    /// both faces at once.
    pub fit: FitClass,
    /// Nominal tenon width in the post's local y direction.
    ///
    /// The keyed through tenon is a blade: it retains the post's complete
    /// local-z depth so the shoulders stay on the two otherwise unworked
    /// faces used by the truss's other housings.
    pub tenon_width: Length,
    /// Distance from the tenon tip to the relation node on top of the tie.
    ///
    /// This must cross the complete tie depth and leave room below it for the
    /// key plus [`Self::minimum_tip_relish`].
    pub tenon_length: Length,
    /// Key width along the post's local z direction.
    pub key_width: Length,
    /// Key height along the post's local x direction.
    pub key_height: Length,
    /// Distance the key projects past each side of the full post section.
    pub key_projection: Length,
    /// Least uncut tenon below the key slot.
    pub minimum_tip_relish: Length,
    /// Least uncut tie material around the mortise in plan.
    pub minimum_mortise_relish: Length,
}

impl Default for KingPostTieParams {
    fn default() -> Self {
        Self {
            fit: FitClass::CLOSE,
            tenon_width: default_millimeters(120),
            tenon_length: default_millimeters(440),
            key_width: default_millimeters(50),
            key_height: default_millimeters(40),
            key_projection: default_millimeters(40),
            minimum_tip_relish: default_millimeters(40),
            minimum_mortise_relish: default_millimeters(20),
        }
    }
}

impl KingPostTieParams {
    /// Lowers all exact dimensions together at the recipe-building boundary.
    fn lower_to_geometry(self) -> KingPostTieGeometry {
        KingPostTieGeometry {
            fit: self.fit,
            tenon_width: self.tenon_width.as_meters(),
            tenon_length: self.tenon_length.as_meters(),
            key_width: self.key_width.as_meters(),
            key_height: self.key_height.as_meters(),
            key_projection: self.key_projection.as_meters(),
            minimum_tip_relish: self.minimum_tip_relish.as_meters(),
            minimum_mortise_relish: self.minimum_mortise_relish.as_meters(),
        }
    }
}

/// Floating-point dimensions used only while fitting construction geometry.
#[derive(Copy, Clone)]
struct KingPostTieGeometry {
    fit: FitClass,
    tenon_width: f64,
    tenon_length: f64,
    key_width: f64,
    key_height: f64,
    key_projection: f64,
    minimum_tip_relish: f64,
    minimum_mortise_relish: f64,
}

/// Cuts an inspectable through tenon and generates its transverse timber key.
///
/// The relation node is the shoulder on top of the tie. The post begins
/// [`KingPostTieParams::tenon_length`] below it at the exposed tenon tip. A
/// through mortise exposes the tenon below the tie. A
/// generated key passes through a matching slot immediately under the tie,
/// leaving explicit tip relish below it.
///
/// The two contacts state the tension mechanism without pretending that the
/// post's top shoulder carries it: the tie bears on the key, and the key bears
/// on the bottom of the tenon slot. Parameter checks are geometric safeguards,
/// not connection-capacity design.
#[derive(Copy, Clone, Debug, Default)]
pub struct KingPostTieRule;

impl Rule for KingPostTieRule {
    type Params = KingPostTieParams;

    fn key(&self) -> &str {
        KING_POST_TIE_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        let pair = match resolve_pair(ctx, KING_POST_ROLE, TIE_ROLE) {
            Ok(pair) => pair,
            Err(rejection) => return Applicability::Unsuitable(alloc::vec![rejection]),
        };
        if let Err(what) = check_shoulder_section(&pair) {
            return Applicability::unsuitable(
                &ctx.relation().key,
                RejectionReason::Unsupported { what },
            );
        }
        Applicability::Suitable(alloc::vec![Observation::new(
            "through-tenon-shoulder",
            &pair.carried.key,
            "relation lies on the post section centre at the tie top",
        )])
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        let pair = resolve_pair(ctx, KING_POST_ROLE, TIE_ROLE)
            .map_err(|rejection| RuleError::NotApplicable(alloc::vec![rejection]))?;
        check_shoulder_section(&pair).map_err(|what| {
            RuleError::NotApplicable(alloc::vec![Rejection::new(
                &ctx.relation().key,
                RejectionReason::Unsupported { what },
            )])
        })?;
        let params = params.lower_to_geometry();
        let interface = validate_params(&pair, &params)?;

        let relation = &ctx.relation().key;
        let evidence = ctx.relation().evidence.clone();
        let insertion = pair.carried.extent.axes[0];
        let across = pair.carried.extent.axes[1];
        let depth = pair.carried.extent.axes[2];
        let full = [pair.carried.extent.size[1], pair.carried.extent.size[2]];
        let overrun = full
            .into_iter()
            .chain(pair.carrier.extent.size)
            .fold(params.tenon_length, f64::max);
        let tenon_tip = sub(pair.node.point, scale(insertion, params.tenon_length));

        // Two simple side slabs leave a full-depth blade tenon. Unlike an
        // annular cutter, neither slab has an inner loop that terminates on a
        // long post face; their only exact boundary is the shoulder plane.
        let tenon_side = (full[0] - params.tenon_width) * 0.5;
        let section_origin = sub(
            sub(tenon_tip, scale(across, full[0] * 0.5)),
            scale(depth, full[1] * 0.5),
        );
        let post_side_low_cut = profile_tool_world(
            &alloc::format!("{relation}-post-shoulder-low"),
            nominal_rect(tenon_side + overrun, full[1] + 2.0 * overrun)?,
            params.tenon_length + overrun,
            sub(
                sub(
                    sub(section_origin, scale(across, overrun)),
                    scale(depth, overrun),
                ),
                scale(insertion, overrun),
            ),
            [across, depth, insertion],
            &pair.carried.extent,
        )?;
        let post_side_high_cut = profile_tool_world(
            &alloc::format!("{relation}-post-shoulder-high"),
            nominal_rect(tenon_side + overrun, full[1] + 2.0 * overrun)?,
            params.tenon_length + overrun,
            sub(
                add(
                    section_origin,
                    scale(across, tenon_side + params.tenon_width),
                ),
                add(scale(depth, overrun), scale(insertion, overrun)),
            ),
            [across, depth, insertion],
            &pair.carried.extent,
        )?;

        // The tie mortise deliberately overruns both faces. Its mating size
        // still comes only from the nominal tenon plus the typed fit offset.
        let nominal_tenon = nominal_rect(params.tenon_width, full[1])?;
        let receiving_tenon = receiving_profile(&nominal_tenon, params.fit)?;
        let mortise_origin = add(
            sub(
                sub(interface.shoulder, scale(across, params.tenon_width * 0.5)),
                scale(depth, full[1] * 0.5),
            ),
            scale(insertion, overrun),
        );
        let tie_through_mortise = profile_tool_world(
            &alloc::format!("{relation}-tie-through-mortise"),
            receiving_tenon,
            interface.tie_depth + 2.0 * overrun,
            mortise_origin,
            [across, depth, scale(insertion, -1.0)],
            &pair.carrier.extent,
        )?;

        // The key's top face is tight to the tie underside and its bottom
        // face bears on the tenon-slot shoulder. Clearance therefore belongs
        // only across the key, where it permits insertion. Applying the
        // generic all-around profile offset here would enlarge the slot in
        // the load direction and leave the rectangular key floating above
        // the lower contact claimed below.
        let key_clearance = params.fit.allowance_meters();
        let receiving_key_slot =
            nominal_rect(params.key_width + 2.0 * key_clearance, params.key_height)?;
        let key_bottom = sub(interface.tie_underside, scale(insertion, params.key_height));
        let slot_origin = sub(
            sub(
                key_bottom,
                scale(depth, params.key_width * 0.5 + key_clearance),
            ),
            scale(across, overrun),
        );
        let tenon_key_slot = profile_tool_world(
            &alloc::format!("{relation}-tenon-key-slot"),
            receiving_key_slot,
            full[0] + 2.0 * overrun,
            slot_origin,
            [depth, insertion, across],
            &pair.carried.extent,
        )?;

        let key_length = full[0] + 2.0 * params.key_projection;
        let key_origin = sub(
            sub(key_bottom, scale(across, key_length * 0.5)),
            scale(depth, params.key_width * 0.5),
        );
        let key_name = alloc::format!("{relation}-key");
        let key_extent = joiner::OrientedBox {
            origin: key_origin,
            axes: [across, depth, insertion],
            size: [key_length, params.key_width, params.key_height],
        };

        let trim_post_side_low =
            PartEdit::remove(&pair.carried.key, post_side_low_cut, evidence.clone());
        let trim_post_side_high =
            PartEdit::remove(&pair.carried.key, post_side_high_cut, evidence.clone());
        let open_tie_mortise =
            PartEdit::remove(&pair.carrier.key, tie_through_mortise, evidence.clone());
        let open_tenon_key_slot =
            PartEdit::remove(&pair.carried.key, tenon_key_slot, evidence.clone());
        let transverse_key = Element::new(
            &key_name,
            KEY_ROLE,
            &pair.carried.material,
            key_extent.clone(),
            evidence.clone(),
        );

        // The generated key is structural rather than illustrative: the two
        // contacts and transfers below are the explicit suspension path from
        // the tie, through the key, into the post tenon.
        let tie_on_key_point = interface.tie_underside;
        let tie_bears_on_key = ContactPatch::new(
            &alloc::format!("contact-{relation}-tie-on-key"),
            Anchor::new(
                &pair.carrier.key,
                world_to_local(&pair.carrier.extent, tie_on_key_point),
            ),
            Anchor::new(&key_name, world_to_local(&key_extent, tie_on_key_point)),
            insertion,
            [across, depth],
            ContactMeaning::Bearing,
            evidence.clone(),
        )
        .with_minimum_overlap_meters([full[0] * 0.8, params.key_width * 0.8])
        .with_detail("tie-underside-on-transverse-key");
        let key_on_tenon_point = key_bottom;
        let key_bears_on_tenon = ContactPatch::new(
            &alloc::format!("contact-{relation}-key-on-tenon"),
            Anchor::new(&key_name, world_to_local(&key_extent, key_on_tenon_point)),
            Anchor::new(
                &pair.carried.key,
                world_to_local(&pair.carried.extent, key_on_tenon_point),
            ),
            insertion,
            [across, depth],
            ContactMeaning::Shoulder,
            evidence,
        )
        .with_minimum_overlap_meters([params.tenon_width * 0.8, params.key_width * 0.8])
        .with_detail("key-on-tenon-slot-bottom");
        let route_tie_load_to_key = TransferEdge::new(
            &alloc::format!("load-{}-through-{relation}-key", pair.carrier.key),
            &pair.carrier.key,
            TransferTarget::element(&key_name),
            TransferKind::Contact,
        );
        let route_key_load_to_post = TransferEdge::new(
            &alloc::format!("load-{key_name}-through-{relation}-tenon"),
            &key_name,
            TransferTarget::element(&pair.carried.key),
            TransferKind::Contact,
        );

        let mut output = RuleOutput::new();
        output
            .edit(trim_post_side_low)
            .edit(trim_post_side_high)
            .edit(open_tie_mortise)
            .edit(open_tenon_key_slot)
            .generate(transverse_key)
            .contact(tie_bears_on_key)
            .contact(key_bears_on_tenon)
            .transfer(route_tie_load_to_key)
            .transfer(route_key_load_to_post);
        Ok(output)
    }
}

struct KingPostTieInterface {
    shoulder: joiner::Vec3,
    tie_underside: joiner::Vec3,
    tie_depth: f64,
}

fn check_shoulder_section(pair: &ParticipantPair<'_>) -> Result<(), &'static str> {
    let local = world_to_local(&pair.carried.extent, pair.node.point);
    let expected_section = [
        pair.carried.extent.size[1] * 0.5,
        pair.carried.extent.size[2] * 0.5,
    ];
    if local[0] <= FRAME_EPSILON
        || local[0] >= pair.carried.extent.size[0] - FRAME_EPSILON
        || local[1..]
            .iter()
            .zip(expected_section)
            .any(|(actual, expected)| (*actual - expected).abs() > FRAME_EPSILON)
    {
        return Err("king-post shoulder node is not on its interior section centre");
    }
    Ok(())
}

fn validate_params(
    pair: &ParticipantPair<'_>,
    params: &KingPostTieGeometry,
) -> Result<KingPostTieInterface, RuleError> {
    let clearance = params.fit.allowance_meters();
    if params.tenon_width >= pair.carried.extent.size[1] {
        return Err(RuleError::InvalidParameter {
            what: "tenon must leave post shoulders",
        });
    }
    if params.tenon_length >= pair.carried.extent.size[0] {
        return Err(RuleError::InvalidParameter {
            what: "tenon length",
        });
    }
    if params.key_width + 2.0 * clearance >= pair.carried.extent.size[2] {
        return Err(RuleError::InvalidParameter {
            what: "key slot must leave tenon side relish",
        });
    }

    let tie_depth = pair.carrier.extent.size[2];
    if params.tenon_length
        < tie_depth + params.key_height + params.minimum_tip_relish - FRAME_EPSILON
    {
        return Err(RuleError::InvalidParameter {
            what: "tenon must cross tie and leave keyed tip relish",
        });
    }

    let insertion = pair.carried.extent.axes[0];
    let shoulder = pair.node.point;
    let carried_shoulder = world_to_local(&pair.carried.extent, shoulder);
    if (carried_shoulder[0] - params.tenon_length).abs() > FRAME_EPSILON {
        return Err(RuleError::InvalidParameter {
            what: "post start does not match tenon length below shoulder",
        });
    }
    let carrier_shoulder = world_to_local(&pair.carrier.extent, shoulder);
    let outward = dot(insertion, pair.carrier.extent.axes[2]);
    let carrier_face = if outward >= 0.0 {
        pair.carrier.extent.size[2]
    } else {
        0.0
    };
    if outward.abs() < 1.0 - FRAME_EPSILON
        || (carrier_shoulder[2] - carrier_face).abs() > FRAME_EPSILON
    {
        return Err(RuleError::InvalidParameter {
            what: "post shoulder must lie on the tie face normal to the post",
        });
    }
    let protected = [
        params.tenon_width * 0.5 + clearance + params.minimum_mortise_relish,
        pair.carried.extent.size[2] * 0.5 + clearance + params.minimum_mortise_relish,
    ];
    if carrier_shoulder[0] < protected[1]
        || pair.carrier.extent.size[0] - carrier_shoulder[0] < protected[1]
        || carrier_shoulder[1] < protected[0]
        || pair.carrier.extent.size[1] - carrier_shoulder[1] < protected[0]
    {
        return Err(RuleError::Degenerate {
            what: "through mortise needs tie relish around its plan perimeter",
        });
    }

    Ok(KingPostTieInterface {
        shoulder,
        tie_underside: sub(shoulder, scale(insertion, tie_depth)),
        tie_depth,
    })
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

    fn fixture() -> Construction {
        let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
        let mut construction = Construction::new();
        construction
            .add_evidence_source(EvidenceSource::new(
                "fixture",
                EvidenceClass::ModernEngineeringInference,
                "https://example.invalid/keyed-king-post-tie",
                "Deterministic keyed king-post-to-tie test fixture",
            ))
            .unwrap();
        let params = KingPostTieParams::default();
        let tip = [0.0, 0.0, 0.30 - params.tenon_length.as_meters()];
        construction
            .add_element(
                Element::new(
                    "tie",
                    TIE_ROLE,
                    "oak",
                    OrientedBox::axis_aligned([-0.5, -0.15, 0.0], [1.0, 0.30, 0.30]),
                    evidence.clone(),
                )
                .with_member(),
            )
            .unwrap();
        construction
            .add_element(
                Element::new(
                    "post",
                    KING_POST_ROLE,
                    "oak",
                    OrientedBox {
                        origin: [-0.13, -0.13, tip[2]],
                        axes: [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                        size: [1.30 - tip[2], 0.26, 0.26],
                    },
                    evidence.clone(),
                )
                .with_member(),
            )
            .unwrap();
        construction.add_node(Node::new("tip", tip)).unwrap();
        construction
            .add_node(Node::new("shoulder", [0.0, 0.0, 0.30]))
            .unwrap();
        construction
            .add_node(Node::new("top", [0.0, 0.0, 1.30]))
            .unwrap();
        construction
            .add_node(Node::new("tie-end", [0.45, 0.0, 0.30]))
            .unwrap();
        construction
            .add_member(Member::new("post", "post", "tip", "top", evidence.clone()))
            .unwrap();
        construction
            .add_member(Member::new(
                "tie",
                "tie",
                "shoulder",
                "tie-end",
                evidence.clone(),
            ))
            .unwrap();
        construction
            .add_relation(Relation::new(
                "post-tie",
                RelationKind::member_member("shoulder", &["tie", "post"]),
                "keyed-through-tenon",
                evidence,
            ))
            .unwrap();
        construction
    }

    #[test]
    fn keyed_through_tenon_exposes_its_tension_mechanism() {
        // The result must cut the post shoulder, tie mortise, and key slot,
        // then generate the separate key and route tie load through both of
        // its bearing faces rather than through a fictitious post shoulder.
        let construction = fixture();
        let ctx = RuleContext::new(&construction, "post-tie").unwrap();
        let output = KingPostTieRule
            .instantiate(&ctx, &KingPostTieParams::default())
            .unwrap();
        assert_eq!(output.part_edits.len(), 4);
        assert_eq!(output.generated.len(), 1);
        assert_eq!(output.generated[0].role, KEY_ROLE);
        assert_eq!(output.contacts.len(), 2);
        assert_eq!(output.transfers.len(), 2);
        assert!(
            output
                .contacts
                .iter()
                .all(|contact| contact.meaning.carries_load())
        );
        assert!(
            output
                .transfers
                .iter()
                .all(|transfer| transfer.kind == TransferKind::Contact)
        );
    }

    #[test]
    fn key_slot_clearance_is_transverse_to_its_bearing_faces() {
        // The key must simultaneously touch the tie underside and the lower
        // tenon-slot shoulder. Pin the emitted slot, not just its parameters:
        // fit widens the slot across the key while leaving its load-direction
        // height exactly equal to the generated key height.
        let construction = fixture();
        let ctx = RuleContext::new(&construction, "post-tie").unwrap();
        let params = KingPostTieParams::default();
        let output = KingPostTieRule.instantiate(&ctx, &params).unwrap();
        let slot = output.part_edits[3].op.tool();
        let exedra_constructive::ir::NodeKind::Extrude { profile, .. } =
            &slot.recipe.node(slot.recipe.root()).unwrap().kind
        else {
            panic!("key slot tool is one extrusion");
        };
        let profile = slot.recipe.profile(*profile).unwrap();
        let (min_x, max_x, min_y, max_y) = profile.outer().segs().iter().fold(
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ),
            |(min_x, max_x, min_y, max_y), segment| {
                (
                    min_x.min(segment.to.x),
                    max_x.max(segment.to.x),
                    min_y.min(segment.to.y),
                    max_y.max(segment.to.y),
                )
            },
        );
        let clearance = params.fit.allowance_meters();
        let key_width = params.key_width.as_meters();
        let key_height = params.key_height.as_meters();
        assert!((max_x - min_x - (key_width + 2.0 * clearance)).abs() < 1.0e-12);
        assert!((max_y - min_y - key_height).abs() < 1.0e-12);

        let post = construction.element("post").unwrap();
        let key = &output.generated[0];
        let key_bottom = world_to_local(&post.extent, key.extent.origin)[0];
        let key_top = world_to_local(
            &post.extent,
            add(
                key.extent.origin,
                scale(key.extent.axes[2], key.extent.size[2]),
            ),
        )[0];
        let slot_bottom = slot.placement.rows[0][3];
        let slot_top = slot_bottom + slot.placement.rows[0][1] * key_height;
        assert!((slot_bottom - key_bottom).abs() < 1.0e-12);
        assert!((slot_top - key_top).abs() < 1.0e-12);
    }

    #[test]
    fn applied_keyed_joint_compiles_every_piece_without_diagnostics() {
        // This exercises the actual Boolean stack, including the key slot
        // crossing the already reduced through tenon.
        let mut construction = fixture();
        let ctx = RuleContext::new(&construction, "post-tie").unwrap();
        let output = KingPostTieRule
            .instantiate(&ctx, &KingPostTieParams::default())
            .unwrap();
        construction
            .apply(RuleApplication::new(
                "fit-post-tie",
                KING_POST_TIE_RULE_KEY,
                "post-tie",
                Evidence::new("fixture", EvidenceClass::ModernEngineeringInference),
                output,
            ))
            .unwrap();

        for key in ["post", "tie", "post-tie-key"] {
            let element = construction.element(key).unwrap();
            let recipe = compose(&construction, element).unwrap();
            let evaluated = evaluate(&recipe, &EvalPolicy::default()).unwrap();
            assert_eq!(
                evaluated.bodies.len(),
                1,
                "{key}: {:?}",
                evaluated.report.diagnostics
            );
            assert!(evaluated.report.clean_at(Severity::Warning), "{key}");
            assert!(
                evaluated.bodies[0].body.mesh.validate_deep().is_empty(),
                "{key}"
            );
        }
    }

    #[test]
    fn shallow_tenon_cannot_hide_the_key_or_erase_tip_relish() {
        // A key drawn inside the tie or at the tenon tip would look plausible
        // in isolation but could not suspend the tie as the rule claims.
        let mut construction = fixture();
        let params = KingPostTieParams {
            tenon_length: Length::millimeters(360).unwrap(),
            ..KingPostTieParams::default()
        };
        construction
            .set_element_extent(
                "post",
                OrientedBox {
                    origin: [-0.13, -0.13, -0.06],
                    axes: [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    size: [1.36, 0.26, 0.26],
                },
            )
            .unwrap();
        let ctx = RuleContext::new(&construction, "post-tie").unwrap();
        assert!(matches!(
            KingPostTieRule.instantiate(&ctx, &params),
            Err(RuleError::InvalidParameter {
                what: "tenon must cross tie and leave keyed tip relish"
            })
        ));
    }

    #[test]
    fn shoulder_must_land_on_the_tie_top_face() {
        // The authored relation node determines the complete axial setout;
        // the rule refuses a shoulder floating inside the tie rather than
        // moving it silently to make the Boolean succeed.
        let mut construction = fixture();
        construction
            .set_element_extent(
                "tie",
                OrientedBox::axis_aligned([-0.5, -0.15, 0.0], [1.0, 0.30, 0.31]),
            )
            .unwrap();
        let ctx = RuleContext::new(&construction, "post-tie").unwrap();
        assert!(matches!(
            KingPostTieRule.instantiate(&ctx, &KingPostTieParams::default()),
            Err(RuleError::InvalidParameter {
                what: "post shoulder must lie on the tie face normal to the post"
            })
        ));
    }
}
