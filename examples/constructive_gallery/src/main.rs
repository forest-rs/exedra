// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The spearhead gallery: six shapes through the public constructive
//! surface, standing in for an external spec compiler.

use exedra::ExtractParams;
use exedra_constructive::builders;
use exedra_constructive::evaluate::evaluate;
use exedra_constructive::ir::{CapMode, CsgOp, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::profile::{Loop2, Profile2, Seg2, SegKind, SegTag};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_testkit::golden::trimesh_signature;

/// One gallery scenario: a named recipe.
#[derive(Debug)]
pub struct Scenario {
    /// Stable scenario name.
    pub name: &'static str,
    /// The recipe, built through the public API only.
    pub recipe: Recipe,
}

/// Builds all six spearhead scenarios.
#[must_use]
pub fn scenarios() -> Vec<Scenario> {
    vec![
        rect_prism(),
        l_prism(),
        rounded_prism(),
        ring_prism(),
        quarter_sweep(),
        csg_difference(),
        policy_curve(),
    ]
}

fn rect_prism() -> Scenario {
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(builders::rect(600.0, 400.0).expect("rect"));
    let src = b.source_ref("gallery:rect_prism");
    let n = b
        .with_source(src)
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 720.0,
            caps: CapMode::Both,
        })
        .expect("valid extrude");
    Scenario {
        name: "rect_prism",
        recipe: b.finish(n).expect("valid recipe"),
    }
}

fn l_prism() -> Scenario {
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(builders::l_profile(600.0, 600.0, 300.0, 300.0).expect("L"));
    let src = b.source_ref("gallery:l_prism");
    let n = b
        .with_source(src)
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 500.0,
            caps: CapMode::Both,
        })
        .expect("valid extrude");
    Scenario {
        name: "l_prism",
        recipe: b.finish(n).expect("valid recipe"),
    }
}

fn rounded_prism() -> Scenario {
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(builders::rounded_rect(400.0, 300.0, 50.0).expect("rounded"));
    let src = b.source_ref("gallery:rounded_prism");
    let n = b
        .with_source(src)
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 200.0,
            caps: CapMode::Both,
        })
        .expect("valid extrude");
    Scenario {
        name: "rounded_prism",
        recipe: b.finish(n).expect("valid recipe"),
    }
}

fn ring_prism() -> Scenario {
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(builders::ring(120.0, 60.0).expect("ring"));
    let src = b.source_ref("gallery:ring_prism");
    let n = b
        .with_source(src)
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 40.0,
            caps: CapMode::Both,
        })
        .expect("valid extrude");
    Scenario {
        name: "ring_prism",
        recipe: b.finish(n).expect("valid recipe"),
    }
}

fn quarter_sweep() -> Scenario {
    // A hand-built profile with tagged segments, the way a compiler
    // attaches per-segment provenance.
    let outer = Loop2::new(vec![
        Seg2::line((340.0, 0.0)).tagged(SegTag(0)),
        Seg2::line((340.0, 80.0)).tagged(SegTag(1)),
        Seg2::line((260.0, 80.0)).tagged(SegTag(2)),
        Seg2::line((260.0, 0.0)).tagged(SegTag(3)),
    ])
    .expect("valid section");
    let profile = Profile2::simple(outer).expect("valid profile");
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(profile);
    let src = b.source_ref("gallery:quarter_sweep");
    let n = b
        .with_source(src)
        .add(NodeKind::Revolve {
            profile: p,
            placement: Placement3::IDENTITY,
            sweep: std::f64::consts::FRAC_PI_2,
            caps: CapMode::Both,
        })
        .expect("valid revolve");
    Scenario {
        name: "quarter_sweep",
        recipe: b.finish(n).expect("valid recipe"),
    }
}

