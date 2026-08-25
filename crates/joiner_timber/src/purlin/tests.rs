// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::evaluate::{Severity, evaluate};
use exedra_constructive::tessellate::EvalPolicy;
use joiner::{
    Construction, Element, Evidence, EvidenceClass, EvidenceSource, Member, Node, OrientedBox,
    Relation, RelationKind, Rule, RuleApplication, RuleContext, RuleError, compose,
};

use super::principal_trench::{PRINCIPAL_RAFTER_ROLE, PURLIN_ROLE};
use super::rafter_seat::COMMON_RAFTER_ROLE;
use super::*;
use crate::length::default_millimeters;

fn fixture() -> Construction {
    let evidence = Evidence::new("fixture", EvidenceClass::RegionalAnalogy);
    let mut construction = Construction::new();
    construction
        .add_evidence_source(EvidenceSource::new(
            "fixture",
            EvidenceClass::RegionalAnalogy,
            "https://example.invalid/purlin-crossings",
            "Deterministic trenched-purlin test fixture",
        ))
        .unwrap();
    construction
        .add_element(
            Element::new(
                "principal",
                PRINCIPAL_RAFTER_ROLE,
                "oak",
                OrientedBox {
                    origin: [-0.10, -1.0, 0.0],
                    axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                    size: [2.0, 0.20, 0.30],
                },
                evidence.clone(),
            )
            .with_member(),
        )
        .unwrap();
    construction
        .add_element(
            Element::new(
                "common",
                COMMON_RAFTER_ROLE,
                "oak",
                OrientedBox {
                    origin: [-0.06, -1.0, 0.47],
                    axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                    size: [2.0, 0.12, 0.16],
                },
                evidence.clone(),
            )
            .with_member(),
        )
        .unwrap();
    construction
        .add_node(Node::new("principal-start", [0.0, -0.9, 0.15]))
        .unwrap();
    construction
        .add_node(Node::new("principal-end", [0.0, 0.9, 0.15]))
        .unwrap();
    construction
        .add_node(Node::new("common-start", [0.0, -0.9, 0.55]))
        .unwrap();
    construction
        .add_node(Node::new("common-end", [0.0, 0.9, 0.55]))
        .unwrap();
    construction
        .add_member(Member::new(
            "principal",
            "principal",
            "principal-start",
            "principal-end",
            evidence.clone(),
        ))
        .unwrap();
    construction
        .add_member(Member::new(
            "common",
            "common",
            "common-start",
            "common-end",
            evidence.clone(),
        ))
        .unwrap();

    for (ordinal, y) in [(-0.35_f64), 0.35].into_iter().enumerate() {
        let purlin = alloc::format!("purlin-{ordinal}");
        let west = alloc::format!("{purlin}-west");
        let east = alloc::format!("{purlin}-east");
        let trench = alloc::format!("trench-{ordinal}");
        let seat = alloc::format!("seat-{ordinal}");
        construction
            .add_element(
                Element::new(
                    &purlin,
                    PURLIN_ROLE,
                    "oak",
                    OrientedBox::axis_aligned([-1.0, y - 0.10, 0.27], [2.0, 0.20, 0.22]),
                    evidence.clone(),
                )
                .with_member(),
            )
            .unwrap();
        construction
            .add_node(Node::new(&west, [-0.9, y, 0.38]))
            .unwrap();
        construction
            .add_node(Node::new(&east, [0.9, y, 0.38]))
            .unwrap();
        construction
            .add_node(Node::new(&trench, [0.0, y, 0.27]))
            .unwrap();
        construction
            .add_node(Node::new(&seat, [0.0, y, 0.49]))
            .unwrap();
        construction
            .add_member(Member::new(
                &purlin,
                &purlin,
                &west,
                &east,
                evidence.clone(),
            ))
            .unwrap();
        construction
            .add_relation(Relation::new(
                &trench,
                RelationKind::member_member(&trench, &["principal", &purlin]),
                "purlin-trench",
                evidence.clone(),
            ))
            .unwrap();
        construction
            .add_relation(Relation::new(
                &seat,
                RelationKind::member_member(&seat, &[&purlin, "common"]),
                "common-rafter-seat",
                evidence.clone(),
            ))
            .unwrap();
    }
    construction
}

fn apply_rule<R: Rule>(
    construction: &mut Construction,
    application: &str,
    relation: &str,
    rule: &R,
    params: &R::Params,
) {
    let output = {
        let context = RuleContext::new(construction, relation).unwrap();
        rule.instantiate(&context, params).unwrap()
    };
    construction
        .apply(RuleApplication::new(
            application,
            rule.key(),
            relation,
            Evidence::new("fixture", EvidenceClass::RegionalAnalogy),
            output,
        ))
        .unwrap();
}

