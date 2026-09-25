// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::tests::{assert_clean, mesh_volume};
use super::*;
use crate::builders;
use crate::chart::ChartTransform;
use crate::ir::{FramePolicy, Law, NodeKind, RecipeBuilder, RecipeError, SectionLawError};
use crate::path::PathSegment3;
use alloc::vec;
use exedra_mesh::attr;

fn positions(body: &TessellatedBody) -> Vec<[f64; 3]> {
    body.mesh
        .vertices()
        .map(|v| body.mesh.vertex_position(v).unwrap().map(f64::from))
        .collect()
}

/// Radial distance from the Z axis of every vertex at height `z`.
fn radii_at(body: &TessellatedBody, z: f64) -> Vec<f64> {
    positions(body)
        .into_iter()
        .filter(|p| (p[2] - z).abs() < 1e-5)
        .map(|p| libm::hypot(p[0], p[1]))
        .collect()
}

fn straight_polyline() -> Path3 {
    Path3::Polyline {
        points: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 5.0], [0.0, 0.0, 10.0]],
        frame: FramePolicy::RotationMinimizing,
    }
}

fn straight_curve(closure: PathClosure) -> Path3 {
    Path3::Curves {
        start: [0.0; 3],
        segments: vec![PathSegment3::Line {
            to: [0.0, 0.0, 10.0],
        }],
        section_x: [1.0, 0.0, 0.0],
        section_origin: [0.0; 2],
        closure,
        joins: PathJoin::Smooth,
    }
}

fn mitered_l() -> Path3 {
    Path3::MiteredPolyline {
        points: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 6.0], [6.0, 0.0, 6.0]],
        section_x: [0.0, 1.0, 0.0],
        section_origin: [0.0; 2],
        closure: PathClosure::Open,
        miter_limit: 2.0,
    }
}

fn sweep(profile: &Profile2, path: &Path3, section: &SectionLaw) -> TessellatedBody {
    tessellate_path_sweep(
        profile,
        &Placement3::IDENTITY,
        path,
        section,
        CapMode::Both,
        None,
        &EvalPolicy::default(),
    )
    .unwrap()
}

#[test]
fn taper_scales_sections_by_arc_length() {
    let circle = builders::circle(1.0).unwrap();
    let body = sweep(&circle, &straight_polyline(), &SectionLaw::taper(1.0, 0.25));
    assert_clean(&body);
    for (z, radius) in [(0.0, 1.0), (5.0, 0.625), (10.0, 0.25)] {
        let radii = radii_at(&body, z);
        assert!(!radii.is_empty(), "a ring at z = {z}");
        for r in radii {
            // Cap centroids, if any, sit on the axis.
            assert!(r < 1e-6 || (r - radius).abs() < 1e-5, "z = {z}: {r}");
        }
    }
    // A truncated cone: pi h (R^2 + R r + r^2) / 3, less discretization.
    let cone = core::f64::consts::PI * 10.0 * (1.0 + 0.25 + 0.0625) / 3.0;
    let volume = mesh_volume(&body.mesh);
    assert!((volume - cone).abs() < 0.02 * cone, "{volume} vs {cone}");
}

#[test]
fn identity_law_reproduces_constant_sweeps() {
    let profile = builders::l_profile(0.3, 0.2, 0.1, 0.05).unwrap();
    let policy = EvalPolicy::default();
    let paths = [
        straight_polyline(),
        mitered_l(),
        straight_curve(PathClosure::Open),
    ];
    for path in &paths {
        let shaped = sweep(&profile, path, &SectionLaw::IDENTITY);
        let legacy = match path {
            Path3::Polyline { points, .. } => tessellate_sweep(
                &profile,
                &Placement3::IDENTITY,
                points,
                CapMode::Both,
                &policy,
            ),
            Path3::MiteredPolyline {
                points,
                section_x,
                section_origin,
                closure,
                miter_limit,
            } => tessellate_mitered_sweep(
                &profile,
                &Placement3::IDENTITY,
                points,
                *section_x,
                *section_origin,
                *closure,
                *miter_limit,
                CapMode::Both,
                &policy,
            ),
            Path3::Curves {
                start,
                segments,
                section_x,
                section_origin,
                closure,
                joins,
            } => tessellate_curved_sweep(
                &profile,
                &Placement3::IDENTITY,
                *start,
                segments,
                *section_x,
                *section_origin,
                *closure,
                *joins,
                CapMode::Both,
                &policy,
            ),
        }
        .unwrap();
        assert_eq!(positions(&shaped), positions(&legacy));
        assert_eq!(shaped.mesh.faces().count(), legacy.mesh.faces().count());
    }
}

