// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Housed heel rule: a horizontal seat on a tie member.

use exedra_math::{add, dot, norm, normalize, scale, sub};
use joiner::{
    Anchor, Applicability, ContactMeaning, ContactPatch, Observation, PartEdit, Rejection,
    RejectionReason, Rule, RuleContext, RuleError, RuleOutput, Vec3,
};

use crate::length::default_millimeters;
use crate::participants::{ParticipantPair, resolve_pair};
use crate::tool::{nominal_rect, profile_tool_world, receiving_profile, world_to_local};
use crate::{FitClass, Length};

/// Stable identity recorded on housed-heel rule applications.
pub const HEEL_RULE_KEY: &str = "joiner_timber:housed-heel@1";

const RAFTER_ROLE: &str = "principal-rafter";
const TIE_ROLE: &str = "tie-beam";
const FRAME_EPSILON: f64 = 1.0e-9;

/// Parameters for one housed heel.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct HeelParams {
    /// Fit allowance applied only to the tie's receiving housing.
    pub fit: FitClass,
    /// Depth cut into the tie, measured normal to its upper face.
    pub housing_depth: Length,
    /// Smallest acceptable run of the derived bearing seat.
    pub minimum_seat_run: Length,
    /// Least uncut carrier material around every side of the housing in plan.
    ///
    /// This is a geometric safeguard, not a capacity calculation. A real
    /// joint still needs its end relish sized for the timber and design load.
    pub minimum_relish: Length,
}

impl Default for HeelParams {
    fn default() -> Self {
        Self {
            fit: FitClass::CLOSE,
            housing_depth: default_millimeters(25),
            minimum_seat_run: default_millimeters(60),
            minimum_relish: default_millimeters(10),
        }
    }
}

impl HeelParams {
    /// Lowers all exact dimensions together at the recipe-building boundary.
    fn lower_to_geometry(self) -> HeelGeometry {
        HeelGeometry {
            fit: self.fit,
            housing_depth: self.housing_depth.as_meters(),
            minimum_seat_run: self.minimum_seat_run.as_meters(),
            minimum_relish: self.minimum_relish.as_meters(),
        }
    }
}

/// Floating-point dimensions used only while fitting construction geometry.
#[derive(Copy, Clone)]
struct HeelGeometry {
    fit: FitClass,
    housing_depth: f64,
    minimum_seat_run: f64,
    minimum_relish: f64,
}

/// Cuts a principal-rafter heel and its matching housing in a tie beam.
///
/// The relation node is the rafter section centre at its start. Intersecting
/// that rafter with the tie's upper-face plane derives one rectangular seat:
/// its width is the rafter width and its run follows from the rafter pitch and
/// depth. The rafter is cut exactly to that plane. The tie housing extrudes
/// the same profile after applying [`HeelParams::fit`].
///
/// The bearing patch records where rafter thrust is resolved, but this rule
/// does not add a directed gravity-transfer edge. A heel closes the rafter/tie
/// force triangle; treating that chord force as another downstream support
/// route creates a cycle in `joiner`'s deliberately acyclic load-path graph.
#[derive(Copy, Clone, Debug, Default)]
pub struct HeelRule;

impl Rule for HeelRule {
    type Params = HeelParams;

