// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::tests::{assert_clean, mesh_volume};
use super::*;
use crate::builders;
use crate::cache::EvalCache;
use crate::evaluate::{Severity, evaluate_with_cache};
use crate::ir::{CsgOp, NodeKind, Path3, Plane3, Recipe, RecipeBuilder};
use crate::path::{PathDiscretizeError, PathSegment3};
use crate::section::{SectionPolicy, section_body};
use alloc::vec;

fn closure() -> PathClosure {
    PathClosure::ClosedPlanar {
        normal: [0.0, 0.0, 1.0],
    }
}
fn profile() -> Profile2 {
    builders::l_profile(0.8, 0.6, 0.6, 0.45).unwrap()
}
fn arch() -> Vec<PathSegment3> {
    vec![
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
    ]
}
fn arch_body(
    section: &Profile2,
    datum: [f64; 2],
    policy: &EvalPolicy,
) -> Result<TessellatedBody, TessellateError> {
    tessellate_curved_sweep(
        section,
        &Placement3::IDENTITY,
        [0.0; 3],
        &arch(),
        [0.0, 1.0, 0.0],
        datum,
        closure(),
        PathJoin::Miter { limit: 2.0 },
        CapMode::None,
        policy,
    )
}
fn positions(body: &TessellatedBody) -> Vec<[f64; 3]> {
    body.mesh
        .vertices()
        .map(|v| body.mesh.vertex_position(v).unwrap().map(f64::from))
        .collect()
}
fn same_geometry(a: &TessellatedBody, b: &TessellatedBody) {
    let (a, b) = (positions(a), positions(b));
    assert_eq!(a.len(), b.len());
    for point in a {
        assert!(
            b.iter().any(|p| norm(sub(point, *p)) < 2e-6),
            "missing {point:?}"
        );
    }
}
#[test]
fn arched_asymmetric_surround_has_correct_volume_opening_and_shared_seam() {
    let section = profile();
    let body = arch_body(&section, [0.0; 2], &EvalPolicy::default()).unwrap();
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    let sampling = body.path_sampling.as_ref().unwrap();
    assert_eq!(sampling.corners.len(), 2);
    assert_eq!(sampling.corners[0].station, 0);
    assert_eq!(sampling.corners[0].segment, 0);
    assert_eq!(sampling.corners[1].segment, 1);
    let section_count = discretize_profile(&section, &EvalPolicy::default().discretize)
        .unwrap()
        .points_len();
    assert_eq!(
        body.mesh.vertices().count(),
        section_count * sampling.spans.len()
    );
    assert!(
        body.source_map
            .face_features()
            .iter()
            .all(|f| matches!(f, Feature::SweepWall { .. }))
    );
    let n = u32::try_from(sampling.spans.iter().filter(|s| s.segment == 2).count()).unwrap();
    let angular = f64::from(n) * libm::sin(core::f64::consts::PI / f64::from(n));
    let area = 0.8 * 0.15 + 0.2 * 0.45;
    let moment = (0.8 * 0.8 * 0.15 + 0.2 * 0.2 * 0.45) / 2.0;
    let expected = (22.0 + 5.0 * angular) * area - (4.0 + angular) * moment;
    assert!((mesh_volume(&body.mesh) - expected).abs() < 2e-5);
    let cut = section_body(
        &body,
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 0.075,
        },
        &SectionPolicy::default(),
    )
    .unwrap();
    assert_eq!(cut.regions.len(), 1);
    assert_eq!(cut.regions[0].holes.len(), 1);
}
#[test]
fn smooth_circle_closes_without_caps_or_an_artificial_crease() {
    let body = tessellate_curved_sweep(
        &profile(),
        &Placement3::IDENTITY,
        [5.0, 0.0, 0.0],
        &[PathSegment3::Arc {
            axis_origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            sweep: core::f64::consts::TAU,
        }],
        [-1.0, 0.0, 0.0],
        [0.0; 2],
        closure(),
        PathJoin::Smooth,
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    let sampling = body.path_sampling.as_ref().unwrap();
    assert!(sampling.corners.is_empty());
    let n = f64::from(u32::try_from(sampling.spans.len()).unwrap());
    let area = 0.8 * 0.15 + 0.2 * 0.45;
    let moment = (0.8 * 0.8 * 0.15 + 0.2 * 0.2 * 0.45) / 2.0;
    assert!(
        (mesh_volume(&body.mesh)
            - n * libm::sin(core::f64::consts::TAU / n) * (5.0 * area - moment))
            .abs()
            < 2e-5
    );
    let vertices: Vec<_> = body.mesh.vertices().take(6).collect();
    let mut edges = 0;
    for face in body.mesh.faces() {
        for edge in body.mesh.face_loop(face) {
            if vertices.contains(&body.mesh.from_vertex(edge).unwrap())
                && vertices.contains(&body.mesh.to_vertex(edge).unwrap())
            {
                assert_eq!(
                    body.mesh
                        .edge_sharpness(body.mesh.canonical_edge(edge).unwrap())
                        .unwrap_or(0.0),
                    0.0
                );
                edges += 1;
            }
        }
    }
    assert_eq!(edges, 12);
}
#[test]
fn relocating_arch_seam_and_reversing_plane_normal_preserve_authored_roll() {
    let original = arch_body(&profile(), [0.15, 0.1], &EvalPolicy::default()).unwrap();
    let mut segments = arch();
    segments.rotate_left(1);
    let moved = tessellate_curved_sweep(
        &profile(),
        &Placement3::IDENTITY,
        [10.0, 0.0, 0.0],
        &segments,
        [-1.0, 0.0, 0.0],
        [0.15, 0.1],
        PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, -7.0],
        },
        PathJoin::Miter { limit: 2.0 },
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    same_geometry(&original, &moved);
    assert_clean(&moved);
}
#[test]
fn closure_planarity_caps_continuity_and_budgets_are_explicit() {
    let section = profile();
    let run = |segments: &[PathSegment3], joins, caps, policy: &EvalPolicy| {
        tessellate_curved_sweep(
            &section,
            &Placement3::IDENTITY,
            [0.0; 3],
            segments,
            [0.0, 1.0, 0.0],
            [0.0; 2],
            closure(),
            joins,
            caps,
            policy,
        )
    };
    let policy = EvalPolicy::default();
    assert!(matches!(
        run(&arch(), PathJoin::Smooth, CapMode::None, &policy),
        Err(TessellateError::Path(
            PathDiscretizeError::DiscontinuousTangent { segment: 1 }
        ))
    ));
    assert!(matches!(
        run(
            &arch(),
            PathJoin::Miter { limit: 2.0 },
            CapMode::Both,
            &policy
        ),
        Err(TessellateError::ClosedSweepCaps)
    ));
    let mut unclosed = arch();
    *unclosed.last_mut().unwrap() = PathSegment3::Line {
        to: [1e-14, 0.0, 0.0],
    };
    assert!(matches!(
        run(
            &unclosed,
            PathJoin::Miter { limit: 2.0 },
            CapMode::None,
            &policy
        ),
        Err(TessellateError::Path(PathDiscretizeError::NotClosed { .. }))
    ));
    let mut nonplanar = arch();
    nonplanar[0] = PathSegment3::Cubic {
        control1: [3.0, 0.0, 0.2],
        control2: [7.0, 0.0, 0.0],
        to: [10.0, 0.0, 0.0],
    };
    assert!(matches!(
        run(
            &nonplanar,
            PathJoin::Miter { limit: 2.0 },
            CapMode::None,
            &policy
        ),
        Err(TessellateError::Path(PathDiscretizeError::NonPlanar {
            segment: 0
        }))
    ));
    assert!(matches!(
        run(
            &arch(),
            PathJoin::Miter { limit: 1.4 },
            CapMode::None,
            &policy
        ),
        Err(TessellateError::MiterLimitExceeded { .. })
    ));
    let mut limited = policy;
    limited.sweep_path.max_path_edges = 4;
    assert!(matches!(
        run(
            &arch(),
            PathJoin::Miter { limit: 2.0 },
            CapMode::None,
            &limited
        ),
        Err(TessellateError::Path(
            PathDiscretizeError::PathBudgetExceeded { .. }
        ))
    ));
    limited = policy;
    limited.max_sweep_vertices = 1;
    assert!(matches!(
        run(
            &arch(),
            PathJoin::Miter { limit: 2.0 },
            CapMode::None,
            &limited
        ),
        Err(TessellateError::SweepVertexBudgetExceeded { .. })
    ));
    assert!(matches!(
        arch_body(
            &builders::rect_from_corner(6.0, 0.4).unwrap(),
            [0.0; 2],
            &policy
        ),
        Err(TessellateError::SweepFoldover { .. })
    ));
}
fn recipe(cut: bool) -> Recipe {
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(profile());
    let source = b.source_ref("arched-surround");
    let mut root = b
        .with_source(source)
        .add(NodeKind::Sweep {
            profile: p,
            path: Path3::Curves {
                start: [0.0; 3],
                segments: arch(),
                section_x: [0.0, 1.0, 0.0],
                section_origin: [0.0; 2],
                closure: closure(),
                joins: PathJoin::Miter { limit: 2.0 },
            },
            caps: CapMode::None,
        })
        .unwrap();
    if cut {
        let cutter = b
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [2.0, 2.0, 2.0],
                },
                placement: Placement3::translate(4.0, -0.5, -0.5),
            })
            .unwrap();
        root = b
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![root, cutter],
            })
            .unwrap();
    }
    b.finish(root).unwrap()
}
#[test]
fn retained_curved_closure_roundtrips_caches_and_preserves_sampling_through_cuts() {
    for cut in [false, true] {
        let recipe = recipe(cut);
        let text = crate::text::dump_recipe(&recipe);
        assert!(text.contains("curved_path_sweep"));
        assert_eq!(
            recipe.recipe_fingerprint(),
            crate::text::parse_recipe(&text)
                .unwrap()
                .recipe_fingerprint()
        );
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&crate::interchange::to_dto(&recipe)).unwrap();
            assert!(json.contains("curved_path_sweep"));
            assert_eq!(
                recipe.recipe_fingerprint(),
                crate::interchange::from_dto(&serde_json::from_str(&json).unwrap())
                    .unwrap()
                    .recipe_fingerprint()
            );
        }
        let mut cache = EvalCache::new();
        for warm in [false, true] {
            let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
            assert!(
                result.report.clean_at(Severity::Error),
                "{:?}",
                result.report.diagnostics
            );
            if warm {
                assert_eq!(result.report.counters.tessellations, 0);
            }
            assert_eq!(result.bodies.len(), 1);
            let body = &result.bodies[0].body;
            assert_clean(body);
            assert!(body.mesh.boundary_loops().unwrap().is_empty());
            let mut walls = 0;
            for face in body.mesh.faces() {
                let origin = body.source_map.surface_origin(face).unwrap();
                if let Feature::SweepWall { band, .. } = origin.feature {
                    let sampling = body
                        .source_map
                        .sweep_sampling(face)
                        .unwrap()
                        .path
                        .as_ref()
                        .unwrap();
                    assert_eq!(sampling.closure, closure());
                    assert_eq!(sampling.corners.len(), 2);
                    assert!(sampling.spans[usize::from(band)].segment < 4);
                    walls += 1;
                }
            }
            assert!(walls > 0);
            if cut {
                assert!(body.sweep_checks.is_none());
                assert!(body.path_sampling.is_none());
            }
        }
    }
}