#[test]
fn twist_turns_sections_about_the_tangent() {
    let rect = builders::rect_centered(2.0, 1.0).unwrap();
    let twist = SectionLaw {
        scale: Law::Constant(1.0),
        twist: Law::Linear(vec![[0.0, 0.0], [1.0, core::f64::consts::FRAC_PI_2]]),
        turns: 0,
    };
    // Laws are evaluated at the path's stations, so a turn needs enough of
    // them: eleven stations give nine degrees per band.
    let path = Path3::MiteredPolyline {
        points: (0..=10).map(|z| [0.0, 0.0, f64::from(z)]).collect(),
        section_x: [1.0, 0.0, 0.0],
        section_origin: [0.0; 2],
        closure: PathClosure::Open,
        miter_limit: 1.0,
    };
    let body = sweep(&rect, &path, &twist);
    assert_clean(&body);
    // Section X = +X and Y = +Z x +X = +Y at the start. A quarter turn maps
    // (x, y) to x * Y - y * X at the end.
    let end: Vec<_> = positions(&body)
        .into_iter()
        .filter(|p| (p[2] - 10.0).abs() < 1e-5)
        .collect();
    for corner in [[1.0, 0.5], [-1.0, 0.5], [-1.0, -0.5], [1.0, -0.5]] {
        let expected = [-corner[1], corner[0], 10.0];
        assert!(
            end.iter()
                .any(|p| (0..3).all(|i| (p[i] - expected[i]).abs() < 1e-5)),
            "{expected:?} in {end:?}"
        );
    }
}

#[test]
fn laws_scale_about_the_section_datum() {
    let rect = builders::rect_from_corner(2.0, 1.0).unwrap();
    let Path3::Curves {
        start,
        segments,
        section_x,
        closure,
        joins,
        ..
    } = straight_curve(PathClosure::Open)
    else {
        unreachable!()
    };
    let path = Path3::Curves {
        start,
        segments,
        section_x,
        section_origin: [1.0, 0.5],
        closure,
        joins,
    };
    let body = sweep(&rect, &path, &SectionLaw::taper(1.0, 0.5));
    let end: Vec<_> = positions(&body)
        .into_iter()
        .filter(|p| (p[2] - 10.0).abs() < 1e-5)
        .collect();
    // The datum stays on the path while the section halves about it.
    for corner in [[0.5, 0.25], [-0.5, 0.25], [-0.5, -0.25], [0.5, -0.25]] {
        assert!(
            end.iter()
                .any(|p| (p[0] - corner[0]).abs() < 1e-6 && (p[1] - corner[1]).abs() < 1e-6),
            "{corner:?} in {end:?}"
        );
    }
}

#[test]
fn mitered_sweeps_taper_through_their_corners() {
    let rect = builders::rect_centered(1.0, 1.0).unwrap();
    let body = sweep(&rect, &mitered_l(), &SectionLaw::taper(1.0, 0.5));
    assert_clean(&body);
    assert!(body.sweep_checks.is_some(), "controlled sweeps are checked");
    // Arc length 12: the corner at 6 carries scale 0.75 on both runs.
    let start = radii_at(&body, 0.0);
    assert!(
        start
            .iter()
            .all(|r| (r - core::f64::consts::SQRT_2 / 2.0).abs() < 1e-6)
    );
}

