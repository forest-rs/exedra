// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spearhead golden fixtures: the first production users of the
//! `exedra-mesh-golden-v1` format.
//!
//! Each spearhead shape is evaluated from a recipe and pinned three ways:
//! the mesh golden (`goldens/<name>.mesh.golden`), the source-map dump
//! (`goldens/<name>.map.golden`), and — for the CSG fixture — the report
//! rendering (`goldens/csg_prism.report.golden`). Any behavior change in
//! profiles, discretization, tessellation, or reporting shows up as a
//! reviewable diff in these files.
//!
//! To re-bless after a deliberate change (with the matching
//! `EVAL_SCHEMA_VERSION` bump when evaluation output changed):
//!
//! ```text
//! cargo test -p exedra_constructive -- --ignored bless_goldens
//! ```

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::builders;
use crate::evaluate::{Evaluation, evaluate};
use crate::ir::{CapMode, CsgOp, NodeKind, Placement3, Plane3, Recipe, RecipeBuilder};
use crate::tessellate::EvalPolicy;

struct Fixture {
    name: &'static str,
    recipe: Recipe,
}

/// The spearhead fixtures, anonymous and spec-agnostic by design.
fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();

    // 1. A plain rectangular prism.
    {
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
            .expect("valid");
        out.push(Fixture {
            name: "rect_prism",
            recipe: b.finish(n).expect("valid"),
        });
    }

    // 2. A concave L-shaped prism (corner profile).
    {
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
            .expect("valid");
        out.push(Fixture {
            name: "l_prism",
            recipe: b.finish(n).expect("valid"),
        });
    }

    // 3. A rounded-front profile with true arcs.
    {
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
            .expect("valid");
        out.push(Fixture {
            name: "rounded_prism",
            recipe: b.finish(n).expect("valid"),
        });
    }

    // 4. A holed profile (annular ring).
    {
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
            .expect("valid");
        out.push(Fixture {
            name: "ring_prism",
            recipe: b.finish(n).expect("valid"),
        });
    }

    // 5. A partial revolve with caps.
    {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(annulus_square(300.0, 80.0));
        let src = b.source_ref("gallery:quarter_sweep");
        let n = b
            .with_source(src)
            .add(NodeKind::Revolve {
                profile: p,
                placement: Placement3::IDENTITY,
                sweep: core::f64::consts::FRAC_PI_2,
                caps: CapMode::Both,
            })
            .expect("valid");
        out.push(Fixture {
            name: "quarter_sweep",
            recipe: b.finish(n).expect("valid"),
        });
    }

    // 6. A thickened wavy grid shell (integer-derived heights, so the
    // fixture is exactly reproducible from this source).
    {
        let mut b = RecipeBuilder::new();
        let src = b.source_ref("gallery:grid_shell");
        let (rows, cols) = (4_u32, 6_u32);
        let points: Vec<[f64; 3]> = (0..rows)
            .flat_map(|r| {
                (0..cols).map(move |c| {
                    // A saddle-ish integer height field.
                    let h = f64::from((r * c) % 3) * 20.0 - f64::from((r + c) % 2) * 10.0;
                    [f64::from(c) * 100.0, f64::from(r) * 100.0, h]
                })
            })
            .collect();
        let n = b
            .with_source(src)
            .add(NodeKind::GridSurface {
                points,
                rows,
                cols,
                close_u: false,
                close_w: false,
                thickness: Some(18.0),
                placement: Placement3::IDENTITY,
            })
            .expect("valid");
        out.push(Fixture {
            name: "grid_shell",
            recipe: b.finish(n).expect("valid"),
        });
    }

    // 7. A rounded panel stretched through its straight middle band. The
    // golden pins both preserved corner arcs and the inserted profile seams.
    {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rounded_rect(400.0, 300.0, 50.0).expect("rounded"));
        let src = b.source_ref("gallery:stretched_panel");
        let child = b
            .with_source(src)
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 100.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let stretch = b
            .add(NodeKind::Stretch {
                child,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 200.0,
                },
                length: 100.0,
            })
            .expect("valid");
        out.push(Fixture {
            name: "stretched_panel",
            recipe: b.finish(stretch).expect("valid"),
        });
    }

    // 8. A curved, holed planar face. The golden pins single-sided triangle
    // winding, the open outer/hole boundaries, and planar-face provenance.
    {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::ring(2.0, 1.0).expect("ring"));
        let src = b.source_ref("gallery:planar_face");
        let n = b
            .with_source(src)
            .add(NodeKind::PlanarFace {
                profile: p,
                placement: Placement3::IDENTITY,
            })
            .expect("valid");
        out.push(Fixture {
            name: "planar_face",
            recipe: b.finish(n).expect("valid"),
        });
    }

    // 9. A half-disc revolved through a full turn. The small radius keeps the
    // golden compact while pinning shared poles, triangle fans, the skipped
    // axis closure, cardinal extrema, and wall provenance.
    {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(axis_closed_semicircle(0.02));
        let src = b.source_ref("gallery:axis_revolve");
        let n = b
            .with_source(src)
            .add(NodeKind::Revolve {
                profile: p,
                placement: Placement3::IDENTITY,
                sweep: core::f64::consts::TAU,
                caps: CapMode::Both,
            })
            .expect("valid");
        out.push(Fixture {
            name: "axis_revolve",
            recipe: b.finish(n).expect("valid"),
        });
    }

    out
}

