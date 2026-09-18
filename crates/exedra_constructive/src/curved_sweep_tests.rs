// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::tests::{assert_clean, mesh_volume};
use super::*;
use crate::builders;
use crate::cache::{EvalCache, policy_fingerprint};
use crate::evaluate::{evaluate, evaluate_with_cache};
use crate::ir::{CsgOp, NodeKind, Path3, RecipeBuilder};
use crate::path::{PathDiscretizeError, PathSegment3};
use alloc::vec;

fn profile() -> Profile2 {
    builders::l_profile(0.3, 0.2, 0.1, 0.05).expect("asymmetric section")
}

#[test]
fn boolean_descendants_retain_shared_original_curve_sampling() {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref("arc-rail");
    let p = builder.add_profile(profile());
    let rail = builder
        .with_source(source)
        .add(NodeKind::Sweep {
            profile: p,
            path: Path3::Curves {
                section_origin: [0.0; 2],
                closure: PathClosure::Open,
                joins: PathJoin::Smooth,
                start: [0.0; 3],
                segments: vec![PathSegment3::Arc {
                    axis_origin: [4.0, 0.0, 0.0],
                    axis: [0.0, 1.0, 0.0],
                    sweep: core::f64::consts::FRAC_PI_2,
                }],
                section_x: [1.0, 0.0, 0.0],
            },
            caps: CapMode::Both,
        })
        .unwrap();
    let cut = builder
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [20.0, 20.0, 0.2],
            },
            placement: Placement3::translate(-5.0, -5.0, 1.4),
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![rail, cut],
        })
        .unwrap();
    let result = evaluate(&builder.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert!(
        result.report.clean_at(crate::evaluate::Severity::Error),
        "{:?}",
        result.report.diagnostics
    );
    let body = &result.bodies[0].body;
    assert_clean(body);
    assert!(body.path_sampling.is_none());
    assert!(body.sweep_checks.is_none());
    let mut shared = None;
    let mut walls = 0;
    for face in body.mesh.faces() {
        let origin = body.source_map.surface_origin(face).unwrap();
        if let Feature::SweepWall { band, .. } = origin.feature {
            assert_eq!(origin.source.as_deref(), Some("arc-rail"));
            let path = body
                .source_map
                .sweep_sampling(face)
                .unwrap()
                .path
                .as_ref()
                .unwrap();
            if let Some(previous) = shared {
                assert!(Arc::ptr_eq(previous, path));
            }
            shared = Some(path);
            let span = path.spans[usize::from(band)];
            assert_eq!(span.segment, 0);
            assert!(span.parameter[0] < span.parameter[1]);
            assert!(span.chord_bound <= path.policy.chord_tolerance);
            walls += 1;
        }
    }
    assert!(walls > 0);
}
fn mixed_path() -> Vec<PathSegment3> {
    vec![
        PathSegment3::Line {
            to: [0.0, 0.0, 3.0],
        },
        PathSegment3::Arc {
            axis_origin: [3.0, 0.0, 3.0],
            axis: [0.0, 1.0, 0.0],
            sweep: core::f64::consts::FRAC_PI_2,
        },
        PathSegment3::Cubic {
            control1: [5.0, 0.0, 6.0],
            control2: [6.0, 2.0, 6.0],
            to: [8.0, 2.0, 7.0],
        },
    ]
}
fn positions(body: &TessellatedBody) -> Vec<[f64; 3]> {
    body.mesh
        .vertices()
        .map(|v| {
            body.mesh
                .vertex_position(v)
                .expect("position")
                .map(f64::from)
        })
        .collect()
}
fn curved(
    segments: &[PathSegment3],
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    tessellate_curved_sweep(
        &profile(),
        &Placement3::IDENTITY,
        [0.0; 3],
        segments,
        [1.0, 0.0, 0.0],
        [0.0; 2],
        PathClosure::Open,
        PathJoin::Smooth,
        CapMode::Both,
        policy,
    )
}