fn fitted_fixture(ordinals: [usize; 2]) -> Construction {
    let mut construction = fixture();
    for ordinal in ordinals {
        apply_rule(
            &mut construction,
            &alloc::format!("fit-trench-{ordinal}"),
            &alloc::format!("trench-{ordinal}"),
            &PurlinToPrincipalTrenchRule,
            &PurlinPrincipalTrenchParams::default(),
        );
        apply_rule(
            &mut construction,
            &alloc::format!("fit-seat-{ordinal}"),
            &alloc::format!("seat-{ordinal}"),
            &CommonRafterToPurlinSeatRule,
            &CommonRafterPurlinSeatParams::default(),
        );
    }
    construction
}

fn sound_quantized_vertices(construction: &Construction, key: &str) -> alloc::vec::Vec<[u64; 3]> {
    let recipe = compose(construction, construction.element(key).unwrap()).unwrap();
    let evaluated = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    assert_eq!(
        evaluated.bodies.len(),
        1,
        "{key}: {:?}",
        evaluated.report.diagnostics
    );
    assert!(evaluated.report.clean_at(Severity::Warning), "{key}");
    let mesh = &evaluated.bodies[0].body.mesh;
    assert!(mesh.validate_deep().is_empty());

    // Boolean order can legitimately change topology identifiers and
    // traversal order. Comparing the sorted geometric vertex set at the
    // kernel tolerance checks the resulting solid without depending on
    // either implementation detail.
    let mut vertices = mesh
        .vertices()
        .filter_map(|vertex| mesh.vertex_position(vertex))
        .map(|point| {
            point.map(|value| {
                let quantized = (f64::from(value) * 1.0e8).round() / 1.0e8;
                if quantized == 0.0 {
                    0.0_f64.to_bits()
                } else {
                    quantized.to_bits()
                }
            })
        })
        .collect::<alloc::vec::Vec<_>>();
    vertices.sort_unstable();
    vertices
}

#[test]
fn distinct_rules_edit_only_the_timber_that_receives_each_crossing() {
    // Relation participant order is deliberately reversed between the
    // fixtures. Role selection must still trench the principal and seat the
    // common rafter while leaving each full-section purlin untouched.
    let construction = fixture();
    let trench = PurlinToPrincipalTrenchRule
        .instantiate(
            &RuleContext::new(&construction, "trench-0").unwrap(),
            &PurlinPrincipalTrenchParams::default(),
        )
        .unwrap();
    let seat = CommonRafterToPurlinSeatRule
        .instantiate(
            &RuleContext::new(&construction, "seat-0").unwrap(),
            &CommonRafterPurlinSeatParams::default(),
        )
        .unwrap();
    assert_eq!(trench.part_edits.len(), 1);
    assert_eq!(trench.part_edits[0].target, "principal");
    assert_eq!(seat.part_edits.len(), 1);
    assert_eq!(seat.part_edits[0].target, "common");
}

#[test]
fn assess_accepts_valid_authored_overlaps_that_use_custom_depths() {
    // Applicability describes the crossing geometry, not the default
    // parameter choice. Both custom overlaps must assess as suitable and
    // then instantiate when their explicit depths match the extents.
    let mut trench_construction = fixture();
    trench_construction
        .set_element_extent(
            "principal",
            OrientedBox {
                origin: [-0.10, -1.0, 0.0],
                axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                size: [2.0, 0.20, 0.31],
            },
        )
        .unwrap();
    let trench_context = RuleContext::new(&trench_construction, "trench-0").unwrap();
    assert!(
        PurlinToPrincipalTrenchRule
            .assess(&trench_context)
            .is_suitable()
    );
    PurlinToPrincipalTrenchRule
        .instantiate(
            &trench_context,
            &PurlinPrincipalTrenchParams {
                trench_depth: default_millimeters(40),
                ..PurlinPrincipalTrenchParams::default()
            },
        )
        .unwrap();

    let mut seat_construction = fixture();
    seat_construction
        .set_element_extent(
            "common",
            OrientedBox {
                origin: [-0.06, -1.0, 0.46],
                axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                size: [2.0, 0.12, 0.16],
            },
        )
        .unwrap();
    let seat_context = RuleContext::new(&seat_construction, "seat-0").unwrap();
    assert!(
        CommonRafterToPurlinSeatRule
            .assess(&seat_context)
            .is_suitable()
    );
    CommonRafterToPurlinSeatRule
        .instantiate(
            &seat_context,
            &CommonRafterPurlinSeatParams {
                seat_depth: default_millimeters(30),
                ..CommonRafterPurlinSeatParams::default()
            },
        )
        .unwrap();
}