    fn key(&self) -> &str {
        HEEL_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        let pair = match resolve_pair(ctx, RAFTER_ROLE, TIE_ROLE) {
            Ok(pair) => pair,
            Err(rejection) => return Applicability::Unsuitable(alloc::vec![rejection]),
        };
        // Applicability is geometric; use the documented default housing
        // depth only to report a useful derived run. Custom parameter
        // failures remain typed instantiate errors.
        let interface = match derive_interface(
            &pair,
            HeelParams::default().lower_to_geometry().housing_depth,
        ) {
            Ok(interface) => interface,
            Err(what) => {
                return Applicability::unsuitable(
                    &ctx.relation().key,
                    RejectionReason::Unsupported { what },
                );
            }
        };
        Applicability::Suitable(alloc::vec![Observation::new(
            "derived-seat-run",
            &pair.carried.key,
            &alloc::format!("bearing run is {} m", interface.run),
        )])
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        let pair = resolve_pair(ctx, RAFTER_ROLE, TIE_ROLE)
            .map_err(|rejection| RuleError::NotApplicable(alloc::vec![rejection]))?;
        let params = params.lower_to_geometry();
        if params.housing_depth >= pair.carrier.extent.size[2] {
            return Err(RuleError::InvalidParameter {
                what: "housing depth",
            });
        }
        let interface = derive_interface(&pair, params.housing_depth).map_err(|what| {
            RuleError::NotApplicable(alloc::vec![Rejection::new(
                &ctx.relation().key,
                RejectionReason::Unsupported { what },
            )])
        })?;
        if interface.run < params.minimum_seat_run {
            return Err(RuleError::Degenerate {
                what: "heel seat run",
            });
        }

        let nominal = nominal_rect(interface.width, interface.run)?;
        let receiving = receiving_profile(&nominal, params.fit)?;
        let clearance = params.fit.allowance_meters();
        // A housed seat is surrounded by carrier material in plan. If its
        // offset profile reaches an end or side face, the result is an open
        // notch/slot and must be named by a different rule rather than
        // silently emitted as a housed heel.
        let protected_margin = clearance + params.minimum_relish;
        for (across, along) in [
            (-protected_margin, -protected_margin),
            (interface.width + protected_margin, -protected_margin),
            (-protected_margin, interface.run + protected_margin),
            (
                interface.width + protected_margin,
                interface.run + protected_margin,
            ),
        ] {
            let corner = add(
                add(interface.origin, scale(interface.across, across)),
                scale(interface.along_seat, along),
            );
            if !pair.carrier.extent.contains_point(corner, FRAME_EPSILON) {
                return Err(RuleError::Degenerate {
                    what: "housed heel needs carrier relish around its plan perimeter",
                });
            }
        }
        let relation = &ctx.relation().key;
        let evidence = ctx.relation().evidence.clone();

        // The cutter deliberately overruns every non-interface face. Flush
        // cutter walls would turn irrelevant member boundaries into Boolean
        // contacts; the top face remains exactly the nominal bearing plane.
        let overrun = pair
            .carried
            .extent
            .size
            .into_iter()
            .chain(pair.carrier.extent.size)
            .fold(0.0_f64, f64::max);
        let housing_overrun = pair.carrier.extent.size[2].max(pair.carried.extent.size[2]);
        let seat_cut_origin = sub(
            sub(interface.origin, scale(interface.across, overrun)),
            scale(interface.along_seat, overrun),
        );
        let seat_cut_profile = nominal_rect(
            interface.width + 2.0 * overrun,
            interface.run + 2.0 * overrun,
        )?;
        let rafter_above_seat = profile_tool_world(
            &alloc::format!("{relation}-rafter-above-seat"),
            seat_cut_profile,
            overrun * 2.0,
            seat_cut_origin,
            [interface.across, interface.along_seat, interface.normal],
            &pair.carried.extent,
        )?;
        // The receiver is a top-open housing. Its bottom is the exact seat
        // plane used by the rafter cutter; starting at the upper face avoids
        // the visually plausible but mechanically wrong enclosed end slot.
        let housing_origin = add(
            interface.origin,
            scale(interface.normal, params.housing_depth + housing_overrun),
        );
        let tie_housing = profile_tool_world(
            &alloc::format!("{relation}-tie-housing"),
            receiving,
            params.housing_depth + housing_overrun,
            housing_origin,
            [
                interface.across,
                interface.along_seat,
                scale(interface.normal, -1.0),
            ],
            &pair.carrier.extent,
        )?;

        let contact_point = add(
            add(
                interface.origin,
                scale(interface.across, interface.width * 0.5),
            ),
            scale(interface.along_seat, interface.run * 0.5),
        );
        let trim_rafter_to_seat =
            PartEdit::retain(&pair.carried.key, rafter_above_seat, evidence.clone());
        let open_tie_housing = PartEdit::remove(&pair.carrier.key, tie_housing, evidence.clone());
        let heel_bearing = ContactPatch::new(
            &alloc::format!("contact-{relation}"),
            Anchor::new(
                &pair.carried.key,
                world_to_local(&pair.carried.extent, contact_point),
            ),
            Anchor::new(
                &pair.carrier.key,
                world_to_local(&pair.carrier.extent, contact_point),
            ),
            interface.normal,
            [interface.across, interface.along_seat],
            ContactMeaning::Bearing,
            evidence,
        )
        .with_minimum_overlap_meters([interface.width * 0.8, interface.run * 0.8])
        .with_detail("housed-heel-seat");

        let mut output = RuleOutput::new();
        output
            .edit(trim_rafter_to_seat)
            .edit(open_tie_housing)
            .contact(heel_bearing);
        Ok(output)
    }
}