#[test]
fn asymmetric_arc_matches_analytic_cross_sections_and_closes_caps() {
    let segments = [PathSegment3::Arc {
        axis_origin: [4.0, 0.0, 0.0],
        axis: [0.0, 1.0, 0.0],
        sweep: core::f64::consts::FRAC_PI_2,
    }];
    let policy = EvalPolicy::default();
    let body = curved(&segments, &policy).expect("curved rail");
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().expect("boundaries").is_empty());
    assert!(mesh_volume(&body.mesh) > 0.0);
    let d = discretize_profile(&profile(), &policy.discretize).expect("section");
    let positions = positions(&body);
    let sampling = body.path_sampling.as_ref().expect("path evidence");
    let parameters = core::iter::once(0.0).chain(sampling.spans.iter().map(|s| s.parameter[1]));
    for (ring, t) in parameters.enumerate() {
        let angle = core::f64::consts::FRAC_PI_2 * t;
        let (s, c) = (libm::sin(angle), libm::cos(angle));
        for (i, p) in d.outer.points.iter().enumerate() {
            // Start is at the origin, tangent +Z. Section X rotates with the
            // radius, section Y stays +Y through the entire planar arc.
            let expected = [4.0 - 4.0 * c + p[0] * c, p[1], 4.0 * s - p[0] * s];
            assert!(
                norm(sub(positions[ring * d.points_len() + i], expected)) < 6e-7,
                "authored landmark on ring {ring}"
            );
        }
    }
    let n = d.points_len();
    for face in body.mesh.faces() {
        let feature = body.source_map.face_feature(face).expect("feature");
        if let Feature::SweepWall { band, .. } = feature {
            assert_eq!(sampling.spans[usize::from(band)].segment, 0);
        }
    }
    assert_eq!(positions.len(), (sampling.spans.len() + 1) * n);
}

#[test]
fn planar_inflection_does_not_flip_the_asymmetric_section() {
    let segments = [PathSegment3::Cubic {
        control1: [0.0, 0.0, 3.0],
        control2: [3.0, 0.0, 3.0],
        to: [3.0, 0.0, 6.0],
    }];
    let body = curved(&segments, &EvalPolicy::default()).expect("S-shaped rail");
    assert_clean(&body);
    let d = discretize_profile(&profile(), &EvalPolicy::default().discretize).expect("section");
    for (ring, points) in positions(&body).chunks_exact(d.points_len()).enumerate() {
        for (p, source) in points.iter().zip(&d.outer.points) {
            assert!(
                (p[1] - source[1]).abs() < 1e-7,
                "section Y must not flip at the inflection, ring {ring}"
            );
        }
    }
}

