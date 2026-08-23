// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The two seed cases the IR was shaped by, as hand-authored rule outputs.
//!
//! No rule exists yet: these build a [`RuleOutput`] by hand and hand it to
//! [`Construction::apply`], which is exactly what a rule library will do. That
//! is the point of the fixtures. They prove the IR expresses a **truss heel**
//! (member/member: two members cut to fit, one bearing face, one transfer)
//! and a **window opening** (host/fill: a wall voided and pocketed, sill and
//! lintel generated, bearing faces, a declared clearance, transfers) without
//! bending toward either — the same four output lists, the same validation,
//! the same lowering.
//!
//! Both fixtures also pin the parts of the IR that are easy to get wrong: a
//! clearance-only contact carries nothing, a cut composes into the
//! participant's own recipe rather than between instances, and instance paths
//! are the element keys.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use exedra_constructive::ir::{CsgOp, NodeKind, Placement3};

use crate::construction::{Construction, channel};
use crate::element::{Element, ElementOrigin, Member, Node, Part, Support};
use crate::evidence::{Evidence, EvidenceClass, EvidenceSource};
use crate::geometry::OrientedBox;
use crate::lower::{compose, lower};
use crate::relation::{Relation, RelationKind};
use crate::rule::{
    Anchor, ContactMeaning, ContactPatch, PartEdit, RuleApplication, RuleOutput, ToolSolid,
    TransferEdge, TransferKind, TransferTarget,
};
use crate::validate::{is_witnessed, trace_to_ground, validate};

const SOURCE: &str = "seed-fixture";

fn evidence() -> Evidence {
    Evidence::new(SOURCE, EvidenceClass::ModernEngineeringInference)
}

fn seeded() -> Construction {
    let mut construction = Construction::new();
    construction
        .add_evidence_source(EvidenceSource::new(
            SOURCE,
            EvidenceClass::ModernEngineeringInference,
            "https://example.invalid/seed-fixture",
            "A shape-of-the-IR fixture, not a construction claim about any building",
        ))
        .expect("fresh evidence source");
    construction
}

fn tool(key: &str, size: [f64; 3], at: [f64; 3]) -> ToolSolid {
    ToolSolid::new(
        key,
        Part::boxed(size, &alloc::format!("joiner:tool/{key}")).expect("positive tool extent"),
        Placement3::translate(at[0], at[1], at[2]),
    )
}

