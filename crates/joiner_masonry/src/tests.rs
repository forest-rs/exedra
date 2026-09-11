// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{CircularOpening, Length, RunningBondParams, RunningBondRule};
use alloc::collections::BTreeSet;
use exedra_constructive::evaluate::{Severity, evaluate};
use exedra_constructive::tessellate::EvalPolicy;
use joiner::{
    Construction, Element, Evidence, EvidenceClass, EvidenceSource, OrientedBox, Relation,
    RelationKind, Rule, RuleContext,
};

fn mm(value: u64) -> Length {
    Length::millimeters(value).unwrap()
}
fn setup() -> Construction {
    setup_extent(OrientedBox::axis_aligned([0.0; 3], [5.0, 0.3, 4.0]))
}
fn setup_extent(extent: OrientedBox) -> Construction {
    let mut c = Construction::new();
    let evidence = Evidence::new("authored", EvidenceClass::ModernEngineeringInference);
    c.add_evidence_source(EvidenceSource::new(
        "authored",
        evidence.class,
        "local:fixture",
        "Authored geometric fixture",
    ))
    .unwrap();
    c.add_element(Element::new("wall", "wall", "brick", extent, evidence.clone()).without_part())
        .unwrap();
    c.add_relation(Relation::new(
        "bond",
        RelationKind::element_units("wall"),
        "running-bond",
        evidence,
    ))
    .unwrap();
    c
}
fn params() -> RunningBondParams {
    RunningBondParams {
        unit_length: mm(240),
        unit_height: mm(65),
        joint: mm(10),
        minimum_closure: mm(30),
        opening: None,
    }
}

#[test]
fn repeated_units_share_shapes_and_expansion_emits_no_parent_solid() {
    let mut c = setup();
    c.apply_rule("course-wall", "bond", &RunningBondRule, &params())
        .unwrap();
    let a = joiner::lower_shared(&c, |e| e.role.clone()).unwrap();
    assert_eq!(a.instances().len(), c.elements().len() - 1);
    assert!(
        a.parts().len() <= 9,
        "rectangular units and closures should share: {}",
        a.parts().len()
    );
    assert!(a.resolve_path(&joiner::instance_path("wall")).is_none());
    assert!(c.elements().iter().skip(1).all(|e| {
        e.extent.size[0] >= 0.03
            && e.extent.size[0] <= 0.24 + 1e-12
            && e.extent.size[2] >= 0.03
            && e.extent.size[2] <= 0.065 + 1e-12
    }));
    assert!(
        !RunningBondRule
            .assess(&RuleContext::new(&c, "bond").unwrap())
            .is_suitable()
    );
}

#[test]
fn malformed_or_already_solid_walls_and_unbounded_layouts_are_refused() {
    let mut c = setup();
    let mut bad = params();
    bad.unit_height = mm(1);
    assert!(
        RunningBondRule
            .instantiate(&RuleContext::new(&c, "bond").unwrap(), &bad)
            .is_err()
    );
    bad = params();
    bad.opening = Some(CircularOpening {
        center: [f64::NAN, 1.0],
        radius: mm(1000),
        surround: mm(250),
        segments: 64,
    });
    assert!(
        RunningBondRule
            .instantiate(&RuleContext::new(&c, "bond").unwrap(), &bad)
            .is_err()
    );
    let e = c.elements()[0].evidence.clone();
    c.add_element(Element::new(
        "solid",
        "wall",
        "brick",
        OrientedBox::axis_aligned([0.0; 3], [1.0; 3]),
        e.clone(),
    ))
    .unwrap();
    c.add_relation(Relation::new(
        "bad",
        RelationKind::element_units("solid"),
        "bond",
        e,
    ))
    .unwrap();
    assert!(
        !RunningBondRule
            .assess(&RuleContext::new(&c, "bad").unwrap())
            .is_suitable()
    );
}

#[test]
fn circular_opening_produces_sound_surround_and_trimmed_courses() {
    let c = setup();
    let mut params = params();
    params.opening = Some(CircularOpening {
        center: [2.5, 1.9],
        radius: mm(1350),
        surround: mm(240),
        segments: 64,
    });
    let output = RunningBondRule
        .instantiate(&RuleContext::new(&c, "bond").unwrap(), &params)
        .unwrap();
    assert_eq!(
        output
            .generated
            .iter()
            .filter(|e| e.role == "masonry-surround")
            .count(),
        64
    );
    let mut seen = BTreeSet::new();
    let mut trimmed = 0;
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.001;
    for e in &output.generated {
        let recipe = &e.part.as_ref().unwrap().recipe;
        if recipe.nodes().len() > 1 {
            trimmed += 1;
        }
        if !seen.insert(recipe.recipe_fingerprint().0) {
            continue;
        }
        let result = evaluate(recipe, &policy).unwrap();
        assert!(
            result.report.clean_at(Severity::Warning),
            "{}: {:?}",
            e.key,
            result.report.diagnostics
        );
        assert_eq!(result.bodies.len(), 1, "{}", e.key);
        for body in &result.bodies {
            assert!(body.body.mesh.validate_deep().is_empty(), "{}", e.key);
        }
    }
    assert!(trimmed > 30);
    let keys: BTreeSet<_> = output.generated.iter().map(|e| e.key.clone()).collect();
    assert_eq!(keys.len(), output.generated.len());
    assert!(
        output.contacts.is_empty(),
        "open mortar joints are not contact claims"
    );
}