#[test]
fn spatial_transport_converges_and_sampling_stations_do_not_crease() {
    let segments = mixed_path();
    let mut coarse = EvalPolicy {
        sharp_sin_threshold: 0.001,
        ..Default::default()
    };
    coarse.sweep_path.max_tangent_angle = 0.15;
    let mut fine = coarse;
    fine.sweep_path.max_tangent_angle = 0.025;
    let a = curved(&segments, &coarse).expect("coarse");
    let b = curved(&segments, &fine).expect("fine");
    assert_clean(&a);
    assert_clean(&b);
    let n = a.sweep_checks.expect("checks").section_vertices;
    let pa = positions(&a);
    let pb = positions(&b);
    for (a, b) in pa[pa.len() - n..].iter().zip(&pb[pb.len() - n..]) {
        assert!(
            norm(sub(*a, *b)) < 2e-4,
            "transport must converge under refinement"
        );
    }
    let ids: BTreeMap<_, _> = b.mesh.vertices().enumerate().map(|(i, v)| (v, i)).collect();
    let last = pb.len() / n - 1;
    let mut checked = 0;
    for face in b.mesh.faces() {
        for he in b.mesh.face_loop(face) {
            let from = ids[&b.mesh.from_vertex(he).expect("from")] / n;
            let to = ids[&b.mesh.to_vertex(he).expect("to")] / n;
            if from == to && from > 0 && from < last {
                let edge = b.mesh.canonical_edge(he).expect("edge");
                assert_eq!(
                    b.mesh.edge_sharpness(edge).unwrap_or(0.0),
                    0.0,
                    "sampling is not an authored corner"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
}

#[test]
fn source_correspondence_serialization_and_cache_replay_are_preserved() {
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(profile());
    let root = b
        .add(NodeKind::Sweep {
            profile: p,
            path: Path3::Curves {
                section_origin: [0.0; 2],
                closure: PathClosure::Open,
                joins: PathJoin::Smooth,
                start: [0.0; 3],
                segments: mixed_path(),
                section_x: [1.0, 0.0, 0.0],
            },
            caps: CapMode::Both,
        })
        .expect("node");
    let r = b.finish(root).expect("recipe");
    let text = crate::text::dump_recipe(&r);
    assert!(text.contains("curved_path_sweep"));
    let parsed = crate::text::parse_recipe(&text).expect("text roundtrip");
    assert_eq!(r.recipe_fingerprint(), parsed.recipe_fingerprint());
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&crate::interchange::to_dto(&r)).expect("json");
        let dto = serde_json::from_str(&json).expect("parse json");
        assert_eq!(
            r.recipe_fingerprint(),
            crate::interchange::from_dto(&dto)
                .expect("dto")
                .recipe_fingerprint()
        );
    }
    let policy = EvalPolicy::default();
    let pure = evaluate(&r, &policy).expect("pure");
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&r, &policy, &mut cache).expect("cold");
    let warm = evaluate_with_cache(&r, &policy, &mut cache).expect("warm");
    assert_eq!(warm.report.counters.cache_hits, 1);
    assert_eq!(warm.report.counters.tessellations, 0);
    for result in [&cold, &warm] {
        assert_eq!(
            positions(&pure.bodies[0].body),
            positions(&result.bodies[0].body)
        );
        assert_eq!(
            pure.bodies[0].body.path_sampling,
            result.bodies[0].body.path_sampling
        );
    }
    let body = &warm.bodies[0].body;
    let spans = &body.path_sampling.as_ref().expect("evidence").spans;
    for face in body.mesh.faces() {
        if let Feature::SweepWall { band, .. } =
            body.source_map.face_feature(face).expect("feature")
        {
            assert!(spans[usize::from(band)].segment < 3);
        }
    }
    let mut limited = policy;
    limited.sweep_path.max_path_edges = 2;
    let failure = evaluate_with_cache(&r, &limited, &mut cache)
        .expect_err("must not reuse looser cached body");
    assert_eq!(failure.node, root);
    assert!(matches!(
        failure.error,
        TessellateError::Path(PathDiscretizeError::PathBudgetExceeded { maximum: 2 })
    ));
    for changed in [
        crate::path::PathDiscretizePolicy {
            chord_tolerance: 0.001,
            ..policy.sweep_path
        },
        crate::path::PathDiscretizePolicy {
            max_tangent_angle: 0.02,
            ..policy.sweep_path
        },
        crate::path::PathDiscretizePolicy {
            max_segment_edges: 100,
            ..policy.sweep_path
        },
        crate::path::PathDiscretizePolicy {
            max_path_edges: 100,
            ..policy.sweep_path
        },
    ] {
        assert_ne!(
            policy_fingerprint(&policy),
            policy_fingerprint(&EvalPolicy {
                sweep_path: changed,
                ..policy
            })
        );
    }
}

#[test]
fn tight_curves_and_invalid_orientation_fail_without_a_partial_body() {
    let tight = [PathSegment3::Arc {
        axis_origin: [0.1, 0.0, 0.0],
        axis: [0.0, 1.0, 0.0],
        sweep: core::f64::consts::FRAC_PI_2,
    }];
    assert!(matches!(
        curved(&tight, &EvalPolicy::default()),
        Err(TessellateError::SweepFoldover { .. })
    ));
    let segments = mixed_path();
    assert!(matches!(
        tessellate_curved_sweep(
            &profile(),
            &Placement3::IDENTITY,
            [0.0; 3],
            &segments,
            [0.0, 0.0, 1.0],
            [0.0; 2],
            PathClosure::Open,
            PathJoin::Smooth,
            CapMode::Both,
            &EvalPolicy::default(),
        ),
        Err(TessellateError::InvalidSweepOrientation)
    ));
}

#[test]
fn mirrored_curved_sweep_keeps_cap_winding_and_provenance() {
    let segments = mixed_path();
    let policy = EvalPolicy::default();
    let original = curved(&segments, &policy).expect("original");
    let placement = Placement3 {
        rows: [
            [-1.0, 0.0, 0.0, 12.0],
            [0.0, 1.0, 0.0, 2.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };
    let reflected = tessellate_curved_sweep(
        &profile(),
        &placement,
        [0.0; 3],
        &segments,
        [1.0, 0.0, 0.0],
        [0.0; 2],
        PathClosure::Open,
        PathJoin::Smooth,
        CapMode::Both,
        &policy,
    )
    .expect("reflected");
    assert_clean(&reflected);
    assert!(mesh_volume(&reflected.mesh) > 0.0);
    assert!((mesh_volume(&original.mesh) - mesh_volume(&reflected.mesh)).abs() < 2e-6);
    assert_eq!(
        original.source_map.face_features(),
        reflected.source_map.face_features()
    );
    assert_eq!(original.path_sampling, reflected.path_sampling);
}

#[test]
fn instances_preserve_source_sampling_without_claiming_placed_checks() {
    let policy = EvalPolicy::default();
    let original = curved(&mixed_path(), &policy).expect("source rail");
    assert!(original.sweep_checks.is_some());
    for placement in [
        Placement3::IDENTITY,
        Placement3 {
            rows: [
                [1.0, 0.0, 0.0, 12.0],
                [0.0, 1.0, 0.0, 2.0],
                [0.0, 0.0, 1.0, -3.0],
            ],
        },
        Placement3 {
            rows: [
                [-1.0, 0.0, 0.0, 12.0],
                [0.0, 1.0, 0.0, 2.0],
                [0.0, 0.0, 1.0, -3.0],
            ],
        },
        // Source-local bounds also survive nonuniform scaling: they must
        // not silently become claims about world-space accuracy.
        Placement3 {
            rows: [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, 3.0, 0.0, 0.0],
                [0.0, 0.0, 0.5, 0.0],
            ],
        },
    ] {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(profile());
        let source = builder
            .add(NodeKind::Sweep {
                profile,
                path: Path3::Curves {
                    section_origin: [0.0; 2],
                    closure: PathClosure::Open,
                    joins: PathJoin::Smooth,
                    start: [0.0; 3],
                    segments: mixed_path(),
                    section_x: [1.0, 0.0, 0.0],
                },
                caps: CapMode::Both,
            })
            .expect("source");
        let root = builder
            .add(NodeKind::Instance {
                of: source,
                placement,
            })
            .expect("instance");
        let recipe = builder.finish(root).expect("recipe");
        let mut cache = EvalCache::new();
        for _ in 0..2 {
            let result =
                evaluate_with_cache(&recipe, &policy, &mut cache).expect("instance evaluation");
            let body = &result.bodies[0].body;
            assert_eq!(
                body.path_sampling, original.path_sampling,
                "source correspondence survives placement and cache replay"
            );
            assert_eq!(
                body.sweep_checks, None,
                "source checks do not certify the placed mesh"
            );
            assert_clean(body);
            assert!(body.mesh.boundary_loops().expect("boundaries").is_empty());
            assert!(mesh_volume(&body.mesh) > 0.0);
            assert_eq!(
                body.source_map.face_features(),
                original.source_map.face_features()
            );
            for face in body.mesh.faces() {
                if let Feature::SweepWall { band, .. } =
                    body.source_map.face_feature(face).expect("feature")
                {
                    let spans = &body.path_sampling.as_ref().expect("correspondence").spans;
                    assert!((spans[usize::from(band)].segment as usize) < mixed_path().len());
                }
            }
        }
    }
}

#[test]
fn spatial_transport_matches_an_independent_differential_equation() {
    // Integrate u' = -(u dot t') t directly with RK4. This oracle does not
    // use the production double-reflection construction or its stations.
    fn derivative(p: [[f64; 3]; 4], t: f64) -> ([f64; 3], [f64; 3]) {
        let q = add(
            add(
                scale(sub(p[1], p[0]), 3.0 * (1.0 - t) * (1.0 - t)),
                scale(sub(p[2], p[1]), 6.0 * (1.0 - t) * t),
            ),
            scale(sub(p[3], p[2]), 3.0 * t * t),
        );
        let dq = add(
            scale(add(sub(p[2], scale(p[1], 2.0)), p[0]), 6.0 * (1.0 - t)),
            scale(add(sub(p[3], scale(p[2], 2.0)), p[1]), 6.0 * t),
        );
        let tangent = scale(q, 1.0 / norm(q));
        let change = scale(sub(dq, scale(tangent, dot(tangent, dq))), 1.0 / norm(q));
        (tangent, change)
    }
    fn rhs(p: [[f64; 3]; 4], t: f64, u: [f64; 3]) -> [f64; 3] {
        let (tangent, change) = derivative(p, t);
        scale(tangent, -dot(u, change))
    }
    fn integrate(p: [[f64; 3]; 4], steps: u32) -> [f64; 3] {
        let h = 1.0 / f64::from(steps);
        let mut u = [1.0, 0.0, 0.0];
        for i in 0..steps {
            let t = f64::from(i) * h;
            let a = rhs(p, t, u);
            let b = rhs(p, t + h / 2.0, add(u, scale(a, h / 2.0)));
            let c = rhs(p, t + h / 2.0, add(u, scale(b, h / 2.0)));
            let d = rhs(p, t + h, add(u, scale(c, h)));
            u = add(
                u,
                scale(add(add(a, scale(b, 2.0)), add(scale(c, 2.0), d)), h / 6.0),
            );
        }
        u
    }
    let p = [[0.0; 3], [0.0, 0.0, 2.0], [2.0, 3.0, 4.0], [5.0, -1.0, 6.0]];
    let u = integrate(p, 16384);
    assert!(norm(sub(u, integrate(p, 8192))) < 1e-11, "oracle converged");
    let v = cross(derivative(p, 1.0).0, u);
    let segments = [PathSegment3::Cubic {
        control1: p[1],
        control2: p[2],
        to: p[3],
    }];
    for angle in [0.3, 0.1, 0.03] {
        let mut policy = EvalPolicy::default();
        policy.sweep_path.max_tangent_angle = angle;
        policy.sweep_path.chord_tolerance = 0.1;
        let body = curved(&segments, &policy).expect("spatial rail");
        let section = discretize_profile(&profile(), &policy.discretize).expect("section");
        let points = positions(&body);
        let last = &points[points.len() - section.points_len()..];
        for (actual, point) in last.iter().zip(&section.outer.points) {
            let expected = add(p[3], add(scale(u, point[0]), scale(v, point[1])));
            assert!(
                norm(sub(*actual, expected)) < 6e-7,
                "section landmark agrees with independent transport at f32 precision"
            );
        }
    }
}