#[test]
fn closed_sweeps_require_a_seamless_law() {
    let circle = builders::circle(0.3).unwrap();
    let ring = Path3::Curves {
        start: [0.0; 3],
        segments: vec![
            PathSegment3::Line {
                to: [10.0, 0.0, 0.0],
            },
            PathSegment3::Line {
                to: [10.0, 6.0, 0.0],
            },
            PathSegment3::Arc {
                axis_origin: [5.0, 6.0, 0.0],
                axis: [0.0, 0.0, 1.0],
                sweep: core::f64::consts::PI,
            },
            PathSegment3::Line { to: [0.0; 3] },
        ],
        section_x: [0.0, 1.0, 0.0],
        section_origin: [0.0; 2],
        closure: PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, 1.0],
        },
        joins: PathJoin::Miter { limit: 2.0 },
    };
    let run = |section: &SectionLaw| {
        tessellate_path_sweep(
            &circle,
            &Placement3::IDENTITY,
            &ring,
            section,
            CapMode::None,
            None,
            &EvalPolicy::default(),
        )
    };
    let result = run(&SectionLaw::taper(1.0, 0.5));
    assert!(
        matches!(
            result,
            Err(TessellateError::InvalidSectionLaw(
                SectionLawError::OpenSeam
            ))
        ),
        "{:?}",
        result.err()
    );
    let pulse = SectionLaw {
        scale: Law::Linear(vec![[0.0, 1.0], [0.5, 2.0], [1.0, 1.0]]),
        twist: Law::Constant(0.0),
        turns: 0,
    };
    assert_clean(&run(&pulse).unwrap());
}

#[test]
fn closed_sweep_retains_four_complete_turns() {
    let profile = builders::rect_centered(1.0, 0.5).unwrap();
    let points: Vec<_> = (0..128)
        .map(|i| {
            let angle = core::f64::consts::TAU * f64::from(i) / 128.0;
            [10.0 * libm::cos(angle), 10.0 * libm::sin(angle), 0.0]
        })
        .collect();
    let path = Path3::MiteredPolyline {
        points,
        section_x: [0.0, 0.0, 1.0],
        section_origin: [0.0; 2],
        closure: PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, 1.0],
        },
        miter_limit: 1.01,
    };
    let section = SectionLaw::IDENTITY.with_turns(4);
    let body = tessellate_path_sweep(
        &profile,
        &Placement3::IDENTITY,
        &path,
        &section,
        CapMode::None,
        None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    let points = positions(&body);
    let ring_size = points.len() / 128;
    assert!(points[0][2] * points[16 * ring_size][2] < 0.0);
    assert!((points[0][2] - points[32 * ring_size][2]).abs() < 1e-5);

    let curved = Path3::Curves {
        start: [10.0, 0.0, 0.0],
        segments: vec![PathSegment3::Arc {
            axis_origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            sweep: core::f64::consts::TAU,
        }],
        section_x: [0.0, 0.0, 1.0],
        section_origin: [0.0; 2],
        closure: PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, 1.0],
        },
        joins: PathJoin::Smooth,
    };
    let curved_body = tessellate_path_sweep(
        &profile,
        &Placement3::IDENTITY,
        &curved,
        &section,
        CapMode::None,
        None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_clean(&curved_body);
    assert!(curved_body.mesh.boundary_loops().unwrap().is_empty());

    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(profile);
    let root = builder
        .add(NodeKind::Sweep {
            profile,
            path,
            section,
            caps: CapMode::None,
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let text = crate::text::dump_recipe(&recipe);
    assert!(text.contains("turns 4"));
    assert_eq!(
        crate::text::parse_recipe(&text)
            .unwrap()
            .recipe_fingerprint(),
        recipe.recipe_fingerprint()
    );
    #[cfg(feature = "serde")]
    {
        let dto = crate::interchange::to_dto(&recipe);
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"op\":\"turned_sweep\""));
        assert_eq!(
            crate::interchange::from_dto(&dto)
                .unwrap()
                .recipe_fingerprint(),
            recipe.recipe_fingerprint()
        );
    }
}