#[test]
fn closed_cubics_carry_a_holed_section_with_consistent_orientation() {
    let k = 5.0 * 0.5522847498307936;
    let segments = [
        PathSegment3::Cubic {
            control1: [5.0, k, 0.0],
            control2: [k, 5.0, 0.0],
            to: [0.0, 5.0, 0.0],
        },
        PathSegment3::Cubic {
            control1: [-k, 5.0, 0.0],
            control2: [-5.0, k, 0.0],
            to: [-5.0, 0.0, 0.0],
        },
        PathSegment3::Cubic {
            control1: [-5.0, -k, 0.0],
            control2: [-k, -5.0, 0.0],
            to: [0.0, -5.0, 0.0],
        },
        PathSegment3::Cubic {
            control1: [k, -5.0, 0.0],
            control2: [5.0, -k, 0.0],
            to: [5.0, 0.0, 0.0],
        },
    ];
    let section = builders::ring(0.3, 0.15).unwrap();
    let body = tessellate_curved_sweep(
        &section,
        &Placement3::IDENTITY,
        [5.0, 0.0, 0.0],
        &segments,
        [-1.0, 0.0, 0.0],
        [0.0; 2],
        closure(),
        PathJoin::Smooth,
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    assert!(mesh_volume(&body.mesh) > 0.0);
    let evidence = body.path_sampling.as_ref().unwrap();
    assert!(evidence.corners.is_empty());
    for segment in 0..4 {
        let spans: Vec<_> = evidence
            .spans
            .iter()
            .filter(|s| s.segment == segment)
            .collect();
        assert_eq!(spans[0].parameter[0], 0.0);
        assert_eq!(spans.last().unwrap().parameter[1], 1.0);
        assert!(
            spans
                .iter()
                .all(|s| s.chord_bound <= evidence.policy.chord_tolerance)
        );
    }
}

#[test]
fn rescaling_the_arch_keeps_physical_section_dimensions() {
    let section = profile();
    let original = arch_body(&section, [0.0; 2], &EvalPolicy::default()).unwrap();
    let doubled: Vec<_> = arch()
        .into_iter()
        .map(|s| match s {
            PathSegment3::Line { to } => PathSegment3::Line {
                to: to.map(|v| v * 2.0),
            },
            PathSegment3::Arc {
                axis_origin,
                axis,
                sweep,
            } => PathSegment3::Arc {
                axis_origin: axis_origin.map(|v| v * 2.0),
                axis,
                sweep,
            },
            _ => unreachable!(),
        })
        .collect();
    let scaled = tessellate_curved_sweep(
        &section,
        &Placement3::IDENTITY,
        [0.0; 3],
        &doubled,
        [0.0, 1.0, 0.0],
        [0.0; 2],
        closure(),
        PathJoin::Miter { limit: 2.0 },
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    let original_points = positions(&original);
    let scaled_points = positions(&scaled);
    let n = discretize_profile(&section, &EvalPolicy::default().discretize)
        .unwrap()
        .points_len();
    assert_eq!(original_points.len(), scaled_points.len());
    for (band, span) in original
        .path_sampling
        .as_ref()
        .unwrap()
        .spans
        .iter()
        .enumerate()
    {
        for i in 0..n {
            let a = original_points[band * n + i];
            let b = scaled_points[band * n + i];
            assert_eq!(a[2], b[2]);
            if span.segment == 2 {
                let original_inset = 5.0 - libm::hypot(a[0] - 5.0, a[1] - 6.0);
                let scaled_inset = 10.0 - libm::hypot(b[0] - 10.0, b[1] - 12.0);
                assert!((original_inset - scaled_inset).abs() < 2e-6);
            }
        }
    }
}

#[test]
fn reversed_traversal_uses_an_authored_reflected_section() {
    use crate::profile::{Loop2, Seg2};
    let section = profile();
    let reflected = Profile2::simple(
        Loop2::new(
            section
                .outer()
                .segs()
                .iter()
                .rev()
                .map(|s| Seg2::line((s.to.x, -s.to.y)))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let original = arch_body(&section, [0.0; 2], &EvalPolicy::default()).unwrap();
    let reversed = [
        PathSegment3::Line {
            to: [0.0, 6.0, 0.0],
        },
        PathSegment3::Arc {
            axis_origin: [5.0, 6.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            sweep: -core::f64::consts::PI,
        },
        PathSegment3::Line {
            to: [10.0, 0.0, 0.0],
        },
        PathSegment3::Line { to: [0.0; 3] },
    ];
    let reversed = tessellate_curved_sweep(
        &reflected,
        &Placement3::IDENTITY,
        [0.0; 3],
        &reversed,
        [1.0, 0.0, 0.0],
        [0.0; 2],
        closure(),
        PathJoin::Miter { limit: 2.0 },
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_clean(&reversed);
    same_geometry(&original, &reversed);
    assert!((mesh_volume(&original.mesh) - mesh_volume(&reversed.mesh)).abs() < 2e-5);
}

#[test]
fn a_concave_mixed_perimeter_keeps_its_authored_corners() {
    let mut segments = vec![
        PathSegment3::Line {
            to: [4.0, 0.0, 0.0],
        },
        PathSegment3::Line {
            to: [4.0, 1.0, 0.0],
        },
        PathSegment3::Line {
            to: [6.0, 1.0, 0.0],
        },
        PathSegment3::Line {
            to: [6.0, 0.0, 0.0],
        },
    ];
    segments.extend(arch());
    let body = tessellate_curved_sweep(
        &builders::rect_from_corner(0.3, 0.4).unwrap(),
        &Placement3::IDENTITY,
        [0.0; 3],
        &segments,
        [0.0, 1.0, 0.0],
        [0.0; 2],
        closure(),
        PathJoin::Miter { limit: 2.0 },
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    assert!(mesh_volume(&body.mesh) > 0.0);
    assert_eq!(body.path_sampling.as_ref().unwrap().corners.len(), 6);
}

#[test]
fn open_spatial_line_corners_match_the_polyline_construction() {
    let points = [
        [0.0, 0.0, 0.0],
        [12.0, 0.0, 3.0],
        [13.0, 12.0, 5.0],
        [2.0, 15.0, 14.0],
        [0.0, 25.0, 12.0],
    ];
    let segments: Vec<_> = points[1..]
        .iter()
        .map(|&to| PathSegment3::Line { to })
        .collect();
    let section = profile();
    let policy = EvalPolicy::default();
    let polyline = tessellate_mitered_sweep(
        &section,
        &Placement3::IDENTITY,
        &points,
        [0.0, 1.0, 1.0],
        [0.15, 0.1],
        PathClosure::Open,
        8.0,
        CapMode::Both,
        &policy,
    )
    .unwrap();
    let curved = tessellate_curved_sweep(
        &section,
        &Placement3::IDENTITY,
        points[0],
        &segments,
        [0.0, 1.0, 1.0],
        [0.15, 0.1],
        PathClosure::Open,
        PathJoin::Miter { limit: 8.0 },
        CapMode::Both,
        &policy,
    )
    .unwrap();
    assert_clean(&curved);
    same_geometry(&polyline, &curved);
}

#[test]
fn tilted_closed_plane_matches_a_placed_circle() {
    let placement =
        Placement3::euler_extrinsic_xyz_then_translate(0.47, -0.81, 0.32, [2.0, 3.0, 4.0]);
    let vector = |p: [f64; 3]| {
        core::array::from_fn(|i| {
            placement.rows[i][0] * p[0] + placement.rows[i][1] * p[1] + placement.rows[i][2] * p[2]
        })
    };
    let point = |p| apply_placement(&placement, p);
    let section = profile();
    let policy = EvalPolicy::default();
    let placed = tessellate_curved_sweep(
        &section,
        &placement,
        [5.0, 0.0, 0.0],
        &[PathSegment3::Arc {
            axis_origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            sweep: core::f64::consts::TAU,
        }],
        [-1.0, 0.0, 0.2],
        [0.15, 0.1],
        closure(),
        PathJoin::Smooth,
        CapMode::None,
        &policy,
    )
    .unwrap();
    let tilted = tessellate_curved_sweep(
        &section,
        &Placement3::IDENTITY,
        point([5.0, 0.0, 0.0]),
        &[PathSegment3::Arc {
            axis_origin: point([0.0; 3]),
            axis: vector([0.0, 0.0, 1.0]),
            sweep: core::f64::consts::TAU,
        }],
        vector([-1.0, 0.0, 0.2]),
        [0.15, 0.1],
        PathClosure::ClosedPlanar {
            normal: vector([0.0, 0.0, 1.0]),
        },
        PathJoin::Smooth,
        CapMode::None,
        &policy,
    )
    .unwrap();
    assert_clean(&tilted);
    same_geometry(&placed, &tilted);
}

#[test]
fn legacy_curve_opcode_keeps_open_smooth_semantics() {
    let mut builder = RecipeBuilder::new();
    let p = builder.add_profile(profile());
    let root = builder
        .add(NodeKind::Sweep {
            profile: p,
            path: Path3::Curves {
                start: [0.0; 3],
                segments: vec![PathSegment3::Line {
                    to: [0.0, 0.0, 10.0],
                }],
                section_x: [1.0, 0.0, 0.0],
                section_origin: [0.0; 2],
                closure: PathClosure::Open,
                joins: PathJoin::Smooth,
            },
            caps: CapMode::Both,
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut legacy = crate::text::dump_recipe(&recipe);
    let begin = legacy.find(" section_origin ").unwrap();
    let end = begin + legacy[begin..].find(" segments ").unwrap();
    legacy.replace_range(begin..end, "");
    legacy = legacy.replace("curved_path_sweep", "curved_sweep");
    assert_eq!(
        recipe.recipe_fingerprint(),
        crate::text::parse_recipe(&legacy)
            .unwrap()
            .recipe_fingerprint()
    );
    #[cfg(feature = "serde")]
    {
        use crate::interchange::{NodeKindDto, from_dto, to_dto};
        let mut dto = to_dto(&recipe);
        let NodeKindDto::CurvedPathSweep {
            profile,
            start,
            segments,
            section_x,
            caps,
            ..
        } = dto.nodes[0].kind.clone()
        else {
            panic!("curved path opcode")
        };
        dto.nodes[0].kind = NodeKindDto::CurvedSweep {
            profile,
            start,
            segments,
            section_x,
            caps,
        };
        assert_eq!(
            recipe.recipe_fingerprint(),
            from_dto(&dto).unwrap().recipe_fingerprint()
        );
    }
}