#[test]
fn nonunit_frames_and_excessive_work_are_rejected_before_generation() {
    for extent in [
        OrientedBox {
            origin: [0.0; 3],
            axes: [[0.0; 3]; 3],
            size: [5.0, 0.3, 4.0],
        },
        OrientedBox {
            origin: [0.0; 3],
            axes: [[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            size: [5.0, 0.3, 4.0],
        },
        OrientedBox::axis_aligned([0.0; 3], [10_000.0, 0.3, 100.0]),
    ] {
        let c = setup_extent(extent);
        assert!(
            RunningBondRule
                .instantiate(&RuleContext::new(&c, "bond").unwrap(), &params())
                .is_err()
        );
    }
}

#[test]
fn reflected_placements_and_changed_openings_preserve_unchanged_unit_identity() {
    let c = setup_extent(OrientedBox {
        origin: [12.0, -3.0, 2.0],
        axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        size: [5.0, 0.3, 4.0],
    });
    let mut p = params();
    let baseline = RunningBondRule
        .instantiate(&RuleContext::new(&c, "bond").unwrap(), &p)
        .unwrap();
    p.opening = Some(CircularOpening {
        center: [2.5, 1.9],
        radius: mm(1350),
        surround: mm(240),
        segments: 64,
    });
    let opened = RunningBondRule
        .instantiate(&RuleContext::new(&c, "bond").unwrap(), &p)
        .unwrap();
    let first = &baseline.generated[0];
    let unchanged = opened
        .generated
        .iter()
        .find(|e| e.key == first.key)
        .unwrap();
    assert_eq!(unchanged.extent, first.extent);
    assert_eq!(
        unchanged.part.as_ref().unwrap().recipe.recipe_fingerprint(),
        first.part.as_ref().unwrap().recipe.recipe_fingerprint()
    );
    let policy = EvalPolicy::default();
    for unit in opened
        .generated
        .iter()
        .filter(|e| e.role == "masonry-surround")
        .step_by(7)
    {
        let body = evaluate(&unit.part.as_ref().unwrap().recipe, &policy).unwrap();
        for vertex in body.bodies[0].body.mesh.vertices() {
            let p = body.bodies[0]
                .body
                .mesh
                .vertex_position(vertex)
                .unwrap()
                .map(f64::from);
            assert!(
                p.iter()
                    .zip(unit.extent.size)
                    .all(|(v, size)| *v >= -0.00001 && *v <= size + 0.00001)
            );
            let world = unit.extent.anchor(p);
            assert!(c.elements()[0].extent.contains_point(world, 0.00001));
        }
    }
}

#[test]
fn end_cuts_preserve_stock_limits_and_continuous_joints() {
    for (width, height) in [(1.005, 0.325), (5.0, 4.0), (0.245, 0.13)] {
        let c = setup_extent(OrientedBox::axis_aligned([0.0; 3], [width, 0.3, height]));
        let output = RunningBondRule
            .instantiate(&RuleContext::new(&c, "bond").unwrap(), &params())
            .unwrap();
        let mut row_z = 0.0;
        let mut row_top = 0.0;
        let mut right = -0.01;
        for unit in &output.generated {
            let [x, _, z] = unit.extent.origin;
            let [length, _, height] = unit.extent.size;
            assert!((0.03 - 1e-12..=0.24 + 1e-12).contains(&length));
            assert!((0.03 - 1e-12..=0.065 + 1e-12).contains(&height));
            if z != row_z {
                assert!((right - width).abs() < 1e-12);
                assert!((z - row_top - 0.01).abs() < 1e-12);
                row_z = z;
                right = -0.01;
            }
            assert!((x - right - 0.01).abs() < 1e-12);
            right = x + length;
            row_top = z + height;
        }
        assert!((right - width).abs() < 1e-12);
        assert!((row_top - height).abs() < 1e-12);
    }
    let c = setup_extent(OrientedBox::axis_aligned([0.0; 3], [1.0, 0.3, 0.066]));
    let mut p = params();
    p.minimum_closure = mm(40);
    assert!(
        RunningBondRule
            .instantiate(&RuleContext::new(&c, "bond").unwrap(), &p)
            .is_err()
    );
}
