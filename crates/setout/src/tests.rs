// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::string::ToString;
use alloc::vec::Vec;

use super::*;

#[test]
fn sum_direction_follows_roots_and_ignores_root_insertion_order() {
    let mut builder = NetworkBuilder::new();
    let left = builder
        .declare::<Length>("press/left", QuantityPolicy::positive())
        .unwrap();
    let right = builder
        .declare::<Length>("press/right", QuantityPolicy::positive())
        .unwrap();
    let total = builder
        .declare::<Length>("press/total", QuantityPolicy::positive())
        .unwrap();
    builder
        .relate(Sum::new("press/add", left.clone(), right.clone(), total.clone()).unwrap())
        .unwrap();
    let definition = builder.finish().unwrap();

    // Author total and right, so the same undirected relation must choose its
    // reverse method and derive left. Rebuilding the root map in the opposite
    // insertion order guards against accidentally using authoring order as
    // semantic identity or plan priority.
    let mut roots_a = RootClaimSetBuilder::new(&definition);
    roots_a
        .author(
            "root/total",
            &total,
            Knowledge::exact(Length::millimetres(224).unwrap()),
        )
        .unwrap()
        .author(
            "root/right",
            &right,
            Knowledge::exact(Length::millimetres(7).unwrap()),
        )
        .unwrap();
    let roots_a = roots_a.finish().unwrap();
    let mut roots_b = RootClaimSetBuilder::new(&definition);
    roots_b
        .author(
            "root/right",
            &right,
            Knowledge::exact(Length::millimetres(7).unwrap()),
        )
        .unwrap()
        .author(
            "root/total",
            &total,
            Knowledge::exact(Length::millimetres(224).unwrap()),
        )
        .unwrap();
    let roots_b = roots_b.finish().unwrap();

    let scenario_a = EvaluationScenarioBuilder::new("press/scenario")
        .unwrap()
        .activate_all(&roots_a)
        .finish(&roots_a)
        .unwrap();
    let scenario_b = EvaluationScenarioBuilder::new("press/scenario")
        .unwrap()
        .activate_all(&roots_b)
        .finish(&roots_b)
        .unwrap();
    let plan_a = compile_plan(&definition, &roots_a, &scenario_a).unwrap();
    let plan_b = compile_plan(&definition, &roots_b, &scenario_b).unwrap();
    assert_eq!(roots_a.fingerprint(), roots_b.fingerprint());
    assert_eq!(plan_a.fingerprint(), plan_b.fingerprint());
    assert_eq!(plan_a.steps()[0].method.as_str(), "total-right-to-left");

    let result = evaluate(&definition, &roots_a, &scenario_a, &plan_a).unwrap();
    assert_eq!(
        result.exact(&left).unwrap(),
        Length::millimetres(217).unwrap()
    );
}

