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
