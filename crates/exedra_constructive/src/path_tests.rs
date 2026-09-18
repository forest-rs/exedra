// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

fn cubic(p: [[f64; 3]; 4], t: f64) -> [f64; 3] {
    let u = 1.0 - t;
    add(
        add(scale(p[0], u * u * u), scale(p[1], 3.0 * u * u * t)),
        add(scale(p[2], 3.0 * u * t * t), scale(p[3], t * t * t)),
    )
}
fn derivative(p: [[f64; 3]; 4], t: f64) -> [f64; 3] {
    let u = 1.0 - t;
    add(
        add(
            scale(sub(p[1], p[0]), 3.0 * u * u),
            scale(sub(p[2], p[1]), 6.0 * u * t),
        ),
        scale(sub(p[3], p[2]), 3.0 * t * t),
    )
}
fn distance_to_chord(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab)).clamp(0.0, 1.0);
    norm(sub(p, add(a, scale(ab, t))))
}

#[test]
fn spatial_cubic_chord_and_tangent_bounds_hold_between_samples() {
    let p = [[0.0; 3], [0.0, 1.0, 4.0], [5.0, -2.0, 3.0], [6.0, 2.0, 7.0]];
    let curve = PathSegment3::Cubic {
        control1: p[1],
        control2: p[2],
        to: p[3],
    };
    let policy = PathDiscretizePolicy {
        chord_tolerance: 0.003,
        max_tangent_angle: 0.12,
        ..Default::default()
    };
    let sampled = discretize_path(p[0], &[curve], PathClosure::Open, PathJoin::Smooth, &policy)
        .expect("spatial cubic");
    assert_eq!(sampled.stations[0].point, p[0]);
    assert_eq!(sampled.stations.last().expect("end").point, p[3]);
    for (span, stations) in sampled
        .sampling
        .spans
        .iter()
        .zip(sampled.stations.windows(2))
    {
        assert!(span.chord_bound <= policy.chord_tolerance);
        assert!(span.tangent_angle_bound <= policy.max_tangent_angle);
        let mut tangents = Vec::new();
        for j in 0..=16 {
            let t =
                span.parameter[0] + (span.parameter[1] - span.parameter[0]) * f64::from(j) / 16.0;
            let distance = distance_to_chord(cubic(p, t), stations[0].point, stations[1].point);
            assert!(
                distance <= span.chord_bound + 1e-13,
                "chord deviation {distance} exceeds evidence {span:?}"
            );
            tangents.push(unit(derivative(p, t)).expect("regular tangent"));
        }
        for a in &tangents {
            for b in &tangents {
                let angle = libm::atan2(norm(cross(*a, *b)), dot(*a, *b));
                assert!(
                    angle <= span.tangent_angle_bound + 1e-12,
                    "tangent variation {angle} exceeds evidence {span:?}"
                );
            }
        }
    }
}

#[test]
fn arc_sampling_matches_right_handed_rotation_and_sagitta() {
    for sign in [-1.0, 1.0] {
        let sweep = sign * core::f64::consts::FRAC_PI_2;
        let policy = PathDiscretizePolicy {
            chord_tolerance: 0.002,
            max_tangent_angle: 0.07,
            ..Default::default()
        };
        let sampled = discretize_path(
            [5.0, 2.0, 0.0],
            &[PathSegment3::Arc {
                axis_origin: [0.0, 100.0, 0.0],
                axis: [0.0, 1e200, 0.0],
                sweep,
            }],
            PathClosure::Open,
            PathJoin::Smooth,
            &policy,
        )
        .expect("arc in a plane normal to Y");
        assert!(
            norm(sub(
                sampled.stations.last().expect("end").point,
                [0.0, 2.0, -sign * 5.0]
            )) < 1e-12
        );
        for (span, pair) in sampled
            .sampling
            .spans
            .iter()
            .zip(sampled.stations.windows(2))
        {
            for j in 0..=16 {
                let t = span.parameter[0]
                    + (span.parameter[1] - span.parameter[0]) * f64::from(j) / 16.0;
                let exact = [5.0 * libm::cos(sweep * t), 2.0, -5.0 * libm::sin(sweep * t)];
                assert!(
                    distance_to_chord(exact, pair[0].point, pair[1].point)
                        <= span.chord_bound + 1e-12
                );
            }
        }
    }
}

