// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;

use exedra_constructive::edge_finish::{EdgeSelection, RoundPolicy};
use exedra_constructive::evaluate::{Severity, evaluate};
use exedra_constructive::ir::PrimitiveSpec;
use exedra_constructive::tessellate::EvalPolicy;

use super::*;
use crate::{
    Evidence, EvidenceClass, OrientedBox, Part, PartEdit, Relation, RelationKind, RuleApplication,
    RuleOutput, ToolSolid,
};

fn block(builder: &mut RecipeBuilder, size: [f64; 3], slot: &str, finish: bool) -> NodeId {
    let material = builder.material_slot(slot);
    let source = builder.source_ref(slot);
    let child = builder
        .with_material(material)
        .with_source(source)
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    if finish {
        builder
            .add(NodeKind::EdgeFinish {
                child,
                selection: EdgeSelection::SharpEdges,
                policy: RoundPolicy::chamfer(0.03),
            })
            .unwrap()
    } else {
        child
    }
}

fn recipe(size: [f64; 3], slot: &str, finish: bool) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let root = block(&mut builder, size, slot, finish);
    builder.finish(root).unwrap()
}

#[test]
fn joint_cuts_preserve_finishes_on_hosts_and_tools() {
    for (finish_host, finish_tool) in [(true, false), (false, true), (true, true)] {
        let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
        let host_size = [2.0; 3];
        let tool_size = [0.4, 0.6, 3.0];
        let placement = Placement3::translate(0.7, 0.7, -0.5);
        let mut construction = Construction::new();
        construction
            .add_element(
                Element::new(
                    "host",
                    "timber",
                    "oak",
                    OrientedBox::axis_aligned([0.0; 3], host_size),
                    evidence.clone(),
                )
                .with_part(Part::new(recipe(host_size, "host", finish_host)).with_slot("host")),
            )
            .unwrap();
        construction
            .add_relation(Relation::new(
                "joint",
                RelationKind::host_fill("host"),
                "mortise",
                evidence.clone(),
            ))
            .unwrap();
        let mut output = RuleOutput::new();
        output.edit(PartEdit::remove(
            "host",
            ToolSolid::new("cutter", recipe(tool_size, "cut", finish_tool), placement),
            evidence.clone(),
        ));
        construction
            .apply(RuleApplication::new(
                "cut", "fixture", "joint", evidence, output,
            ))
            .unwrap();

        let composed = compose(&construction, construction.element("host").unwrap())
            .expect("finishing and fitting must compose");
        // Build the same cut directly: the tool's node, source and material
        // indices move when it is spliced behind the host's finished recipe.
        let mut direct = RecipeBuilder::new();
        let host = block(&mut direct, host_size, "host", finish_host);
        let tool = block(&mut direct, tool_size, "cut", finish_tool);
        let tool = direct
            .add(NodeKind::Transform {
                child: tool,
                xf: placement,
            })
            .unwrap();
        let material = direct.material_slot("host");
        let root = direct
            .with_material(material)
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![host, tool],
            })
            .unwrap();
        let expected = direct.finish(root).unwrap();
        assert_eq!(composed.slots(), expected.slots());
        assert_eq!(composed.recipe_fingerprint(), expected.recipe_fingerprint());
        let result = evaluate(&composed, &EvalPolicy::default()).unwrap();
        assert!(
            result.report.clean_at(Severity::Warning),
            "{:?}",
            result.report
        );
        assert_eq!(result.bodies.len(), 1);
        assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
    }
}