struct HeelInterface {
    origin: Vec3,
    across: Vec3,
    along_seat: Vec3,
    normal: Vec3,
    width: f64,
    run: f64,
}

/// Derives the complete bearing rectangle from the rafter and tie frames.
///
/// The seat plane is `housing_depth` below the carrier's upper face. Its
/// intersection with the finite rafter prism yields both ends of the bearing
/// run, including cases where the relation node lies below that plane. No
/// second width/run is authored for the housing.
fn derive_interface(
    pair: &ParticipantPair<'_>,
    housing_depth: f64,
) -> Result<HeelInterface, &'static str> {
    let local_node = world_to_local(&pair.carried.extent, pair.node.point);
    let expected = [
        0.0,
        pair.carried.extent.size[1] * 0.5,
        pair.carried.extent.size[2] * 0.5,
    ];
    if local_node
        .iter()
        .zip(expected)
        .any(|(actual, expected)| (*actual - expected).abs() > FRAME_EPSILON)
    {
        return Err("rafter start node is not its section centre");
    }

    let along = pair.carried.extent.axes[0];
    let across = pair.carried.extent.axes[1];
    let depth_axis = pair.carried.extent.axes[2];
    let mut normal = pair.carrier.extent.axes[2];
    if dot(along, normal) < 0.0 {
        normal = scale(normal, -1.0);
    }
    let rise = dot(along, normal);
    let depth_normal = dot(depth_axis, normal);
    if rise <= FRAME_EPSILON || depth_normal.abs() <= FRAME_EPSILON {
        return Err("heel needs a pitched rafter above the carrier face");
    }
    if dot(across, normal).abs() > FRAME_EPSILON {
        return Err("rafter width axis is not tangent to the bearing plane");
    }
    let horizontal = sub(along, scale(normal, rise));
    let along_seat = normalize(horizontal).ok_or("heel has no bearing-plane run")?;
    if dot(across, along_seat).abs() > FRAME_EPSILON {
        return Err("rafter width and seat-run axes are not orthogonal");
    }
    let carrier_node = world_to_local(&pair.carrier.extent, pair.node.point);
    let top_distance = if dot(normal, pair.carrier.extent.axes[2]) >= 0.0 {
        pair.carrier.extent.size[2] - carrier_node[2]
    } else {
        carrier_node[2]
    };
    let seat_delta = top_distance - housing_depth;
    let half_depth = pair.carried.extent.size[2] * 0.5;
    let mut first_t = (seat_delta - depth_normal * half_depth) / rise;
    let mut last_t = (seat_delta + depth_normal * half_depth) / rise;
    if first_t > last_t {
        core::mem::swap(&mut first_t, &mut last_t);
    }
    first_t = first_t.max(0.0);
    last_t = last_t.min(pair.carried.extent.size[0]);
    if last_t - first_t <= FRAME_EPSILON {
        return Err("housing bottom does not intersect the finite rafter");
    }
    let intersection_point = |t: f64| {
        let depth_offset = (seat_delta - t * rise) / depth_normal;
        add(
            add(pair.node.point, scale(along, t)),
            scale(depth_axis, depth_offset),
        )
    };
    let mut first = intersection_point(first_t);
    let mut last = intersection_point(last_t);
    if dot(sub(last, first), along_seat) < 0.0 {
        core::mem::swap(&mut first, &mut last);
    }
    let run = dot(sub(last, first), along_seat);
    // `run` is derived from floating-point extent frames and a plane/prism
    // intersection, not copied from an exact rule Length. Keep the kernel
    // result check even though authored dimensions cannot be NaN or infinite.
    if !(run.is_finite() && run > FRAME_EPSILON) {
        return Err("derived heel seat is degenerate");
    }
    debug_assert!(
        (norm(sub(last, first)) - run).abs() < FRAME_EPSILON,
        "seat endpoints must differ only along the derived in-plane axis"
    );
    let origin = sub(first, scale(across, pair.carried.extent.size[1] * 0.5));
    let contact_center = add(first, scale(along_seat, run * 0.5));
    if !pair
        .carrier
        .extent
        .contains_point(contact_center, FRAME_EPSILON)
    {
        return Err("derived heel seat leaves the tie extent");
    }
    Ok(HeelInterface {
        origin,
        across,
        along_seat,
        normal,
        width: pair.carried.extent.size[1],
        run,
    })
}