#[test]
fn tighter_policies_refine_and_budgets_never_silently_clamp() {
    let segments = [PathSegment3::Cubic {
        control1: [0.0, 0.0, 4.0],
        control2: [4.0, 3.0, 4.0],
        to: [4.0, 3.0, 8.0],
    }];
    let coarse_policy = PathDiscretizePolicy {
        chord_tolerance: 0.1,
        max_tangent_angle: 0.4,
        ..Default::default()
    };
    let coarse = discretize_path(
        [0.0; 3],
        &segments,
        PathClosure::Open,
        PathJoin::Smooth,
        &coarse_policy,
    )
    .expect("coarse");
    let fine = discretize_path(
        [0.0; 3],
        &segments,
        PathClosure::Open,
        PathJoin::Smooth,
        &PathDiscretizePolicy {
            chord_tolerance: 0.001,
            max_tangent_angle: 0.02,
            ..coarse_policy
        },
    )
    .expect("fine");
    assert!(fine.sampling.spans.len() > coarse.sampling.spans.len());
    for segment in [
        segments[0].clone(),
        PathSegment3::Arc {
            axis_origin: [4.0, 0.0, 0.0],
            axis: [0.0, 1.0, 0.0],
            sweep: core::f64::consts::FRAC_PI_2,
        },
    ] {
        assert!(matches!(
            discretize_path(
                [0.0; 3],
                &[segment],
                PathClosure::Open,
                PathJoin::Smooth,
                &PathDiscretizePolicy {
                    max_segment_edges: 1,
                    ..coarse_policy
                },
            ),
            Err(PathDiscretizeError::SegmentBudgetExceeded {
                segment: 0,
                maximum: 1
            })
        ));
    }
    assert!(matches!(
        discretize_path(
            [0.0; 3],
            &segments,
            PathClosure::Open,
            PathJoin::Smooth,
            &PathDiscretizePolicy {
                max_path_edges: 1,
                ..coarse_policy
            },
        ),
        Err(PathDiscretizeError::PathBudgetExceeded { maximum: 1 })
    ));
}

#[test]
fn endpoints_and_segment_parameter_intervals_survive_refinement() {
    let segments = [
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
    ];
    let sampled = discretize_path(
        [0.0; 3],
        &segments,
        PathClosure::Open,
        PathJoin::Smooth,
        &PathDiscretizePolicy::default(),
    )
    .expect("line/arc/cubic");
    assert_eq!(sampled.stations.len(), sampled.sampling.spans.len() + 1);
    assert_eq!(sampled.stations[1].point, [0.0, 0.0, 3.0]);
    assert_eq!(sampled.stations.last().expect("end").point, [8.0, 2.0, 7.0]);
    for segment in 0..3 {
        let spans: Vec<_> = sampled
            .sampling
            .spans
            .iter()
            .filter(|s| s.segment == segment)
            .collect();
        assert_eq!(spans[0].parameter[0], 0.0);
        assert_eq!(spans.last().expect("spans").parameter[1], 1.0);
        for pair in spans.windows(2) {
            assert_eq!(pair[0].parameter[1], pair[1].parameter[0]);
        }
    }
}

#[test]
fn invalid_geometry_stationary_tangents_and_sharp_joins_fail() {
    let policy = PathDiscretizePolicy::default();
    assert!(matches!(
        discretize_path([0.0; 3], &[], PathClosure::Open, PathJoin::Smooth, &policy,),
        Err(PathDiscretizeError::InvalidPath)
    ));
    assert!(matches!(
        discretize_path(
            [0.0; 3],
            &[PathSegment3::Line { to: [0.0; 3] }],
            PathClosure::Open,
            PathJoin::Smooth,
            &policy,
        ),
        Err(PathDiscretizeError::InvalidSegment { segment: 0 })
    ));
    let kink = [
        PathSegment3::Line {
            to: [1.0, 0.0, 0.0],
        },
        PathSegment3::Line {
            to: [1.0, 1.0, 0.0],
        },
    ];
    assert!(matches!(
        discretize_path(
            [0.0; 3],
            &kink,
            PathClosure::Open,
            PathJoin::Smooth,
            &policy,
        ),
        Err(PathDiscretizeError::DiscontinuousTangent { segment: 1 })
    ));
    let stationary = PathSegment3::Cubic {
        control1: [0.0; 3],
        control2: [1.0, 0.0, 0.0],
        to: [2.0, 0.0, 0.0],
    };
    assert!(matches!(
        discretize_path(
            [0.0; 3],
            &[stationary],
            PathClosure::Open,
            PathJoin::Smooth,
            &policy,
        ),
        Err(PathDiscretizeError::StationaryTangent {
            segment: 0,
            parameter: 0.0
        })
    ));
    let reversal = PathSegment3::Cubic {
        control1: [1.0, 0.0, 0.0],
        control2: [-1.0, 0.0, 0.0],
        to: [0.1, 0.0, 0.0],
    };
    assert!(
        discretize_path(
            [0.0; 3],
            &[reversal],
            PathClosure::Open,
            PathJoin::Smooth,
            &policy,
        )
        .is_err(),
        "interior reversals cannot pass the tangent cone"
    );
    assert!(matches!(
        discretize_path(
            [1.0, 0.0, 0.0],
            &[PathSegment3::Arc {
                axis_origin: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                sweep: core::f64::consts::TAU
            }],
            PathClosure::Open,
            PathJoin::Smooth,
            &policy,
        ),
        Err(PathDiscretizeError::InvalidPath)
    ));
    for axis in [[0.0; 3], [f64::NAN, 1.0, 0.0]] {
        assert!(matches!(
            discretize_path(
                [1.0, 0.0, 0.0],
                &[PathSegment3::Arc {
                    axis_origin: [0.0; 3],
                    axis,
                    sweep: 1.0
                }],
                PathClosure::Open,
                PathJoin::Smooth,
                &policy,
            ),
            Err(PathDiscretizeError::InvalidSegment { segment: 0 })
        ));
    }
}