fn csg_difference() -> Scenario {
    let mut b = RecipeBuilder::new();
    let block = b.add_profile(builders::rect(200.0, 100.0).expect("rect"));
    let cut = b.add_profile(builders::rect(80.0, 80.0).expect("rect"));
    let e1 = b
        .add(NodeKind::Extrude {
            profile: block,
            placement: Placement3::IDENTITY,
            height: 80.0,
            caps: CapMode::Both,
        })
        .expect("valid");
    // A corner notch: transversal crossings only (interior cut loops are
    // the pipeline's remaining typed deferral, tracked separately).
    let e2 = b
        .add(NodeKind::Extrude {
            profile: cut,
            placement: Placement3::translate(160.0, 60.0, -10.0),
            height: 100.0,
            caps: CapMode::Both,
        })
        .expect("valid");
    let csg = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![e1, e2],
        })
        .expect("valid");
    let moved = b
        .add(NodeKind::Transform {
            child: csg,
            xf: Placement3::rotate_z_then_translate(std::f64::consts::FRAC_PI_4, 50.0, 0.0, 0.0),
        })
        .expect("valid");
    Scenario {
        name: "csg_difference",
        recipe: b.finish(moved).expect("valid recipe"),
    }
}

/// An underspecified front edge: the spec does not close the curve, so the
/// compiler realizes it as a shallow arc under a named policy and cites the
/// spec issue — exercising the `PolicyDefined` and `Conflicted` fidelity
/// channels end to end.
fn policy_curve() -> Scenario {
    let mut b = RecipeBuilder::new();
    let policy = b.curve_policy("gallery.front-transition@1");
    let issue = b.source_ref("gallery.issue.front-profile-nonclosing");
    let src = b.source_ref("gallery:policy_curve");
    let outer = Loop2::new(vec![
        Seg2::line((500.0, 0.0)).tagged(SegTag(0)),
        Seg2::line((500.0, 280.0)).tagged(SegTag(1)),
        Seg2::policy((0.0, 300.0), policy, SegKind::Arc { bulge: -0.15 }).tagged(SegTag(2)),
        Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
    ])
    .expect("valid loop");
    let profile = Profile2::simple(outer).expect("valid profile");
    let p = b.add_profile(profile);
    let n = b
        .with_source(src)
        .with_issue(issue)
        .add(NodeKind::Extrude {
            profile: p,
            placement: Placement3::IDENTITY,
            height: 18.0,
            caps: CapMode::Both,
        })
        .expect("valid extrude");
    Scenario {
        name: "policy_curve",
        recipe: b.finish(n).expect("valid recipe"),
    }
}

/// Evaluates one scenario and renders its one-line summary.
fn summarize(scenario: &Scenario) -> String {
    let result = evaluate(&scenario.recipe, &EvalPolicy::default()).expect("scenario evaluates");
    let mut faces = 0;
    let mut signature = 0_u64;
    for placed in &result.bodies {
        faces += placed.body.mesh.faces().count();
        let (tri, _) = placed.body.mesh.to_trimesh(&ExtractParams::default());
        signature ^= trimesh_signature(&tri);
    }
    let diags = result.report.diagnostics.len();
    format!(
        "scenario={} fingerprint={:032x} bodies={} faces={} diagnostics={} signature={signature:016x}",
        scenario.name,
        scenario.recipe.recipe_fingerprint().0,
        result.bodies.len(),
        faces,
        diags,
    )
}