/// Every instance path, in lowering order. Instances are all roots, so a
/// path is exactly its element key.
fn instance_paths(assembly: &exedra_assembly::Assembly) -> Vec<String> {
    (0..assembly.instances().len())
        .filter_map(|index| {
            u32::try_from(index)
                .ok()
                .and_then(|index| assembly.path_of(exedra_assembly::InstanceId(index)))
        })
        .map(|path| alloc::format!("{path}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Seed 1: a truss heel. Member/member.
// ---------------------------------------------------------------------------

/// A rafter footed on a tie beam, both cut to fit, bearing on one face.
///
/// The tie beam runs along +x. The rafter rises from a seat on the tie's top
/// face on a 4:3 slope. Its extent origin *is* the seat point, so its
/// bearing anchor and the tie's coincide by construction rather than by luck.
fn truss_heel() -> Construction {
    let mut construction = seeded();

    let tie_extent = OrientedBox::axis_aligned([0.0, 0.0, 0.0], [4.0, 0.30, 0.30]);
    let seat = tie_extent.anchor([0.60, 0.15, 0.30]);
    let rafter_extent = OrientedBox {
        origin: [seat[0], seat[1] - 0.10, seat[2]],
        // A 4:3 slope in the xz plane: along, across, up-slope normal.
        axes: [[0.8, 0.0, 0.6], [0.0, 1.0, 0.0], [-0.6, 0.0, 0.8]],
        size: [2.50, 0.20, 0.24],
    };

    construction
        .add_element(
            Element::new(
                "tie-beam",
                "tie-beam",
                "oak",
                tie_extent.clone(),
                evidence(),
            )
            .with_required_supports(1)
            .with_member(),
        )
        .expect("tie beam");
    construction
        .add_element(
            Element::new(
                "principal-rafter",
                "principal-rafter",
                "oak",
                rafter_extent.clone(),
                evidence(),
            )
            .with_required_supports(1)
            .with_member(),
        )
        .expect("rafter");

    construction
        .add_node(Node::new("node-heel", seat))
        .expect("heel node");
    construction
        .add_node(Node::new(
            "node-tie-east",
            tie_extent.anchor([3.90, 0.15, 0.15]),
        ))
        .expect("tie east node");
    construction
        .add_node(Node::new(
            "node-apex",
            rafter_extent.anchor([2.50, 0.10, 0.12]),
        ))
        .expect("apex node");
    construction
        .add_member(Member::new(
            "tie-beam",
            "tie-beam",
            "node-heel",
            "node-tie-east",
            evidence(),
        ))
        .expect("tie member");
    construction
        .add_member(Member::new(
            "principal-rafter",
            "principal-rafter",
            "node-heel",
            "node-apex",
            evidence(),
        ))
        .expect("rafter member");

    // The relation *is* the joint: it witnesses joint transfers and it is
    // what a `joiner_timber` rule will be handed in stage 2.
    construction
        .add_relation(Relation::new(
            "heel-west",
            RelationKind::member_member("node-heel", &["principal-rafter", "tie-beam"]),
            "birdsmouth-on-housed-seat",
            evidence(),
        ))
        .expect("heel relation");

    construction
        .add_support(Support::fixed("support-tie-beam", "tie-beam", "ground"))
        .expect("support");
    construction
        .add_transfer(TransferEdge::new(
            "load-tie-beam-to-ground",
            "tie-beam",
            TransferTarget::support("support-tie-beam"),
            TransferKind::Ground,
        ))
        .expect("ground transfer");

    // What a heel rule will return: a cut on each participant, the face they
    // bear on, and the load that face carries.
    let mut output = RuleOutput::new();
    output
        .edit(PartEdit::remove(
            "principal-rafter",
            tool("heel-birdsmouth", [0.42, 0.40, 0.14], [-0.06, -0.10, -0.02]),
            evidence(),
        ))
        .edit(PartEdit::remove(
            "tie-beam",
            tool("heel-housing", [0.36, 0.24, 0.04], [0.52, 0.03, 0.26]),
            evidence(),
        ))
        .contact(
            ContactPatch::new(
                "contact-rafter-heel-on-tie-beam",
                Anchor::new("principal-rafter", [0.0, 0.10, 0.0]),
                Anchor::new("tie-beam", [0.60, 0.15, 0.30]),
                [0.0, 0.0, 1.0],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                ContactMeaning::Bearing,
                evidence(),
            )
            .with_minimum_overlap([0.15, 0.15])
            .with_detail("heel-seat"),
        )
        .transfer(TransferEdge::new(
            "load-rafter-to-tie-beam",
            "principal-rafter",
            TransferTarget::element("tie-beam"),
            TransferKind::Contact,
        ));

    construction
        .apply(RuleApplication::new(
            "fit-heel-west",
            "seed:truss-heel",
            "heel-west",
            evidence(),
            output,
        ))
        .expect("heel output merges");
    construction
}

// ---------------------------------------------------------------------------

/// A wall voided for a window and filled by a generated sill, two jambs, and
/// a lintel.
///
/// The host takes two cuts: the rough opening, and an unrelated putlog hole
/// well away from it, so lowering's fold of several edits into one n-ary
/// difference is exercised rather than assumed.
///
/// The fills carry each other: the lintel bears on the jambs, the jambs on
/// the sill, the sill on the wall, the wall on the ground. That chain is the
/// point — a host/fill rule generates parts that are structural participants,
/// not decoration, and the load path runs through them like any other.
///
/// The opening is cut 5 mm taller than the pieces filling it, and that gap is
/// declared rather than left implicit: a [`ContactMeaning::ClearanceOnly`]
/// patch, which by construction carries no load.
fn window_opening() -> Construction {
    let mut construction = seeded();

    let wall_extent = OrientedBox::axis_aligned([0.0, 0.0, 0.0], [6.0, 0.60, 4.0]);
    construction
        .add_element(
            Element::new(
                "wall-north",
                "wall",
                "ashlar",
                wall_extent.clone(),
                evidence(),
            )
            .with_required_supports(1),
        )
        .expect("wall");
    construction
        .add_support(Support::fixed("support-wall-north", "wall-north", "ground"))
        .expect("support");
    construction
        .add_transfer(TransferEdge::new(
            "load-wall-north-to-ground",
            "wall-north",
            TransferTarget::support("support-wall-north"),
            TransferKind::Ground,
        ))
        .expect("ground transfer");
    construction
        .add_relation(Relation::new(
            "clerestory-window-01",
            RelationKind::host_fill("wall-north"),
            "trabeated-opening",
            evidence(),
        ))
        .expect("opening relation");

    let sill_extent = OrientedBox::axis_aligned([2.30, 0.0, 1.60], [1.40, 0.60, 0.20]);
    let jamb_west_extent = OrientedBox::axis_aligned([2.30, 0.0, 1.80], [0.15, 0.60, 1.40]);
    let jamb_east_extent = OrientedBox::axis_aligned([3.55, 0.0, 1.80], [0.15, 0.60, 1.40]);
    let lintel_extent = OrientedBox::axis_aligned([2.30, 0.0, 3.20], [1.40, 0.60, 0.20]);

    let bearing = |key: &str, carried: Anchor, carrier: Anchor, minimum: [f64; 2], detail: &str| {
        ContactPatch::new(
            key,
            carried,
            carrier,
            [0.0, 0.0, 1.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            ContactMeaning::Bearing,
            evidence(),
        )
        .with_minimum_overlap(minimum)
        .with_detail(detail)
    };

    let mut output = RuleOutput::new();
    output
        // The rough opening, cut clean through the wall and 5 mm taller than
        // what fills it.
        .edit(PartEdit::remove(
            "wall-north",
            tool("window-void", [1.40, 0.80, 1.805], [2.30, -0.10, 1.60]),
            evidence(),
        ))
        // A putlog hole, unrelated to the window and far from it.
        .edit(PartEdit::remove(
            "wall-north",
            tool("putlog-hole", [0.30, 0.80, 0.30], [0.60, -0.10, 1.00]),
            evidence(),
        ))
        .generate(
            Element::new(
                "sill-clerestory-01",
                "sill",
                "ashlar",
                sill_extent,
                evidence(),
            )
            .with_required_supports(1),
        )
        .generate(
            Element::new(
                "jamb-west-clerestory-01",
                "jamb",
                "ashlar",
                jamb_west_extent,
                evidence(),
            )
            .with_required_supports(1),
        )
        .generate(
            Element::new(
                "jamb-east-clerestory-01",
                "jamb",
                "ashlar",
                jamb_east_extent,
                evidence(),
            )
            .with_required_supports(1),
        )
        .generate(
            Element::new(
                "lintel-clerestory-01",
                "lintel",
                "ashlar",
                lintel_extent,
                evidence(),
            )
            .with_required_supports(2),
        )
        .contact(bearing(
            "contact-sill-on-wall-north",
            Anchor::new("sill-clerestory-01", [0.70, 0.30, 0.0]),
            Anchor::new("wall-north", [3.00, 0.30, 1.60]),
            [1.00, 0.40],
            "mortar-bedded-sill",
        ))
        .contact(bearing(
            "contact-jamb-west-on-sill",
            Anchor::new("jamb-west-clerestory-01", [0.075, 0.30, 0.0]),
            Anchor::new("sill-clerestory-01", [0.075, 0.30, 0.20]),
            [0.10, 0.40],
            "coursed-jamb-foot",
        ))
        .contact(bearing(
            "contact-jamb-east-on-sill",
            Anchor::new("jamb-east-clerestory-01", [0.075, 0.30, 0.0]),
            Anchor::new("sill-clerestory-01", [1.325, 0.30, 0.20]),
            [0.10, 0.40],
            "coursed-jamb-foot",
        ))
        .contact(bearing(
            "contact-lintel-on-jamb-west",
            Anchor::new("lintel-clerestory-01", [0.075, 0.30, 0.0]),
            Anchor::new("jamb-west-clerestory-01", [0.075, 0.30, 1.40]),
            [0.10, 0.40],
            "lintel-end-bearing",
        ))
        .contact(bearing(
            "contact-lintel-on-jamb-east",
            Anchor::new("lintel-clerestory-01", [1.325, 0.30, 0.0]),
            Anchor::new("jamb-east-clerestory-01", [0.075, 0.30, 1.40]),
            [0.10, 0.40],
            "lintel-end-bearing",
        ))
        // The 5 mm over the lintel: declared, not forgotten.
        .contact(
            ContactPatch::new(
                "clearance-over-lintel",
                Anchor::new("lintel-clerestory-01", [0.70, 0.30, 0.20]),
                Anchor::new("wall-north", [3.00, 0.30, 3.405]),
                [0.0, 0.0, -1.0],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                ContactMeaning::ClearanceOnly,
                evidence(),
            )
            .with_detail("settlement-gap"),
        )
        .transfer(TransferEdge::new(
            "load-lintel-to-jamb-west",
            "lintel-clerestory-01",
            TransferTarget::element("jamb-west-clerestory-01"),
            TransferKind::Contact,
        ))
        .transfer(TransferEdge::new(
            "load-lintel-to-jamb-east",
            "lintel-clerestory-01",
            TransferTarget::element("jamb-east-clerestory-01"),
            TransferKind::Contact,
        ))
        .transfer(TransferEdge::new(
            "load-jamb-west-to-sill",
            "jamb-west-clerestory-01",
            TransferTarget::element("sill-clerestory-01"),
            TransferKind::Contact,
        ))
        .transfer(TransferEdge::new(
            "load-jamb-east-to-sill",
            "jamb-east-clerestory-01",
            TransferTarget::element("sill-clerestory-01"),
            TransferKind::Contact,
        ))
        .transfer(TransferEdge::new(
            "load-sill-to-wall-north",
            "sill-clerestory-01",
            TransferTarget::element("wall-north"),
            TransferKind::Contact,
        ));

    construction
        .apply(RuleApplication::new(
            "fit-clerestory-window-01",
            "seed:trabeated-opening",
            "clerestory-window-01",
            evidence(),
            output,
        ))
        .expect("opening output merges");
    construction
}

// ---------------------------------------------------------------------------

#[test]
fn both_seed_cases_validate_clean() {
    for construction in [truss_heel(), window_opening()] {
        let report = validate(&construction);
        assert!(report.is_clean(), "{report}");
    }
}

#[test]
fn the_heel_cuts_both_members_and_carries_the_rafter_to_ground() {
    let construction = truss_heel();

    // Both participants are cut. Neither cut is a mesh operation: each is a
    // constructive difference on that participant's own recipe.
    for element in ["principal-rafter", "tie-beam"] {
        assert_eq!(
            construction.part_edits_for(element).count(),
            1,
            "{element} is cut to fit"
        );
        let recipe = compose(
            &construction,
            construction.element(element).expect("element exists"),
        )
        .expect("composes");
        assert!(
            matches!(
                recipe.node(recipe.root()).map(|node| &node.kind),
                Some(NodeKind::Csg {
                    op: CsgOp::Difference,
                    ..
                })
            ),
            "{element} root is a constructive difference"
        );
    }

    assert_eq!(
        trace_to_ground(&construction, "principal-rafter"),
        Some(alloc::vec![
            "principal-rafter".to_string(),
            "tie-beam".to_string(),
            "support-tie-beam".to_string(),
            "ground".to_string(),
        ])
    );
}

#[test]
fn the_heel_relation_is_the_joint_witness() {
    let mut construction = truss_heel();
    // A joint transfer is witnessed by the member/member relation alone: no
    // separate joint record exists to agree or disagree with it.
    construction
        .add_transfer(TransferEdge::new(
            "load-rafter-to-tie-beam-by-joint",
            "principal-rafter",
            TransferTarget::element("tie-beam"),
            TransferKind::Joint,
        ))
        .expect("joint transfer");
    let transfer = construction
        .transfer("load-rafter-to-tie-beam-by-joint")
        .expect("transfer exists");
    assert!(is_witnessed(&construction, transfer));

    // Move the tie beam away from the heel node and the same claim fails:
    // the relation no longer touches both members.
    let moved = construction
        .element("tie-beam")
        .expect("tie beam")
        .extent
        .translated([0.0, 0.0, -1.0]);
    construction
        .set_element_extent("tie-beam", moved)
        .expect("known element");
    let transfer = construction
        .transfer("load-rafter-to-tie-beam-by-joint")
        .expect("transfer exists");
    assert!(!is_witnessed(&construction, transfer));
    let report = validate(&construction);
    assert!(report.has("relation-not-incident-to-member", "heel-west"));
    assert!(report.has("floating-contact", "contact-rafter-heel-on-tie-beam"));
}

#[test]
fn the_window_generates_its_fills_and_records_them_on_the_relation() {
    let construction = window_opening();

    for key in [
        "sill-clerestory-01",
        "jamb-west-clerestory-01",
        "jamb-east-clerestory-01",
        "lintel-clerestory-01",
    ] {
        let element = construction.element(key).expect("generated element");
        assert_eq!(
            element.origin,
            ElementOrigin::Generated("fit-clerestory-window-01".to_string()),
            "{key} records the fit that made it"
        );
    }
    assert_eq!(
        construction
            .relation("clerestory-window-01")
            .map(|relation| relation.kind.clone()),
        Some(RelationKind::HostFill {
            host: "wall-north".to_string(),
            fills: alloc::vec![
                "sill-clerestory-01".to_string(),
                "jamb-west-clerestory-01".to_string(),
                "jamb-east-clerestory-01".to_string(),
                "lintel-clerestory-01".to_string(),
            ],
        })
    );

    // Two cuts on the host fold into exactly one n-ary difference.
    assert_eq!(construction.part_edits_for("wall-north").count(), 2);
    let wall = construction.element("wall-north").expect("host");
    let recipe = compose(&construction, wall).expect("composes");
    let differences: Vec<usize> = recipe
        .nodes()
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Csg {
                op: CsgOp::Difference,
                operands,
            } => Some(operands.len()),
            _ => None,
        })
        .collect();
    assert_eq!(
        differences,
        [3],
        "one difference, base plus both cutters, not a chain"
    );
}

#[test]
fn a_clearance_only_contact_carries_nothing() {
    let mut construction = window_opening();
    // Asserting a transfer through the declared gap does not create one.
    construction
        .add_transfer(TransferEdge::new(
            "load-lintel-to-wall-through-the-gap",
            "lintel-clerestory-01",
            TransferTarget::element("wall-north"),
            TransferKind::Joint,
        ))
        .expect("transfer");
    let transfer = construction
        .transfer("load-lintel-to-wall-through-the-gap")
        .expect("transfer exists");
    assert!(
        !is_witnessed(&construction, transfer),
        "a clearance is not a bearing"
    );
    assert!(validate(&construction).has(
        "unwitnessed-joint-transfer",
        "load-lintel-to-wall-through-the-gap"
    ));
}

#[test]
fn omitting_the_host_breaks_every_claim_that_rested_on_it() {
    let mut construction = window_opening();
    construction
        .set_element_present("wall-north", false)
        .expect("known element");
    let report = validate(&construction);
    assert!(report.has("no-ground-path", "sill-clerestory-01"));
    assert!(report.has("no-ground-path", "lintel-clerestory-01"));
    assert!(report.has("missing-contact-element", "contact-sill-on-wall-north"));
    assert!(report.has("omitted-relation-participant", "clerestory-window-01"));
    assert!(report.has("part-edit-on-omitted-element", "wall-north"));
}

#[test]
fn both_seed_cases_lower_to_paths_derived_from_element_keys() {
    let heel = lower(&truss_heel()).expect("heel lowers");
    assert_eq!(instance_paths(&heel), ["tie-beam", "principal-rafter"]);
    assert!(
        heel.roots().len() == heel.instances().len(),
        "structural connectivity is never parent-child placement"
    );
    assert!(heel.part_by_key("part-tie-beam").is_some());

    let window = lower(&window_opening()).expect("window lowers");
    assert_eq!(
        instance_paths(&window),
        [
            "wall-north",
            "sill-clerestory-01",
            "jamb-west-clerestory-01",
            "jamb-east-clerestory-01",
            "lintel-clerestory-01",
        ]
    );
    let wall = window
        .resolve_path(&crate::lower::instance_path("wall-north"))
        .expect("host instance");
    let metadata = window.instance(wall).expect("instance").metadata();
    assert!(metadata.contains(&("structural_role".to_string(), "wall".to_string())));
    let sill = window
        .resolve_path(&crate::lower::instance_path("sill-clerestory-01"))
        .expect("generated instance");
    assert!(
        window
            .instance(sill)
            .expect("instance")
            .metadata()
            .contains(&(
                "generated_by".to_string(),
                "fit-clerestory-window-01".to_string()
            ))
    );
}

#[test]
fn both_seed_cases_compile_to_valid_geometry() {
    use exedra_assembly::PartCompiler;
    use exedra_constructive::tessellate::EvalPolicy;

    for construction in [truss_heel(), window_opening()] {
        let assembly = lower(&construction).expect("lowers");
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&assembly, &EvalPolicy::default())
            .expect("every composed recipe evaluates");
        for part in compiled.parts() {
            for body in &part.bodies {
                assert!(!body.tri.indices.is_empty(), "cut parts still have faces");
                assert!(
                    body.tri
                        .positions
                        .iter()
                        .flatten()
                        .all(|value| value.is_finite()),
                    "positions stay finite through the cuts"
                );
            }
        }
    }

    // The cuts are real: an uncut box tessellates to twelve triangles, and
    // the voided wall to a good many more.
    let window = window_opening();
    let assembly = lower(&window).expect("lowers");
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&assembly, &EvalPolicy::default())
        .expect("compiles");
    let wall = assembly
        .part_by_key(&crate::lower::part_key("wall-north"))
        .expect("host part");
    let triangles: usize = compiled
        .part(wall)
        .expect("host compiled")
        .bodies
        .iter()
        .map(|body| body.tri.indices.len() / 3)
        .sum();
    for part in compiled.parts() {
        std::eprintln!("part bodies={}", part.bodies.len());
    }
    assert!(
        triangles > 12,
        "the opening removed material: {triangles} triangles"
    );
}

#[test]
fn editing_one_element_dirties_only_what_depends_on_it() {
    let mut construction = window_opening();
    construction.clear_dirty();
    let moved = construction
        .element("lintel-clerestory-01")
        .expect("lintel")
        .extent
        .translated([0.0, 0.0, 0.02]);
    construction
        .set_element_extent("lintel-clerestory-01", moved)
        .expect("known element");
    assert_eq!(
        construction.take_dirty(channel::GEOMETRY),
        ["lintel-clerestory-01"]
    );
    // The move opened a gap under the lintel, and validation says so.
    let report = validate(&construction);
    assert!(report.has("floating-contact", "contact-lintel-on-jamb-west"));
    assert!(report.has("floating-contact", "contact-lintel-on-jamb-east"));
    assert!(report.has("unwitnessed-contact-transfer", "load-lintel-to-jamb-west"));
    assert!(report.has("no-ground-path", "lintel-clerestory-01"));
}