fn annulus_square(r0: f64, a: f64) -> crate::profile::Profile2 {
    use crate::profile::{Loop2, Profile2, Seg2};
    let (x0, x1) = (r0 - a / 2.0, r0 + a / 2.0);
    let outer = Loop2::new(vec![
        Seg2::line((x1, 0.0)),
        Seg2::line((x1, a)),
        Seg2::line((x0, a)),
        Seg2::line((x0, 0.0)),
    ])
    .expect("valid square section");
    Profile2::simple(outer).expect("valid profile")
}

fn axis_closed_semicircle(radius: f64) -> crate::profile::Profile2 {
    use crate::profile::{Loop2, Profile2, Seg2};

    let outer = Loop2::new(vec![
        Seg2::arc((0.0, radius), 1.0),
        Seg2::line((0.0, -radius)),
    ])
    .expect("valid semicircle");
    Profile2::simple(outer).expect("valid half-disc profile")
}

/// The CSG fixture: a transformed difference expecting the Unsupported
/// diagnostic (until the boolean pipeline lands).
fn csg_fixture() -> Recipe {
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
            xf: Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_4, 50.0, 0.0, 0.0),
        })
        .expect("valid");
    b.finish(moved).expect("valid")
}

fn render_report(result: &Evaluation) -> String {
    use core::fmt::Write;
    let mut out = String::new();
    let r = &result.report;
    let _ = writeln!(out, "schema {}", r.schema_version);
    let _ = writeln!(out, "bodies {}", r.counters.bodies);
    let _ = writeln!(out, "envelope_only {}", r.counters.envelope_only);
    for (node, fidelity) in &r.fidelity {
        let _ = writeln!(out, "fidelity {} {fidelity:?}", node.0);
    }
    for (node, env) in &r.envelopes {
        let _ = writeln!(
            out,
            "envelope {} min {:016X} {:016X} {:016X} max {:016X} {:016X} {:016X}",
            node.0,
            env.min[0].to_bits(),
            env.min[1].to_bits(),
            env.min[2].to_bits(),
            env.max[0].to_bits(),
            env.max[1].to_bits(),
            env.max[2].to_bits(),
        );
    }
    for d in &r.diagnostics {
        let _ = writeln!(
            out,
            "diag {:?} {} node {:?}",
            d.severity,
            d.code,
            d.node.map(|n| n.0)
        );
    }
    out
}

fn evaluate_single(recipe: &Recipe) -> Evaluation {
    evaluate(recipe, &EvalPolicy::default()).expect("fixture evaluates")
}

fn assert_lf_golden(name: &str, golden: &str) {
    assert!(
        !golden.as_bytes().contains(&b'\r'),
        "{name} must use LF line endings; check .gitattributes"
    );
}

