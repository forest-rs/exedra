// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::builders;
use exedra_constructive::evaluate::{Severity, evaluate};
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
use exedra_constructive::tessellate::EvalPolicy;
use joiner::{
    Construction, Element, Evidence, EvidenceClass, EvidenceSource, Member, Node, OrientedBox,
    Part, Relation, RelationKind, compose, measure_contact,
};

use super::*;

fn fixture(depth: f64, station: f64, size: [f64; 3]) -> Construction {
    fixture_in_frame(
        depth,
        station,
        size,
        &OrientedBox::axis_aligned([0.0; 3], [1.0; 3]),
    )
}

fn fixture_in_frame(depth: f64, station: f64, size: [f64; 3], frame: &OrientedBox) -> Construction {
    let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
    let mut c = Construction::new();
    c.add_evidence_source(EvidenceSource::new(
        "fixture",
        evidence.class,
        "https://example.invalid/round-seat",
        "Authored circular purlin and rectangular support",
    ))
    .unwrap();
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let circle = builder.add_profile(builders::circle(size[1] * 0.5).unwrap());
    let root = builder
        .with_material(slot)
        .add(NodeKind::Extrude {
            profile: circle,
            placement: Placement3::from_axes(
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.0, size[1] * 0.5, size[2] * 0.5],
            ),
            height: size[0],
            caps: CapMode::Both,
        })
        .unwrap();
    let purlin = Element::new(
        "purlin",
        "round-purlin",
        "oak",
        OrientedBox::axis_aligned([0.0, -size[1] * 0.5, -depth], size),
        evidence.clone(),
    )
    .with_part(Part::new(builder.finish(root).unwrap()))
    .with_member();
    let support = Element::new(
        "support",
        "purlin-support",
        "oak",
        OrientedBox {
            origin: [station + 0.1, -0.3, -0.2],
            axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            size: [0.6, 0.2, 0.2],
        },
        evidence.clone(),
    )
    .with_member();
    for mut e in [purlin, support] {
        e.extent.origin = frame.anchor(e.extent.origin);
        e.extent.axes = e
            .extent
            .axes
            .map(|axis| [0, 1, 2].map(|i| (0..3).map(|j| frame.axes[j][i] * axis[j]).sum()));
        let key = e.key.clone();
        let start = alloc::format!("{key}-start");
        let end = alloc::format!("{key}-end");
        c.add_node(Node::new(
            &start,
            e.extent
                .anchor([0.0, e.extent.size[1] * 0.5, e.extent.size[2] * 0.5]),
        ))
        .unwrap();
        c.add_node(Node::new(
            &end,
            e.extent.anchor([
                e.extent.size[0],
                e.extent.size[1] * 0.5,
                e.extent.size[2] * 0.5,
            ]),
        ))
        .unwrap();
        c.add_element(e).unwrap();
        c.add_member(Member::new(&key, &key, &start, &end, evidence.clone()))
            .unwrap();
    }
    c.add_node(Node::new("seat", frame.anchor([station, 0.0, 0.0])))
        .unwrap();
    c.add_relation(Relation::new(
        "seat",
        RelationKind::member_member("seat", &["support", "purlin"]),
        "round-seat",
        evidence,
    ))
    .unwrap();
    c
}

#[test]
fn circular_seat_has_a_chord_sized_bearing_and_only_cuts_the_purlin() {
    let mut c = fixture(0.015, 1.0, [2.0, 0.19, 0.19]);
    c.apply_rule(
        "fit",
        "seat",
        &RoundPurlinSeatRule,
        &RoundPurlinSeatParams::default(),
    )
    .unwrap();
    let contact = &c.contacts()[0];
    let footprint = contact.footprint_meters().unwrap();
    assert!((footprint[0] - 2.0 * Real::sqrt(0.015 * 0.175)).abs() < 1e-12);
    assert!((footprint[1] - 0.2).abs() < 1e-12);
    assert!(
        footprint[0] < 0.19 * 0.6,
        "a diameter-wide bearing would be false"
    );
    let measurement = measure_contact(&c, contact).unwrap();
    assert!(measurement.gap.abs() < 1e-12);
    assert_eq!(c.part_edits_for("support").count(), 0);
    assert_eq!(c.part_edits_for("purlin").count(), 1);
    let recipe = compose(&c, c.element("purlin").unwrap()).unwrap();
    let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    assert!(
        result.report.clean_at(Severity::Warning),
        "{:?}",
        result.report
    );
    assert_eq!(result.bodies.len(), 1);
    let mesh = &result.bodies[0].body.mesh;
    assert!(mesh.validate_deep().is_empty());
    let mut seat_vertices = 0;
    for vertex in mesh.vertices() {
        let p = mesh.vertex_position(vertex).unwrap();
        if (f64::from(p[0]) - 1.0).abs() <= 0.100_49 {
            assert!(
                f64::from(p[2]) >= 0.015 - 1e-6,
                "underside survives inside the notch: {p:?}"
            );
        }
        if (f64::from(p[2]) - 0.015).abs() < 1e-6 {
            seat_vertices += 1;
        }
    }
    assert!(seat_vertices >= 4, "the cut must create a finite flat face");
}

#[test]
fn tangent_deep_misplaced_and_incomplete_seats_are_refused() {
    let params = RoundPurlinSeatParams::default();
    for (depth, station, size) in [
        (0.0, 1.0, [2.0, 0.19, 0.19]),
        (0.095, 1.0, [2.0, 0.19, 0.19]),
        (0.014, 1.0, [2.0, 0.19, 0.19]),
        (0.015, 0.12, [2.0, 0.19, 0.19]),
        (0.015, 0.05, [2.0, 0.19, 0.19]),
        (0.015, 1.0, [2.0, 0.20, 0.19]),
    ] {
        let mut c = fixture(depth, station, size);
        assert!(
            c.apply_rule("fit", "seat", &RoundPurlinSeatRule, &params)
                .is_err(),
            "{depth} {station} {size:?}"
        );
        assert!(c.contacts().is_empty());
        assert!(c.applications().is_empty());
        assert_eq!(c.part_edits_for("purlin").count(), 0);
    }
}

#[test]
fn custom_dimensions_are_assessed_independently_of_defaults() {
    let mut c = fixture(0.01, 1.0, [2.0, 0.12, 0.12]);
    assert!(matches!(
        RoundPurlinSeatRule.assess(&RuleContext::new(&c, "seat").unwrap()),
        Applicability::Suitable(_)
    ));
    let params = RoundPurlinSeatParams {
        seat_depth: default_millimeters(10),
        minimum_remaining_depth: default_millimeters(110),
        ..RoundPurlinSeatParams::default()
    };
    c.apply_rule("fit", "seat", &RoundPurlinSeatRule, &params)
        .unwrap();
}

#[test]
fn exact_relish_boundary_and_reflected_world_frames_are_supported() {
    let frame = OrientedBox {
        origin: [3.0, -4.0, 5.0],
        axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        size: [1.0; 3],
    };
    let mut c = fixture_in_frame(0.015, 2.0 - 0.1005 - 0.05, [2.0, 0.19, 0.19], &frame);
    c.apply_rule(
        "fit",
        "seat",
        &RoundPurlinSeatRule,
        &RoundPurlinSeatParams::default(),
    )
    .unwrap();
    let recipe = compose(&c, c.element("purlin").unwrap()).unwrap();
    let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    assert!(result.report.clean_at(Severity::Warning));
    assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
    let measurement = measure_contact(&c, &c.contacts()[0]).unwrap();
    assert!(measurement.anchors_coincide());
}
