// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reference scenarios exercising the public constructive surface as an
//! external geometry frontend would.

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

/// Builds all spearhead scenarios.
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
        grid_shell(),
    ]
}

/// A doubly-curved shell: a 5 × 7 control grid with polynomial dome
/// heights (no trig — deterministic across platforms) thickened into a
/// watertight solid through the `GridSurface` node.
fn grid_shell() -> Scenario {
    let mut b = RecipeBuilder::new();
    let (rows, cols) = (5_u32, 7_u32);
    let points: Vec<[f64; 3]> = (0..rows)
        .flat_map(|r| {
            (0..cols).map(move |c| {
                let (rf, cf) = (f64::from(r), f64::from(c));
                // Parabolic dome: peaks mid-grid, zero at the rim.
                let bump = rf * (4.0 - rf) * cf * (6.0 - cf);
                [cf * 50.0, rf * 50.0, bump * 1.6]
            })
        })
        .collect();
    let src = b.source_ref("gallery:grid_shell");
    let n = b
        .with_source(src)
        .add(NodeKind::GridSurface {
            points,
            rows,
            cols,
            close_u: false,
            close_w: false,
            thickness: Some(12.0),
            placement: Placement3::IDENTITY,
        })
        .expect("valid grid");
    Scenario {
        name: "grid_shell",
        recipe: b.finish(n).expect("valid recipe"),
    }
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
    let drill = b.add_profile(builders::circle(30.0).expect("circle"));
    let e1 = b
        .add(NodeKind::Extrude {
            profile: block,
            placement: Placement3::IDENTITY,
            height: 80.0,
            caps: CapMode::Both,
        })
        .expect("valid");
    // A cylinder drilled clean through the slab: the cut loops are fully
    // interior to the caps, exercising interior-loop face splitting.
    let e2 = b
        .add(NodeKind::Extrude {
            profile: drill,
            placement: Placement3::translate(130.0, 50.0, -20.0),
            height: 120.0,
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
                export_body(dir, scenario.name, &placed.body.mesh);
                // Bonus body: the drilled block with its sharp edges
                // filleted by the kernel rounding pass, strips in their
                // own region.
                if scenario.name == "csg_difference" {
                    let (rounded, stats) = rounded_drill_mesh();
                    assert!(stats.strip_faces > 0, "rounding produced strips");
                    export_body(dir, "csg_rounded", &rounded);
                }
            }
        }
    }
}

/// The rounded drill body: the drilled slab with both seam rims filleted
/// by the kernel rounding pass (strips in region 9). Rounding constructive
/// drill output is blocked on exe-8kli; this card rounds direct boolean
/// output, which is the pass's proven envelope.
fn rounded_drill_mesh() -> (exedra::Mesh, exedra::round::RoundStats) {
    let mut mesh = drilled_slab_mesh();
    let stats = exedra::round::round_sharp_edges(&mut mesh, &rounding_policy()).expect("rounds");
    (mesh, stats)
}

/// A drilled slab built directly from meshes (the rounding pass's proven
/// boolean fixture shape): slab minus a 16-gon prism through both caps.
fn drilled_slab_mesh() -> exedra::Mesh {
    use exedra::MeshBuilder;
    let mut b = MeshBuilder::new();
    // Slab 4 x 4 x 1.
    let corners: [[f32; 3]; 8] = [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [4.0, 4.0, 0.0],
        [0.0, 4.0, 0.0],
        [0.0, 0.0, 1.0],
        [4.0, 0.0, 1.0],
        [4.0, 4.0, 1.0],
        [0.0, 4.0, 1.0],
    ];
    for c in corners {
        b.push_vertex(c);
    }
    for f in [
        [3_u32, 2, 1, 0],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ] {
        b.add_face(&f).expect("slab face");
    }
    let slab = b.build().expect("valid slab").mesh;

    let n = 16_u32;
    let mut b = MeshBuilder::new();
    for z in [-1.0_f64, 2.0] {
        for i in 0..n {
            let angle = core::f64::consts::TAU * f64::from(i) / f64::from(n);
            let p = [2.0 + 0.8 * angle.cos(), 2.0 + 0.8 * angle.sin(), z];
            #[expect(clippy::cast_possible_truncation, reason = "gallery narrowing")]
            b.push_vertex([p[0] as f32, p[1] as f32, p[2] as f32]);
        }
    }
    let bottom: Vec<u32> = (0..n).rev().collect();
    b.add_face(&bottom).expect("bottom cap");
    let top: Vec<u32> = (n..2 * n).collect();
    b.add_face(&top).expect("top cap");
    for i in 0..n {
        let j = (i + 1) % n;
        b.add_face(&[i, j, n + j, n + i]).expect("side wall");
    }
    let prism = b.build().expect("valid prism").mesh;

    let mut scratch = exedra::boolean::BooleanScratch::new();
    let mut diagnostics = exedra::boolean::BooleanDiagnostics::default();
    exedra::boolean::boolean_mesh(
        &slab,
        &prism,
        exedra::boolean::BooleanOp::Difference,
        exedra::FaceTriangulation::Fan,
        &mut scratch,
        &mut diagnostics,
    )
    .expect("drill boolean succeeds")
    .mesh
}