macro_rules! golden_test {
    ($test_name:ident, $fixture_name:literal, $index:expr) => {
        #[test]
        fn $test_name() {
            let fixture = &fixtures()[$index];
            assert_eq!(fixture.name, $fixture_name, "fixture order is stable");
            let result = evaluate_single(&fixture.recipe);
            assert_eq!(result.bodies.len(), 1, "single-body fixture");
            let body = &result.bodies[0].body;
            let errors = body.mesh.validate_deep();
            assert!(errors.is_empty(), "validate_deep: {errors:?}");

            let mesh_golden = include_str!(concat!("../goldens/", $fixture_name, ".mesh.golden"));
            assert_lf_golden(concat!($fixture_name, ".mesh.golden"), mesh_golden);
            exedra_testkit::golden::assert_mesh_golden(&body.mesh, mesh_golden)
                .expect("mesh golden must match; re-bless deliberately");

            let map_golden = include_str!(concat!("../goldens/", $fixture_name, ".map.golden"));
            assert_lf_golden(concat!($fixture_name, ".map.golden"), map_golden);
            assert_eq!(
                body.source_map.dump(),
                map_golden,
                "source-map golden must match; re-bless deliberately"
            );
        }
    };
}

golden_test!(golden_rect_prism, "rect_prism", 0);
golden_test!(golden_l_prism, "l_prism", 1);
golden_test!(golden_rounded_prism, "rounded_prism", 2);
golden_test!(golden_ring_prism, "ring_prism", 3);
golden_test!(golden_quarter_sweep, "quarter_sweep", 4);
golden_test!(golden_stretched_panel, "stretched_panel", 6);
golden_test!(golden_planar_face, "planar_face", 7);
golden_test!(golden_axis_revolve, "axis_revolve", 8);

#[test]
fn golden_csg_report() {
    let recipe = csg_fixture();
    let result = evaluate_single_csg(&recipe);
    let golden = include_str!("../goldens/csg_prism.report.golden");
    assert_lf_golden("csg_prism.report.golden", golden);
    assert_eq!(
        render_report(&result),
        golden,
        "report golden must match; re-bless deliberately"
    );
    // The corner-notch difference is a real boolean now: one exact body
    // whose volume matches the closed form (rigid transforms preserve it).
    assert_eq!(result.bodies.len(), 1);
    let mesh = &result.bodies[0].body.mesh;
    assert!(mesh.validate_deep().is_empty());
    let mut volume = 0.0_f64;
    for face in mesh.faces() {
        let verts: Vec<[f64; 3]> = mesh
            .face_loop(face)
            .filter_map(|he| mesh.to_vertex(he))
            .filter_map(|v| mesh.vertex_position(v))
            .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
            .collect();
        for i in 1..verts.len().saturating_sub(1) {
            let (a, b, c) = (verts[0], verts[i], verts[i + 1]);
            volume += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
        }
    }
    volume /= 6.0;
    let expected = 200.0 * 100.0 * 80.0 - 40.0 * 40.0 * 80.0;
    assert!(
        (volume - expected).abs() / expected < 1e-6,
        "notched volume {volume} vs {expected}"
    );
}

fn evaluate_single_csg(recipe: &Recipe) -> Evaluation {
    evaluate(recipe, &EvalPolicy::default()).expect("csg fixture evaluates")
}

#[test]
fn goldens_are_signature_stable() {
    // Trimesh signatures across two independent evaluations of every
    // fixture: the wind-tunnel style determinism oracle.
    for fixture in fixtures() {
        let a = evaluate_single(&fixture.recipe);
        let b = evaluate_single(&fixture.recipe);
        let sig = |e: &Evaluation| {
            let (tri, _) = e.bodies[0]
                .body
                .mesh
                .to_trimesh(&exedra::ExtractParams::default());
            exedra_testkit::golden::trimesh_signature(&tri)
        };
        assert_eq!(
            sig(&a),
            sig(&b),
            "{}: signature must be stable",
            fixture.name
        );
    }
}

/// Regenerates every golden file. Run deliberately:
/// `cargo test -p exedra_constructive -- --ignored bless_goldens`
#[test]
#[ignore = "regenerates golden files; run deliberately after reviewed changes"]
fn bless_goldens() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("goldens");
    std::fs::create_dir_all(&dir).expect("goldens dir");
    for fixture in fixtures() {
        let result = evaluate_single(&fixture.recipe);
        let body = &result.bodies[0].body;
        std::fs::write(
            dir.join(format!("{}.mesh.golden", fixture.name)),
            exedra_testkit::golden::dump_golden(&body.mesh),
        )
        .expect("write mesh golden");
        std::fs::write(
            dir.join(format!("{}.map.golden", fixture.name)),
            body.source_map.dump(),
        )
        .expect("write map golden");
    }
    let csg = evaluate_single_csg(&csg_fixture());
    std::fs::write(dir.join("csg_prism.report.golden"), render_report(&csg))
        .expect("write report golden");
}