#[test]
fn complete_turns_refuse_undersampled_paths() {
    let profile = builders::rect_centered(1.0, 0.5).unwrap();
    let path = Path3::MiteredPolyline {
        points: (0..8)
            .map(|i| {
                let angle = core::f64::consts::TAU * f64::from(i) / 8.0;
                [10.0 * libm::cos(angle), 10.0 * libm::sin(angle), 0.0]
            })
            .collect(),
        section_x: [0.0, 0.0, 1.0],
        section_origin: [0.0; 2],
        closure: PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, 1.0],
        },
        miter_limit: 1.1,
    };
    let result = tessellate_path_sweep(
        &profile,
        &Placement3::IDENTITY,
        &path,
        &SectionLaw::IDENTITY.with_turns(4),
        CapMode::None,
        None,
        &EvalPolicy::default(),
    );
    assert!(matches!(
        result,
        Err(TessellateError::SweepTwistUndersampled { band: 0, .. })
    ));
}

#[test]
fn charts_measure_the_unscaled_section() {
    let circle = builders::circle(1.0).unwrap();
    let chart = SurfaceChart::Sweep {
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    };
    let charted = |section: &SectionLaw| {
        tessellate_path_sweep(
            &circle,
            &Placement3::IDENTITY,
            &straight_polyline(),
            section,
            CapMode::None,
            Some(chart),
            &EvalPolicy::default(),
        )
        .unwrap()
    };
    let wall_uvs = |body: &TessellatedBody| {
        let layer = body.mesh.attrs().sparse(attr::CORNER_UV).unwrap();
        let mut uvs: Vec<[u32; 2]> = body
            .mesh
            .faces()
            .flat_map(|f| body.mesh.face_loop(f).collect::<Vec<_>>())
            .map(|corner| layer.get(corner.as_id()).unwrap().map(f32::to_bits))
            .collect();
        uvs.sort_unstable();
        uvs
    };
    assert_eq!(
        wall_uvs(&charted(&SectionLaw::taper(1.0, 0.25))),
        wall_uvs(&charted(&SectionLaw::IDENTITY)),
        "U follows the unscaled perimeter and V the centerline"
    );
}

#[test]
fn recipes_validate_fingerprint_and_round_trip_section_laws() {
    let build = |section: SectionLaw| {
        let mut builder = RecipeBuilder::new();
        let p = builder.add_profile(builders::circle(1.0).unwrap());
        let root = builder.add(NodeKind::Sweep {
            profile: p,
            path: straight_curve(PathClosure::Open),
            section,
            caps: CapMode::Both,
        });
        root.and_then(|root| builder.finish(root))
    };
    assert!(matches!(
        build(SectionLaw::taper(1.0, 0.0)),
        Err(RecipeError::InvalidParameter {
            what: "sweep section law"
        })
    ));
    let constant = build(SectionLaw::IDENTITY).unwrap();
    let tapered = build(SectionLaw::taper(1.0, 0.25)).unwrap();
    let twisted = build(SectionLaw {
        scale: Law::Linear(vec![[0.0, 1.0], [1.0, 0.25]]),
        twist: Law::Constant(0.5),
        turns: 0,
    })
    .unwrap();
    let fingerprints = [&constant, &tapered, &twisted].map(|r| r.recipe_fingerprint());
    assert_ne!(fingerprints[0], fingerprints[1]);
    assert_ne!(fingerprints[1], fingerprints[2]);

    let text = crate::text::dump_recipe(&twisted);
    assert!(text.contains("shaped scale linear 2"));
    assert!(!crate::text::dump_recipe(&constant).contains("shaped"));
    let parsed = crate::text::parse_recipe(&text).unwrap();
    assert_eq!(parsed.recipe_fingerprint(), twisted.recipe_fingerprint());

    #[cfg(feature = "serde")]
    {
        let dto = crate::interchange::to_dto(&twisted);
        let back = crate::interchange::from_dto(&dto).unwrap();
        assert_eq!(back.recipe_fingerprint(), twisted.recipe_fingerprint());
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"op\":\"shaped_sweep\""));
        let plain = serde_json::to_string(&crate::interchange::to_dto(&constant)).unwrap();
        assert!(!plain.contains("shaped_sweep"));
    }
}