#[test]
fn common_rafter_seat_opens_from_the_lower_face_and_ends_at_bearing() {
    // Pin the emitted tool in receiver-local coordinates. Its lower cap must
    // overrun the common rafter underside, while its upper cap is exactly the
    // purlin-top bearing plane—not above it inside the member.
    let construction = fixture();
    let params = CommonRafterPurlinSeatParams::default();
    let output = CommonRafterToPurlinSeatRule
        .instantiate(&RuleContext::new(&construction, "seat-0").unwrap(), &params)
        .unwrap();
    let tool = output.part_edits[0].op.tool();
    let exedra_constructive::ir::NodeKind::Extrude { height, .. } =
        &tool.recipe.node(tool.recipe.root()).unwrap().kind
    else {
        panic!("common-rafter seat tool is one extrusion");
    };
    let start = tool.placement.rows[2][3];
    let end = start + tool.placement.rows[2][2] * height;
    assert!(start < 0.0, "seat cutter starts outside the lower face");
    assert!((end - params.seat_depth.as_meters()).abs() < 1.0e-12);
}

#[test]
fn two_trenches_and_two_seats_evaluate_as_single_sound_timbers() {
    // This is the combined Boolean oracle: repeated disjoint cutters on both
    // receiving members must keep every recipe manifold and must not make one
    // valid crossing depend on application order.
    let forward = fitted_fixture([0, 1]);
    let reverse = fitted_fixture([1, 0]);
    for key in ["principal", "common", "purlin-0", "purlin-1"] {
        assert_eq!(
            sound_quantized_vertices(&forward, key),
            sound_quantized_vertices(&reverse, key),
            "{key}: reversing the disjoint crossing applications changed the solid"
        );
    }
}

#[test]
fn cut_depth_must_match_the_overlap_authored_by_setout() {
    // A parameter change must not silently move a member or leave a gap: the
    // same relation is invalid when its requested depth differs from the
    // explicit overlap in the fixture extents.
    let construction = fixture();
    let context = RuleContext::new(&construction, "trench-0").unwrap();
    let params = PurlinPrincipalTrenchParams {
        trench_depth: default_millimeters(40),
        ..PurlinPrincipalTrenchParams::default()
    };
    assert!(matches!(
        PurlinToPrincipalTrenchRule.instantiate(&context, &params),
        Err(RuleError::InvalidParameter {
            what: "authored purlin overlap does not match trench depth"
        })
    ));
}

#[test]
fn through_trench_refuses_a_half_width_endpoint_bearing() {
    // A purlin endpoint centred on the principal can still exceed the numeric
    // minimum bearing while covering only half the principal. Require the
    // complete crossing width and leave half laps to a separately named rule.
    let mut construction = fixture();
    construction
        .set_element_extent(
            "purlin-0",
            OrientedBox::axis_aligned([-0.05, -0.45, 0.27], [0.10, 0.20, 0.22]),
        )
        .unwrap();
    let context = RuleContext::new(&construction, "trench-0").unwrap();
    assert!(matches!(
        PurlinToPrincipalTrenchRule.instantiate(&context, &PurlinPrincipalTrenchParams::default()),
        Err(RuleError::NotApplicable(_))
    ));
}

#[test]
fn rules_refuse_fragile_remaining_depth_and_open_ended_seats() {
    // Geometric fit is not structural sizing, but it must still reject a
    // trench that nearly severs its principal, a trench that breaks through
    // its end, and a common-rafter notch without the requested end relish.
    let construction = fixture();
    let trench_context = RuleContext::new(&construction, "trench-0").unwrap();
    let trench_params = PurlinPrincipalTrenchParams {
        minimum_remaining_depth: default_millimeters(280),
        ..PurlinPrincipalTrenchParams::default()
    };
    assert!(matches!(
        PurlinToPrincipalTrenchRule.instantiate(&trench_context, &trench_params),
        Err(RuleError::Degenerate { .. })
    ));

    let mut end_trench = fixture();
    end_trench
        .set_element_extent(
            "principal",
            OrientedBox {
                origin: [-0.10, -0.48, 0.0],
                axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                size: [2.0, 0.20, 0.30],
            },
        )
        .unwrap();
    let end_context = RuleContext::new(&end_trench, "trench-0").unwrap();
    assert!(matches!(
        PurlinToPrincipalTrenchRule
            .instantiate(&end_context, &PurlinPrincipalTrenchParams::default()),
        Err(RuleError::Degenerate {
            what: "principal-rafter trench needs timber beyond both ends"
        })
    ));

    let seat_context = RuleContext::new(&construction, "seat-0").unwrap();
    let seat_params = CommonRafterPurlinSeatParams {
        minimum_end_relish: default_millimeters(1_000),
        ..CommonRafterPurlinSeatParams::default()
    };
    assert!(matches!(
        CommonRafterToPurlinSeatRule.instantiate(&seat_context, &seat_params),
        Err(RuleError::Degenerate { .. })
    ));
}