/// The rounding policy for the gallery's filleted drill card.
fn rounding_policy() -> exedra::round::RoundPolicy {
    let mut policy = exedra::round::RoundPolicy::fillet(0.3);
    policy.region = Some(9);
    policy
}

/// Writes one OBJ plus its per-triangle `FACE_REGION` sidecar (fan order
/// matches the OBJ), so viewers can shade by provenance.
fn export_body(dir: &std::path::Path, name: &str, mesh: &exedra::Mesh) {
    std::fs::write(
        dir.join(format!("{name}.obj")),
        exedra_testkit::dump::mesh_to_obj(mesh),
    )
    .expect("write obj");
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
    std::fs::write(dir.join(format!("{name}.regions")), lines).expect("write regions");
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::evaluate::{Evaluation, Fidelity, Severity};
    use exedra_constructive::tessellate::Feature;

    fn run(scenario: &Scenario) -> Evaluation {
        evaluate(&scenario.recipe, &EvalPolicy::default()).expect("scenario evaluates")
    }

    /// The rounded drill card: the boolean output filleted by the kernel
    /// rounding pass stays watertight and deterministic.
    #[test]
    fn rounded_drill_is_clean_and_deterministic() {
        let (a, stats_a) = rounded_drill_mesh();
        let (b, stats_b) = rounded_drill_mesh();
        assert_eq!(stats_a, stats_b, "rounding determinism");
        assert!(stats_a.strip_faces > 0);
        let errors = a.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        let (tri_a, _) = a.to_trimesh(&ExtractParams::default());
        let (tri_b, _) = b.to_trimesh(&ExtractParams::default());
        assert_eq!(trimesh_signature(&tri_a), trimesh_signature(&tri_b));
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

    /// Signed volume via the divergence theorem, fanning each face loop
    /// (boolean output faces are triangles or convex).
    fn mesh_volume(mesh: &exedra::Mesh) -> f64 {
        let mut vol = 0.0;
        for face in mesh.faces() {
            let verts: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect();
            for i in 1..verts.len().saturating_sub(1) {
                let (a, b, c) = (verts[0], verts[i], verts[i + 1]);
                vol += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        vol / 6.0
    }

    /// Euler characteristic V - E + F: 2 for a sphere-like shell, 0 for a
    /// single through-hole shell.
    fn euler_characteristic(mesh: &exedra::Mesh) -> i64 {
        let vertices = i64::try_from(mesh.vertices().count()).expect("small");
        let faces = i64::try_from(mesh.faces().count()).expect("small");
        let half_edges: usize = mesh.faces().map(|face| mesh.face_loop(face).count()).sum();
        let edges = i64::try_from(half_edges).expect("small") / 2;
        vertices - edges + faces
    }

    #[test]
    fn csg_scenario_produces_a_real_boolean() {
        let scenario = csg_difference();
        let result = run(&scenario);
        assert_eq!(
            result.bodies.len(),
            1,
            "the difference is a real mesh now: {:?}",
            result.report.diagnostics
        );
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        // A genuine through-hole: genus-1 shell, volume = slab minus the
        // (discretized) cylinder within a fraction of a percent of exact.
        assert_eq!(euler_characteristic(mesh), 0, "drilled shell has genus 1");
        let expected = 200.0 * 100.0 * 80.0 - core::f64::consts::PI * 30.0 * 30.0 * 80.0;
        let volume = mesh_volume(mesh);
        assert!(
            (volume - expected).abs() < 1_000.0,
            "volume {volume} vs analytic {expected}"
        );
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