#[cfg(test)]
mod tests {
    use exedra_constructive::evaluate::{Severity, evaluate};
    use exedra_constructive::tessellate::EvalPolicy;
    use exedra_math::{cross, normalize};
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
                "https://example.invalid/heel",
                "Deterministic housed-heel test fixture",
            ))
            .unwrap();
        let heel = [0.4, 0.0, 0.3];
        let apex = [1.8, 0.0, 1.35];
        let delta = sub(apex, heel);
        let length = norm(delta);
        let along = scale(delta, 1.0 / length);
        let across = [0.0, 1.0, 0.0];
        let depth = normalize(cross(along, across)).unwrap();
        construction
            .add_element(
                Element::new(
                    "tie",
                    TIE_ROLE,
                    "oak",
                    OrientedBox::axis_aligned([0.0, -0.15, 0.0], [2.0, 0.3, 0.3]),
                    evidence.clone(),
                )
                .with_member(),
            )
            .unwrap();
        construction
            .add_element(
                Element::new(
                    "rafter",
                    RAFTER_ROLE,
                    "oak",
                    OrientedBox {
                        origin: sub(sub(heel, scale(across, 0.10)), scale(depth, 0.12)),
                        axes: [along, across, depth],
                        size: [length, 0.20, 0.24],
                    },
                    evidence.clone(),
                )
                .with_member(),
            )
            .unwrap();
        construction.add_node(Node::new("heel", heel)).unwrap();
        construction.add_node(Node::new("apex", apex)).unwrap();
        construction
            .add_node(Node::new("tie-end", [1.8, 0.0, 0.3]))
            .unwrap();
        construction
            .add_member(Member::new(
                "rafter",
                "rafter",
                "heel",
                "apex",
                evidence.clone(),
            ))
            .unwrap();
        construction
            .add_member(Member::new(
                "tie",
                "tie",
                "heel",
                "tie-end",
                evidence.clone(),
            ))
            .unwrap();
        construction
            .add_relation(Relation::new(
                "heel-joint",
                RelationKind::member_member("heel", &["tie", "rafter"]),
                "bearing-seat",
                evidence,
            ))
            .unwrap();
        construction
    }

    #[test]
    fn heel_derives_both_edits_and_offsets_only_the_housing() {
        // The receiving cutter must grow by exactly twice the per-side fit
        // while the carried-side seat remains the nominal rafter section.
        let construction = fixture();
        let ctx = RuleContext::new(&construction, "heel-joint").unwrap();
        let params = HeelParams::default();
        let output = HeelRule.instantiate(&ctx, &params).unwrap();
        assert_eq!(output.part_edits.len(), 2);
        assert_eq!(output.part_edits[0].target, "rafter");
        assert_eq!(output.part_edits[1].target, "tie");

        let housing = output.part_edits[1].op.tool();
        // The housing starts above the tie rather than exactly coplanar with
        // its top face. Its extrusion still ends on the z=0.275 seat plane;
        // the overrun avoids a meaningless flush Boolean boundary.
        let exedra_constructive::ir::NodeKind::Extrude { height, .. } =
            &housing.recipe.node(housing.recipe.root()).unwrap().kind
        else {
            panic!("housing tool is one extrusion");
        };
        assert!((housing.placement.rows[2][3] - height - 0.275).abs() < 1.0e-12);
        let evaluated = evaluate(&housing.recipe, &EvalPolicy::default()).unwrap();
        let positions = &evaluated.bodies[0].body.mesh;
        let (min_x, max_x) = positions
            .vertices()
            .filter_map(|vertex| positions.vertex_position(vertex))
            .map(|point| f64::from(point[0]))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
                (min.min(value), max.max(value))
            });
        let clearance = params.fit.allowance_meters();
        assert!((min_x + clearance).abs() < 1.0e-6);
        assert!((max_x - (0.20 + clearance)).abs() < 1.0e-6);
    }

    #[test]
    fn applied_heel_compiles_both_members_without_diagnostics() {
        // This is the end-to-end Boolean regression: applying the coordinated
        // cuts must leave both edited member recipes as one valid solid.
        let mut construction = fixture();
        let ctx = RuleContext::new(&construction, "heel-joint").unwrap();
        let output = HeelRule.instantiate(&ctx, &HeelParams::default()).unwrap();
        construction
            .apply(RuleApplication::new(
                "fit-heel",
                HEEL_RULE_KEY,
                "heel-joint",
                Evidence::new("fixture", EvidenceClass::ModernEngineeringInference),
                output,
            ))
            .unwrap();

        for key in ["rafter", "tie"] {
            let element = construction.element(key).unwrap();
            let recipe = compose(&construction, element).unwrap();
            let evaluated = evaluate(&recipe, &EvalPolicy::default()).unwrap();
            assert_eq!(evaluated.bodies.len(), 1, "{key}");
            assert!(evaluated.report.clean_at(Severity::Warning), "{key}");
            assert!(
                evaluated.bodies[0].body.mesh.validate_deep().is_empty(),
                "{key}"
            );
        }
    }

    #[test]
    fn housed_heel_rejects_a_receiver_without_end_relish() {
        // Moving the tie boundary onto the heel would turn its top-open
        // housing into an end-breaking notch. That is a different joint, so
        // this rule must refuse it rather than emitting the slit the specimen
        // render originally exposed.
        let mut construction = fixture();
        construction
            .set_element_extent(
                "tie",
                OrientedBox::axis_aligned([0.419, -0.15, 0.0], [1.581, 0.3, 0.3]),
            )
            .unwrap();
        let ctx = RuleContext::new(&construction, "heel-joint").unwrap();
        assert!(matches!(
            HeelRule.instantiate(&ctx, &HeelParams::default()),
            Err(RuleError::Degenerate { .. })
        ));
    }

    #[test]
    fn line_to_line_fit_still_requires_real_carrier_relish() {
        // Fit allowance and structural relish are independent: selecting an
        // exact fit must not let a housing touch the carrier's end face.
        let mut construction = fixture();
        construction
            .set_element_extent(
                "tie",
                OrientedBox::axis_aligned([0.41, -0.15, 0.0], [1.59, 0.3, 0.3]),
            )
            .unwrap();
        let ctx = RuleContext::new(&construction, "heel-joint").unwrap();
        let params = HeelParams {
            fit: FitClass::LineToLine,
            ..HeelParams::default()
        };
        assert!(matches!(
            HeelRule.instantiate(&ctx, &params),
            Err(RuleError::Degenerate { .. })
        ));
    }

    #[test]
    fn heel_refuses_a_relation_at_the_rafter_far_end() {
        // The first slice deliberately supports the member start only; a far
        // end must be a typed refusal, not a reflected or misplaced cutter.
        let mut construction = fixture();
        construction
            .add_relation(Relation::new(
                "far-heel",
                RelationKind::member_member("apex", &["tie", "rafter"]),
                "bearing-seat",
                Evidence::new("fixture", EvidenceClass::ModernEngineeringInference),
            ))
            .unwrap();
        let ctx = RuleContext::new(&construction, "far-heel").unwrap();
        assert!(!HeelRule.assess(&ctx).is_suitable());
    }
}