#[test]
fn unrepresentable_accuracy_and_invalid_policy_are_explicit() {
    let segments = [PathSegment3::Cubic {
        control1: [1e12, 0.0, 1.0],
        control2: [1e12 + 1.0, 0.0, 2.0],
        to: [1e12 + 1.0, 0.0, 3.0],
    }];
    assert!(matches!(
        discretize_path(
            [1e12, 0.0, 0.0],
            &segments,
            PathClosure::Open,
            PathJoin::Smooth,
            &PathDiscretizePolicy {
                chord_tolerance: 1e-8,
                ..Default::default()
            },
        ),
        Err(PathDiscretizeError::NumericLimit { segment: 0 })
    ));
    for policy in [
        PathDiscretizePolicy {
            chord_tolerance: 0.0,
            ..Default::default()
        },
        PathDiscretizePolicy {
            max_tangent_angle: f64::NAN,
            ..Default::default()
        },
        PathDiscretizePolicy {
            max_tangent_angle: 2.0,
            ..Default::default()
        },
        PathDiscretizePolicy {
            max_segment_edges: 0,
            ..Default::default()
        },
        PathDiscretizePolicy {
            max_path_edges: 65536,
            ..Default::default()
        },
    ] {
        assert!(
            matches!(
                discretize_path([0.0; 3], &[], PathClosure::Open, PathJoin::Smooth, &policy,),
                Err(PathDiscretizeError::InvalidPolicy)
            ),
            "policy validation precedes traversal"
        );
    }
}

#[test]
fn collinear_cubic_preserves_authored_geometry_without_false_curvature() {
    let sampled = discretize_path(
        [0.0; 3],
        &[PathSegment3::Cubic {
            control1: [1.0, 2.0, 3.0],
            control2: [2.0, 4.0, 6.0],
            to: [3.0, 6.0, 9.0],
        }],
        PathClosure::Open,
        PathJoin::Smooth,
        &PathDiscretizePolicy {
            max_segment_edges: 1,
            ..Default::default()
        },
    )
    .expect("linear cubic");
    assert_eq!(sampled.sampling.spans.len(), 1);
    assert!(sampled.sampling.spans[0].tangent_angle_bound < 1e-10);
}

#[test]
fn bounds_remain_truthful_across_extreme_recipe_units() {
    let normalized = [[0.0; 3], [0.0, 1.0, 4.0], [5.0, -2.0, 3.0], [6.0, 2.0, 7.0]];
    for units in [1e-180, 1.0, 1e180] {
        let p = normalized.map(|point| scale(point, units));
        let sampled = discretize_path(
            p[0],
            &[PathSegment3::Cubic {
                control1: p[1],
                control2: p[2],
                to: p[3],
            }],
            PathClosure::Open,
            PathJoin::Smooth,
            &PathDiscretizePolicy {
                chord_tolerance: 0.001 * units,
                max_tangent_angle: 0.1,
                ..Default::default()
            },
        )
        .expect("scaled cubic");
        for (span, pair) in sampled
            .sampling
            .spans
            .iter()
            .zip(sampled.stations.windows(2))
        {
            let t = (span.parameter[0] + span.parameter[1]) * 0.5;
            let distance = distance_to_chord(
                cubic(normalized, t),
                pair[0].point.map(|v| v / units),
                pair[1].point.map(|v| v / units),
            );
            assert!(
                distance <= span.chord_bound / units + 1e-12,
                "squared norms must not erase a tiny curve's deviation"
            );
        }
    }
}