#[test]
fn shared_lowering_keeps_exceptional_cuts_materials_and_generated_identity() {
    let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
    let template = recipe([2.0; 3], "surface", false);
    let element = |key: &str, x, material: &str| {
        Element::new(
            key,
            "beam",
            material,
            OrientedBox::axis_aligned([x, 0.0, 0.0], [2.0; 3]),
            evidence.clone(),
        )
        .with_part(Part::new(template.clone()))
    };
    let mut construction = Construction::new();
    for e in [
        element("a", 0.0, "oak"),
        element("b", 3.0, "oak"),
        element("c", 6.0, "paint"),
    ] {
        construction.add_element(e).unwrap();
    }
    assert_eq!(
        lower_shared(&construction, |_| "beam".into())
            .unwrap()
            .parts()
            .len(),
        1
    );
    construction
        .add_relation(Relation::new(
            "joint",
            RelationKind::host_fill("b"),
            "mortise",
            evidence.clone(),
        ))
        .unwrap();
    let mut output = RuleOutput::new();
    output.edit(PartEdit::remove(
        "b",
        ToolSolid::new(
            "cut",
            recipe([0.4, 0.6, 3.0], "surface", false),
            Placement3::translate(0.7, 0.7, -0.5),
        ),
        evidence.clone(),
    ));
    output.generate(element("d", 9.0, "oak"));
    construction
        .apply(RuleApplication::new(
            "fit", "fixture", "joint", evidence, output,
        ))
        .unwrap();
    let assembly = lower_shared(&construction, |_| "beam".into()).unwrap();
    assert_eq!(
        assembly.parts().len(),
        2,
        "an exceptional cut needs its own part"
    );
    assert_eq!(assembly.instances().len(), 4);
    let instances: Vec<_> = ["a", "b", "c", "d"]
        .map(|key| assembly.resolve_path(&instance_path(key)).unwrap())
        .into();
    let part = |index: usize| {
        assembly
            .instance(instances[index])
            .unwrap()
            .part()
            .expect("mesh-bearing test instance")
    };
    assert_ne!(part(0), part(1));
    assert_eq!(part(0), part(2));
    assert_eq!(part(0), part(3));
    let slot = assembly
        .part(part(0))
        .unwrap()
        .slot_index("surface")
        .unwrap();
    assert_eq!(assembly.resolved_material(instances[0], slot), Some("oak"));
    assert_eq!(
        assembly.resolved_material(instances[2], slot),
        Some("paint")
    );
    for (key, instance) in ["a", "b", "c", "d"].into_iter().zip(instances) {
        let definition = assembly.instance(instance).unwrap();
        assert_eq!(
            *definition.placement(),
            construction.element(key).unwrap().extent.placement()
        );
        let metadata = definition.metadata();
        assert!(metadata.contains(&("structural_role".into(), "beam".into())));
        assert!(metadata.contains(&("evidence_source".into(), "fixture".into())));
        assert_eq!(
            metadata
                .iter()
                .find(|(key, _)| key == "generated_by")
                .map(|(_, value)| value.as_str()),
            (key == "d").then_some("fit")
        );
        let PartSource::Recipe(actual) = assembly
            .part(definition.part().expect("mesh-bearing test instance"))
            .unwrap()
            .source()
        else {
            panic!("recipe")
        };
        assert_eq!(
            actual.recipe_fingerprint(),
            compose(&construction, construction.element(key).unwrap())
                .unwrap()
                .recipe_fingerprint()
        );
    }
    assert_eq!(
        lower(&construction).unwrap().parts().len(),
        4,
        "ordinary lowering keeps its existing identity contract"
    );
    construction.set_element_present("b", false).unwrap();
    let omitted = lower_shared(&construction, |_| "beam".into()).unwrap();
    assert_eq!(omitted.parts().len(), 1);
    assert!(omitted.resolve_path(&instance_path("b")).is_none());
}

#[test]
fn sharing_distinguishes_slot_tables_default_slots_and_sources() {
    let evidence = Evidence::new("fixture", EvidenceClass::ModernEngineeringInference);
    let mut construction = Construction::new();
    for (key, slot, default, source) in [
        ("a", "surface", "surface", "template"),
        ("b", "paint", "paint", "template"),
        ("c", "surface", "end", "template"),
        ("d", "surface", "surface", "different-source"),
    ] {
        let mut builder = RecipeBuilder::new();
        let material = builder.material_slot(slot);
        builder.material_slot("end");
        let source = builder.source_ref(source);
        let root = builder
            .with_material(material)
            .with_source(source)
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box { size: [1.0; 3] },
                placement: Placement3::IDENTITY,
            })
            .unwrap();
        construction
            .add_element(
                Element::new(
                    key,
                    "block",
                    "oak",
                    OrientedBox::axis_aligned([0.0; 3], [1.0; 3]),
                    evidence.clone(),
                )
                .with_part(Part::new(builder.finish(root).unwrap()).with_slot(default)),
            )
            .unwrap();
    }
    let assembly = lower_shared(&construction, |_| "block".into()).unwrap();
    assert_eq!(assembly.parts().len(), 4);
}