fn main() {
    // `--obj <dir>` additionally writes one OBJ file per geometry scenario
    // for external viewers.
    let mut args = std::env::args().skip(1);
    let obj_dir = match (args.next().as_deref(), args.next()) {
        (Some("--obj"), Some(dir)) => Some(std::path::PathBuf::from(dir)),
        _ => None,
    };
    for scenario in scenarios() {
        println!("{}", summarize(&scenario));
        if let Some(dir) = &obj_dir {
            let result =
                evaluate(&scenario.recipe, &EvalPolicy::default()).expect("scenario evaluates");
            if let Some(placed) = result.bodies.first() {
                std::fs::create_dir_all(dir).expect("obj dir");
                let mesh = &placed.body.mesh;
                std::fs::write(
                    dir.join(format!("{}.obj", scenario.name)),
                    exedra_testkit::dump::mesh_to_obj(mesh),
                )
                .expect("write obj");
                // Sidecar: one FACE_REGION value per extracted triangle
                // (fan order matches the OBJ), so viewers can shade by
                // provenance.
                let regions = mesh.attrs().dense(exedra::attr::FACE_REGION);
                let mut lines = String::new();
                for face in mesh.faces() {
                    let degree = mesh.face_loop(face).count();
                    let region = regions
                        .and_then(|layer| layer.get(face.as_id()).copied())
                        .unwrap_or(0);
                    for _ in 0..degree.saturating_sub(2) {
                        lines.push_str(&format!("{region}\n"));
                    }
                }
                std::fs::write(dir.join(format!("{}.regions", scenario.name)), lines)
                    .expect("write regions");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::evaluate::{Evaluation, Fidelity, Severity};
    use exedra_constructive::tessellate::Feature;

    fn run(scenario: &Scenario) -> Evaluation {
        evaluate(&scenario.recipe, &EvalPolicy::default()).expect("scenario evaluates")
    }

    #[test]
    fn every_scenario_is_clean_and_deterministic() {
        for scenario in scenarios() {
            let a = run(&scenario);
            let b = run(&scenario);
            assert_eq!(a.report, b.report, "{}: report determinism", scenario.name);
            for placed in &a.bodies {
                let errors = placed.body.mesh.validate_deep();
                assert!(errors.is_empty(), "{}: {errors:?}", scenario.name);
            }
            assert_eq!(summarize(&scenario), summarize(&scenario));
        }
    }

    #[test]
    fn geometry_scenarios_emit_one_exact_body() {
        for scenario in scenarios().iter().take(5) {
            let result = run(scenario);
            assert_eq!(result.bodies.len(), 1, "{}", scenario.name);
            assert!(
                result.report.clean_at(Severity::Warning),
                "{}: {:?}",
                scenario.name,
                result.report.diagnostics
            );
            let node = result.bodies[0].node;
            assert_eq!(
                result.report.fidelity_of(node),
                Some(Fidelity::Exact),
                "{}",
                scenario.name
            );
        }
    }

    #[test]
    fn csg_scenario_produces_a_real_boolean() {
        let scenario = csg_difference();
        let result = run(&scenario);
        assert_eq!(result.bodies.len(), 1, "the difference is a real mesh now");
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        assert!(
            result.report.diagnostics.is_empty(),
            "no fallback diagnostics"
        );
        // Coarse operand attribution is present on every face.
        assert!(
            result.bodies[0]
                .body
                .source_map
                .face_features()
                .iter()
                .all(|f| matches!(f, Feature::BooleanFace { .. }))
        );
    }

    #[test]
    fn source_maps_answer_feature_queries() {
        let result = run(&ring_prism());
        let body = &result.bodies[0].body;
        body.source_map.check(&body.mesh).expect("fresh map");
        // The hole loop produced walls.
        let hole_walls = body
            .source_map
            .face_features()
            .iter()
            .filter(|f| matches!(f, Feature::Wall { loop_index: 1, .. }))
            .count();
        assert!(hole_walls > 0, "hole walls are mapped");
        // Reverse lookups agree with forward lookups.
        for face in body.mesh.faces() {
            let feature = body.source_map.face_feature(face).expect("mapped");
            assert!(
                body.source_map
                    .faces_for(feature)
                    .iter()
                    .any(|&(_, i)| i == face.index())
            );
        }
    }

    #[test]
    fn policy_scenario_reports_policy_and_conflict() {
        let scenario = policy_curve();
        let result = run(&scenario);
        assert_eq!(result.bodies.len(), 1);
        assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
        // The declared spec issue wins the fidelity classification…
        let node = result.bodies[0].node;
        assert!(matches!(
            result.report.fidelity_of(node),
            Some(Fidelity::Conflicted(_))
        ));
        // …while the policy usage is still fully attributed.
        assert_eq!(result.report.policy_curves.len(), 1);
        let (_, policy) = result.report.policy_curves[0];
        assert_eq!(
            scenario.recipe.policy(policy),
            Some("gallery.front-transition@1")
        );
    }

    #[test]
    fn fingerprints_react_to_parameters_only() {
        let a = rect_prism();
        let b = rect_prism();
        assert_eq!(
            a.recipe.recipe_fingerprint(),
            b.recipe.recipe_fingerprint(),
            "identical builds share fingerprints"
        );
        assert_ne!(
            rect_prism().recipe.recipe_fingerprint(),
            l_prism().recipe.recipe_fingerprint(),
            "different shapes differ"
        );
    }
}