#[test]
fn laws_that_fold_a_controlled_wall_are_refused() {
    // A half turn within one band crosses the section through itself.
    let rect = builders::rect_centered(2.0, 1.0).unwrap();
    let path = Path3::MiteredPolyline {
        points: vec![[0.0; 3], [0.0, 0.0, 1.0]],
        section_x: [1.0, 0.0, 0.0],
        section_origin: [0.0; 2],
        closure: PathClosure::Open,
        miter_limit: 1.0,
    };
    let half_turn = SectionLaw {
        scale: Law::Constant(1.0),
        twist: Law::Linear(vec![[0.0, 0.0], [1.0, core::f64::consts::PI]]),
        turns: 0,
    };
    assert!(matches!(
        tessellate_path_sweep(
            &rect,
            &Placement3::IDENTITY,
            &path,
            &half_turn,
            CapMode::Both,
            None,
            &EvalPolicy::default(),
        ),
        Err(TessellateError::CollapsedGeometry | TessellateError::SweepFoldover { .. })
    ));
}

/// Fingerprints of constant-section sweeps computed on the code before
/// section laws existed. Shaped sweeps must not disturb them.
#[test]
fn constant_sweep_fingerprints_are_unchanged() {
    let expected = [
        0xd54f_904c_e91f_26b4_1401_1032_2618_bcd9_u128,
        0xec11_4550_faab_cc41_8baf_63ca_6fb4_0c77,
        0x62c6_e3e4_0a0f_c017_995f_8728_8e51_17e4,
    ];
    let paths = [
        straight_polyline(),
        mitered_l(),
        straight_curve(PathClosure::Open),
    ];
    for (path, expected) in paths.into_iter().zip(expected) {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::l_profile(0.3, 0.2, 0.1, 0.05).unwrap());
        let n = b
            .add(NodeKind::Sweep {
                profile: p,
                path,
                section: SectionLaw::IDENTITY,
                caps: CapMode::Both,
            })
            .unwrap();
        let recipe = b.finish(n).unwrap();
        assert_eq!(recipe.fingerprint(n).unwrap().0, expected);
    }
}

#[test]
fn identity_shaped_laws_are_stored_as_the_identity() {
    let section_of = |section: SectionLaw| {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::circle(1.0).unwrap());
        let n = b
            .add(NodeKind::Sweep {
                profile: p,
                path: straight_polyline(),
                section,
                caps: CapMode::Both,
            })
            .unwrap();
        let recipe = b.finish(n).unwrap();
        let NodeKind::Sweep { section, .. } = &recipe.node(n).unwrap().kind else {
            panic!("a sweep");
        };
        (section.clone(), recipe.fingerprint(n).unwrap())
    };
    let (stored, fingerprint) = section_of(SectionLaw::new(
        Law::Linear(vec![[0.0, 1.0], [1.0, 1.0]]),
        Law::Constant(-0.0),
    ));
    assert_eq!(stored, SectionLaw::IDENTITY);
    assert_eq!(fingerprint, section_of(SectionLaw::IDENTITY).1);

    // A signed-zero key encodes like its positive twin.
    let taper = |first: f64| {
        SectionLaw::new(
            Law::Linear(vec![[first, 1.0], [1.0, 0.5]]),
            Law::Constant(0.0),
        )
    };
    assert_eq!(section_of(taper(-0.0)).1, section_of(taper(0.0)).1);
}