#[test]
fn point_root_decomposes_every_component_and_declared_unknowns_remain_visible() {
    let mut builder = NetworkBuilder::new();
    let x = builder
        .declare::<Length>("survey/x", QuantityPolicy::unrestricted())
        .unwrap();
    let y = builder
        .declare::<Length>("survey/y", QuantityPolicy::unrestricted())
        .unwrap();
    let z = builder
        .declare::<Length>("survey/z", QuantityPolicy::unrestricted())
        .unwrap();
    let point = builder
        .declare::<Point3>("survey/point", QuantityPolicy::unrestricted())
        .unwrap();
    let unobserved = builder
        .declare::<Length>("survey/unobserved", QuantityPolicy::unrestricted())
        .unwrap();
    builder
        .relate(
            ComposePoint::new(
                "survey/components",
                x.clone(),
                y.clone(),
                z.clone(),
                point.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    let definition = builder.finish().unwrap();
    let observed = Point3::new(
        Length::millimetres(100).unwrap(),
        Length::millimetres(200).unwrap(),
        Length::millimetres(300).unwrap(),
    );
    let mut roots = RootClaimSetBuilder::new(&definition);
    roots
        .author("root/point", &point, Knowledge::exact(observed))
        .unwrap();
    let roots = roots.finish().unwrap();
    let scenario = EvaluationScenarioBuilder::new("survey/scenario")
        .unwrap()
        .activate_all(&roots)
        .finish(&roots)
        .unwrap();

    // ComposePoint is unusual among the built-in relations: one known point
    // exposes three independent outputs. This specifically prevents the plan
    // from consuming the relation after deriving only its first component.
    let plan = compile_plan(&definition, &roots, &scenario).unwrap();
    let methods: Vec<_> = plan
        .steps()
        .iter()
        .map(|step| step.method.as_str())
        .collect();
    assert_eq!(methods, ["point-to-x", "point-to-y", "point-to-z"]);
    let result = evaluate(&definition, &roots, &scenario, &plan).unwrap();
    assert_eq!(result.exact(&x).unwrap(), observed.x);
    assert_eq!(result.exact(&y).unwrap(), observed.y);
    assert_eq!(result.exact(&z).unwrap(), observed.z);

    // A declared quantity has a real public state even when no claim has ever
    // mentioned it; callers need to distinguish Unknown from a foreign key.
    assert_eq!(result.state(unobserved.key()), Some(QuantityState::Unknown));
}

#[test]
fn inactive_root_selection_is_orphaned_without_panicking() {
    let mut builder = NetworkBuilder::new();
    let length = builder
        .declare::<Length>("survey/length", QuantityPolicy::positive())
        .unwrap();
    let definition = builder.finish().unwrap();
    let mut roots = RootClaimSetBuilder::new(&definition);
    roots
        .author(
            "root/active",
            &length,
            Knowledge::exact(Length::millimetres(100).unwrap()),
        )
        .unwrap()
        .author(
            "root/inactive",
            &length,
            Knowledge::exact(Length::millimetres(101).unwrap()),
        )
        .unwrap();
    let roots = roots.finish().unwrap();
    let inactive = RootClaimKey::new("root/inactive").unwrap();
    let decision = Decision::new(
        "decision/use-inactive",
        DecisionAction::SelectClaim {
            quantity: length.key().clone(),
            selection: ClaimSelection {
                producer: ClaimProducer::ExternalRoot(inactive.clone()),
                expected: roots.claim_key(&inactive).unwrap(),
            },
        },
    )
    .unwrap();
    let scenario = EvaluationScenarioBuilder::new("survey/inactive-selection")
        .unwrap()
        .activate("root/active")
        .unwrap()
        .decide(decision)
        .unwrap()
        .finish(&roots)
        .unwrap();

    // Durable decisions can outlive scenario activation edits. Evaluation must
    // retain that intent as an orphaned state, not assume the root was inserted
    // and panic while opening an otherwise valid scenario.
    let plan = compile_plan(&definition, &roots, &scenario).unwrap();
    let result = evaluate(&definition, &roots, &scenario, &plan).unwrap();
    assert!(matches!(
        result.state(length.key()),
        Some(QuantityState::OrphanedSelection { .. })
    ));
    assert!(matches!(
        result.diagnostics(),
        [Diagnostic::OrphanedDecision { actual: None, .. }]
    ));
}

#[test]
fn relation_selection_cannot_be_forced_onto_another_quantity() {
    let mut builder = NetworkBuilder::new();
    let left = builder
        .declare::<Length>("survey/left", QuantityPolicy::positive())
        .unwrap();
    let right = builder
        .declare::<Length>("survey/right", QuantityPolicy::positive())
        .unwrap();
    let total = builder
        .declare::<Length>("survey/total", QuantityPolicy::positive())
        .unwrap();
    let unrelated = builder
        .declare::<Length>("survey/unrelated", QuantityPolicy::positive())
        .unwrap();
    builder
        .relate(Sum::new("survey/sum", left.clone(), right.clone(), total.clone()).unwrap())
        .unwrap();
    let definition = builder.finish().unwrap();
    let mut roots = RootClaimSetBuilder::new(&definition);
    roots
        .author(
            "root/left",
            &left,
            Knowledge::exact(Length::millimetres(40).unwrap()),
        )
        .unwrap()
        .author(
            "root/right",
            &right,
            Knowledge::exact(Length::millimetres(60).unwrap()),
        )
        .unwrap();
    let roots = roots.finish().unwrap();
    let base_scenario = EvaluationScenarioBuilder::new("survey/base")
        .unwrap()
        .activate_all(&roots)
        .finish(&roots)
        .unwrap();
    let base_plan = compile_plan(&definition, &roots, &base_scenario).unwrap();
    let base = evaluate(&definition, &roots, &base_scenario, &base_plan).unwrap();
    let total_claim = base
        .provenance()
        .operative(total.key())
        .expect("sum produces total")
        .key();
    let decision = Decision::new(
        "decision/wrong-target",
        DecisionAction::SelectClaim {
            quantity: unrelated.key().clone(),
            selection: ClaimSelection {
                producer: ClaimProducer::Relation {
                    relation: RelationKey::new("survey/sum").unwrap(),
                    method: MethodId::new("left-plus-right-to-total").unwrap(),
                },
                expected: total_claim,
            },
        },
    )
    .unwrap();
    let scenario = EvaluationScenarioBuilder::new("survey/wrong-target")
        .unwrap()
        .activate_all(&roots)
        .decide(decision)
        .unwrap()
        .finish(&roots)
        .unwrap();

    // A persisted producer and expected claim are insufficient on their own:
    // the decision's quantity is part of the contract. This guards against
    // applying a valid total-producing method to an unrelated requested slot.
    let plan = compile_plan(&definition, &roots, &scenario).unwrap();
    let result = evaluate(&definition, &roots, &scenario, &plan).unwrap();
    assert!(matches!(
        result.state(total.key()),
        Some(QuantityState::Unique { .. })
    ));
    assert!(matches!(
        result.state(unrelated.key()),
        Some(QuantityState::OrphanedSelection { .. })
    ));
}

#[test]
fn roof_spine_derives_pitch_points_and_integer_root_with_complete_explain() {
    let mut builder = NetworkBuilder::new();
    let zero = builder
        .declare::<Length>("roof/zero", QuantityPolicy::non_negative())
        .unwrap();
    let span = builder
        .declare::<Length>("roof/span", QuantityPolicy::positive())
        .unwrap();
    let half_span = builder
        .declare::<Length>("roof/half-span", QuantityPolicy::positive())
        .unwrap();
    let wall_head = builder
        .declare::<Length>("roof/wall-head", QuantityPolicy::positive())
        .unwrap();
    let plate_height = builder
        .declare::<Length>("roof/wall-plate-height", QuantityPolicy::positive())
        .unwrap();
    let plate_top = builder
        .declare::<Length>("roof/wall-plate-top", QuantityPolicy::positive())
        .unwrap();
    let rise = builder
        .declare::<Length>("roof/rise", QuantityPolicy::positive())
        .unwrap();
    let ridge = builder
        .declare::<Length>("roof/ridge", QuantityPolicy::positive())
        .unwrap();
    let pitch = builder
        .declare::<Rational>("roof/pitch", QuantityPolicy::unrestricted())
        .unwrap();
    let slope = builder
        .declare::<Length>("roof/slope", QuantityPolicy::positive())
        .unwrap();
    let south_y = builder
        .declare::<Length>("roof/south-y", QuantityPolicy::unrestricted())
        .unwrap();
    let north_foot = builder
        .declare::<Point3>("roof/north-foot", QuantityPolicy::unrestricted())
        .unwrap();
    let south_foot = builder
        .declare::<Point3>("roof/south-foot", QuantityPolicy::unrestricted())
        .unwrap();
    let ridge_point = builder
        .declare::<Point3>("roof/ridge-point", QuantityPolicy::unrestricted())
        .unwrap();

    builder
        .relate(
            ScaleLength::new(
                "roof/a-half-span",
                span.clone(),
                half_span.clone(),
                Rational::new(1, 2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            Sum::new(
                "roof/b-plate-top",
                wall_head.clone(),
                plate_height.clone(),
                plate_top.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            Sum::new(
                "roof/c-ridge",
                plate_top.clone(),
                rise.clone(),
                ridge.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            Pitch::new(
                "roof/d-pitch",
                half_span.clone(),
                rise.clone(),
                pitch.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            Pythagorean::new(
                "roof/e-slope",
                half_span.clone(),
                rise.clone(),
                slope.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            ScaleLength::new(
                "roof/f-south-y",
                half_span.clone(),
                south_y.clone(),
                Rational::new(-1, 1).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            ComposePoint::new(
                "roof/g-north-foot",
                zero.clone(),
                half_span.clone(),
                plate_top.clone(),
                north_foot.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            ComposePoint::new(
                "roof/h-south-foot",
                zero.clone(),
                south_y.clone(),
                plate_top.clone(),
                south_foot.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    builder
        .relate(
            ComposePoint::new(
                "roof/i-ridge-point",
                zero.clone(),
                zero.clone(),
                ridge.clone(),
                ridge_point.clone(),
            )
            .unwrap(),
        )
        .unwrap();
    let definition = builder.finish().unwrap();

    let mut roots = RootClaimSetBuilder::new(&definition);
    roots
        .author("root/zero", &zero, Knowledge::exact(Length::ZERO))
        .unwrap()
        .author(
            "root/span",
            &span,
            Knowledge::exact(Length::millimetres(9_000).unwrap()),
        )
        .unwrap()
        .author(
            "root/wall-head",
            &wall_head,
            Knowledge::exact(Length::millimetres(11_000).unwrap()),
        )
        .unwrap()
        .author(
            "root/plate-height",
            &plate_height,
            Knowledge::exact(Length::millimetres(180).unwrap()),
        )
        .unwrap()
        .author(
            "root/rise",
            &rise,
            Knowledge::exact(Length::millimetres(3_200).unwrap()),
        )
        .unwrap();
    let roots = roots.finish().unwrap();
    let scenario = EvaluationScenarioBuilder::new("roof/surviving-gable")
        .unwrap()
        .activate_all(&roots)
        .finish(&roots)
        .unwrap();
    let plan = compile_plan(&definition, &roots, &scenario).unwrap();
    let result = evaluate(&definition, &roots, &scenario, &plan).unwrap();

    assert_eq!(
        result.exact(&plate_top).unwrap(),
        Length::millimetres(11_180).unwrap()
    );
    assert_eq!(
        result.exact(&ridge).unwrap(),
        Length::millimetres(14_380).unwrap()
    );
    assert_eq!(
        result.exact(&pitch).unwrap(),
        Rational::new(32, 45).unwrap()
    );
    assert_eq!(
        result.exact(&north_foot).unwrap(),
        Point3::new(
            Length::ZERO,
            Length::millimetres(4_500).unwrap(),
            Length::millimetres(11_180).unwrap(),
        )
    );

    // The slope is irrational in iota. Its claim must carry the integer-root
    // certificate rather than pretending the rounded iota value is exact.
    let slope_claim = result
        .provenance()
        .operative(slope.key())
        .expect("slope is derived");
    assert!(matches!(
        slope_claim.exactness(),
        ExactnessTrace::RootQuantization(rounding)
            if rounding.remainder > 0 && rounding.policy == Round::Nearest
    ));
    let explanation = result
        .provenance()
        .explain(ridge_point.key())
        .expect("ridge point is explainable");
    assert!(explanation.contains("root root/wall-head"));
    assert!(explanation.contains("root root/rise"));
}

#[test]
fn challenger_stays_local_until_a_structural_decision_selects_it() {
    let mut builder = NetworkBuilder::new();
    let left = builder
        .declare::<Length>("gauge/left", QuantityPolicy::positive())
        .unwrap();
    let right = builder
        .declare::<Length>("gauge/right", QuantityPolicy::positive())
        .unwrap();
    let total = builder
        .declare::<Length>("gauge/total", QuantityPolicy::positive())
        .unwrap();
    let downstream = builder
        .declare::<Length>("gauge/downstream", QuantityPolicy::positive())
        .unwrap();
    builder
        .relate(Sum::new("gauge/a-sum", left.clone(), right.clone(), total.clone()).unwrap())
        .unwrap();
    builder
        .relate(
            OffsetLength::new(
                "gauge/b-offset",
                total.clone(),
                downstream.clone(),
                Length::millimetres(1).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let definition = builder.finish().unwrap();
    let mut roots = RootClaimSetBuilder::new(&definition);
    roots
        .author(
            "root/left",
            &left,
            Knowledge::exact(Length::millimetres(40).unwrap()),
        )
        .unwrap()
        .author(
            "root/right",
            &right,
            Knowledge::exact(Length::millimetres(60).unwrap()),
        )
        .unwrap()
        .author(
            "root/total",
            &total,
            Knowledge::exact(Length::millimetres(101).unwrap()),
        )
        .unwrap();
    let roots = roots.finish().unwrap();
    let base_scenario = EvaluationScenarioBuilder::new("gauge/base")
        .unwrap()
        .activate_all(&roots)
        .finish(&roots)
        .unwrap();
    let base_plan = compile_plan(&definition, &roots, &base_scenario).unwrap();
    let base = evaluate(&definition, &roots, &base_scenario, &base_plan).unwrap();
    assert!(matches!(
        base.state(total.key()),
        Some(QuantityState::Contested { .. })
    ));
    assert_eq!(
        base.exact(&downstream).unwrap(),
        Length::millimetres(102).unwrap(),
        "the independent total remains provisional; its challenger does not leak downstream"
    );

    let derived = base
        .provenance()
        .claims()
        .find(|claim| {
            matches!(
                claim.origin(),
                ClaimOrigin::Relation { relation, method, .. }
                    if relation.as_str() == "gauge/a-sum"
                        && method.as_str() == "left-plus-right-to-total"
            )
        })
        .expect("sum challenger exists");
    let decision = Decision::new(
        "decision/use-components",
        DecisionAction::SelectClaim {
            quantity: total.key().clone(),
            selection: ClaimSelection {
                producer: ClaimProducer::Relation {
                    relation: RelationKey::new("gauge/a-sum").unwrap(),
                    method: MethodId::new("left-plus-right-to-total").unwrap(),
                },
                expected: derived.key(),
            },
        },
    )
    .unwrap();
    let selected_scenario = EvaluationScenarioBuilder::new("gauge/selected")
        .unwrap()
        .activate_all(&roots)
        .decide(decision)
        .unwrap()
        .finish(&roots)
        .unwrap();
    let selected_plan = compile_plan(&definition, &roots, &selected_scenario).unwrap();
    let selected = evaluate(&definition, &roots, &selected_scenario, &selected_plan).unwrap();
    assert_eq!(
        selected.exact(&downstream).unwrap(),
        Length::millimetres(101).unwrap(),
        "the explicit counterfactual now propagates through the downstream relation"
    );
}

#[test]
fn incremental_successor_reuses_the_unaffected_branch_and_matches_fresh() {
    let mut builder = NetworkBuilder::new();
    let a = builder
        .declare::<Length>("net/a", QuantityPolicy::positive())
        .unwrap();
    let b = builder
        .declare::<Length>("net/b", QuantityPolicy::positive())
        .unwrap();
    let ab = builder
        .declare::<Length>("net/ab", QuantityPolicy::positive())
        .unwrap();
    let c = builder
        .declare::<Length>("net/c", QuantityPolicy::positive())
        .unwrap();
    let d = builder
        .declare::<Length>("net/d", QuantityPolicy::positive())
        .unwrap();
    let cd = builder
        .declare::<Length>("net/cd", QuantityPolicy::positive())
        .unwrap();
    builder
        .relate(Sum::new("net/add-ab", a.clone(), b.clone(), ab.clone()).unwrap())
        .unwrap();
    builder
        .relate(Sum::new("net/add-cd", c.clone(), d.clone(), cd.clone()).unwrap())
        .unwrap();
    let definition = builder.finish().unwrap();

    let build_roots = |b_value: i64| {
        let mut roots = RootClaimSetBuilder::new(&definition);
        for (key, quantity, value) in [
            ("root/a", &a, 10),
            ("root/b", &b, b_value),
            ("root/c", &c, 30),
            ("root/d", &d, 40),
        ] {
            roots
                .author(
                    key,
                    quantity,
                    Knowledge::exact(Length::millimetres(value).unwrap()),
                )
                .unwrap();
        }
        roots.finish().unwrap()
    };
    let roots_before = build_roots(20);
    let scenario_before = EvaluationScenarioBuilder::new("net/scenario")
        .unwrap()
        .activate_all(&roots_before)
        .finish(&roots_before)
        .unwrap();
    let plan_before = compile_plan(&definition, &roots_before, &scenario_before).unwrap();
    let before = evaluate(&definition, &roots_before, &scenario_before, &plan_before).unwrap();
    let unaffected_id = before
        .provenance()
        .operative(cd.key())
        .expect("independent sum exists")
        .id();

    let roots_after = build_roots(21);
    let scenario_after = EvaluationScenarioBuilder::new("net/scenario")
        .unwrap()
        .activate_all(&roots_after)
        .finish(&roots_after)
        .unwrap();
    let plan_after = compile_plan(&definition, &roots_after, &scenario_after).unwrap();
    assert_eq!(plan_before.fingerprint(), plan_after.fingerprint());
    let warm = IncrementalEvaluator::new()
        .successor(
            &definition,
            &roots_after,
            &scenario_after,
            &plan_after,
            &before,
        )
        .unwrap();
    let fresh = evaluate(&definition, &roots_after, &scenario_after, &plan_after).unwrap();

    assert_eq!(warm.fingerprint(), fresh.fingerprint());
    assert_eq!(warm.work_report().steps_reused, 1);
    assert_eq!(warm.work_report().steps_evaluated, 1);
    assert_eq!(
        warm.provenance()
            .operative(cd.key())
            .expect("unaffected sum remains")
            .id(),
        unaffected_id,
        "unchanged public claim retains its safe arena handle in the lineage"
    );
    assert_eq!(
        &*warm.delta_from(&before).quantities_changed,
        &[b.key().clone(), ab.key().clone()]
    );
}

#[test]
fn dimension_parse_and_format_round_trip_iota_exactly() {
    // Joto parsing/formatting is part of the calm boundary, but setout stores
    // only the resulting iota. This guards the useful SI/US-customary bridge
    // without allowing unit text into propagation fingerprints.
    let length: Length = "9.18m".parse().unwrap();
    assert_eq!(length, Length::millimetres(9_180).unwrap());
    assert_eq!(length.to_string(), "9.18m");
}
