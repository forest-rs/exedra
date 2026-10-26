// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Claims checked against sampled values.

use alloc::vec;
use alloc::vec::Vec;

use crate::analytic::{
    BoxField, CylinderField, Difference, HalfSpaceField, Intersection, SmoothUnion, SphereField,
    TorusField, Union,
};
use crate::analytic2d::{CircleField2d, RectField2d};
use crate::capability::{FieldCapabilities, Metric, Support};
use crate::lift::{Extrude, Revolve};
use crate::skeletal::{CapsuleField, RoundConeField, SmoothUnionN};
use crate::spatial::Aabb;
use crate::transform::{Offset, RigidTransform3, ScaleValue, Transform3, Translate, UniformScale};
use crate::{
    Aabb2, DualContourParams, EdgeSearchParams, FieldError, QefParams, ScalarField, ScalarField2d,
    dual_contour,
};

/// A deterministic sequence in `[0, 1)`.
struct Sequence(u64);

impl Sequence {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        #[expect(clippy::cast_precision_loss, reason = "24 high bits")]
        {
            (self.0 >> 40) as f32 / (1_u64 << 24) as f32
        }
    }

    fn point(&mut self, half: f32) -> [f32; 3] {
        [0, 1, 2].map(|_| (2.0 * self.next() - 1.0) * half)
    }
}

fn value<F: ScalarField>(field: &F, point: [f32; 3]) -> f32 {
    let mut out = [0.0];
    field.eval_points(&[point], &mut out);
    out[0]
}

/// Checks that sampled inside points lie in the claimed support and that
/// sampled pairs respect the claimed Lipschitz factor.
fn check_claims<F: ScalarField>(name: &str, field: &F) -> FieldCapabilities {
    let claims = field.capabilities();
    let mut sequence = Sequence(17);
    let mut inside = 0;
    for _ in 0..4000 {
        let point = sequence.point(6.0);
        let v = value(field, point);
        if v < 0.0 {
            inside += 1;
            if let Support::Bounded(b) | Support::Asserted(b) = claims.support {
                assert!(
                    b.contains(point),
                    "{name}: {point:?} inside but outside {b:?}"
                );
            }
            assert!(
                claims.support != Support::Empty,
                "{name}: inside an empty support"
            );
        }
        if let Some(lipschitz) = claims.metric.lipschitz() {
            let other = sequence.point(6.0);
            let distance = exedra_math::norm(exedra_math::sub(point, other));
            let change = (v - value(field, other)).abs();
            assert!(
                change <= lipschitz * distance * (1.0 + 1e-4) + 1e-4,
                "{name}: change {change} over {distance} exceeds {lipschitz}"
            );
        }
    }
    assert!(
        inside > 0 || claims.support == Support::Empty,
        "{name}: no samples inside"
    );
    claims
}

fn sphere(center: [f32; 3], radius: f32) -> SphereField {
    SphereField::new(center, radius).unwrap()
}

#[test]
fn built_in_fields_claim_exact_distances_within_their_bounds() {
    let up = [0.3, 0.5, 0.8];
    let checks: Vec<(&str, FieldCapabilities)> = vec![
        (
            "sphere",
            check_claims("sphere", &sphere([0.5, -1.0, 0.25], 2.0)),
        ),
        (
            "box",
            check_claims(
                "box",
                &BoxField::new([1.0, 0.0, -1.0], [1.0, 2.0, 0.5]).unwrap(),
            ),
        ),
        (
            "cylinder",
            check_claims(
                "cylinder",
                &CylinderField::new([0.0; 3], up, 1.5, 2.0).unwrap(),
            ),
        ),
        (
            "torus",
            check_claims("torus", &TorusField::new([0.0; 3], up, 2.0, 0.75).unwrap()),
        ),
        (
            "capsule",
            check_claims(
                "capsule",
                &CapsuleField::new([-2.0, 0.0, 1.0], [2.0, 1.0, -1.0], 0.75).unwrap(),
            ),
        ),
        (
            "round cone",
            check_claims(
                "round cone",
                &RoundConeField::new([-2.0, 0.0, 0.0], [2.0, 1.0, 0.0], 1.5, 0.5).unwrap(),
            ),
        ),
    ];
    for (name, claims) in checks {
        assert_eq!(claims.metric, Metric::ExactDistance, "{name}");
        assert!(matches!(claims.support, Support::Bounded(_)), "{name}");
    }
    let half = HalfSpaceField::new([0.0; 3], [0.0, 0.0, 1.0]).unwrap();
    assert_eq!(
        check_claims("half space", &half).support,
        Support::Unbounded
    );
}

#[test]
fn composition_keeps_only_justified_claims() {
    let (a, b) = (sphere([-1.0, 0.0, 0.0], 1.5), sphere([1.5, 0.0, 0.0], 1.0));
    // A minimum or maximum of distances is no longer an exact distance.
    let union = check_claims("union", &Union::new(a, b));
    assert_eq!(union.metric, Metric::Lipschitz(1.0));
    let difference = check_claims("difference", &Difference::new(a, b));
    assert_eq!(difference.support, a.capabilities().support);
    // Bounded by a half space, the intersection is bounded by the sphere.
    let cut = Intersection::new(a, HalfSpaceField::new([0.0; 3], [1.0, 0.0, 0.0]).unwrap());
    assert_eq!(check_claims("cut", &cut).support, a.capabilities().support);
    // A blend reaches beyond both shapes, by at most a quarter of k.
    let blended = check_claims("smooth union", &SmoothUnion::new(a, b, 1.2));
    let Support::Bounded(grown) = blended.support else {
        panic!("exact operands bound the blend");
    };
    let Support::Bounded(plain) = union.support else {
        panic!("bounded operands");
    };
    assert!(grown.min[0] < plain.min[0] && grown.max[0] > plain.max[0]);
    let many = SmoothUnionN::new(vec![a, b, sphere([0.0, 1.5, 0.0], 0.8)], 0.7);
    assert!(matches!(
        check_claims("n-ary blend", &many).support,
        Support::Bounded(_)
    ));
    let none: SmoothUnionN<SphereField> = SmoothUnionN::new(vec![], 0.7);
    assert_eq!(none.capabilities().support, Support::Empty);
}

#[test]
fn transforms_and_lifts_carry_claims() {
    let ball = sphere([0.5, 0.0, 0.0], 1.0);
    let moved = check_claims("translate", &Translate::new(ball, [1.0, -2.0, 0.5]));
    assert_eq!(moved.metric, Metric::ExactDistance);
    let grown = check_claims("scale", &UniformScale::new(ball, 2.5).unwrap());
    assert_eq!(grown.metric, Metric::ExactDistance);
    let turned = RigidTransform3::new(
        [1.0, 0.5, -0.5],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    )
    .unwrap();
    let box_field = BoxField::new([1.0, 0.0, 0.0], [1.5, 0.5, 0.75]).unwrap();
    check_claims("rigid", &Transform3::new(box_field, turned));
    let slab = Extrude::new(RectField2d::new([0.0, 0.5], [1.5, 1.0]).unwrap(), 1.25).unwrap();
    assert_eq!(check_claims("extrude", &slab).metric, Metric::ExactDistance);
    let ring = Revolve::new(CircleField2d::new([2.0, 0.0], 0.75).unwrap());
    assert_eq!(check_claims("revolve", &ring).metric, Metric::ExactDistance);
    // A profile reaching across the axis keeps only its Lipschitz bound.
    let across = Revolve::new(CircleField2d::new([0.25, 0.0], 1.0).unwrap());
    assert_eq!(
        check_claims("revolve across", &across).metric,
        Metric::Lipschitz(1.0)
    );
}

fn surface<F: ScalarField>(field: &F) -> Vec<[f32; 3]> {
    let params = DualContourParams {
        root_bounds: Aabb::new([-2.0; 3], [2.0; 3]).unwrap(),
        max_depth: 5,
        cell_budget: None,
        limits: crate::ExtractionLimits::default(),
        witness_limit: 0,
        vertex_merge_tolerance: 0.0,
        edge_search: EdgeSearchParams::default(),
        qef: QefParams::default(),
    };
    let mesh = dual_contour(field, &params).unwrap().mesh;
    mesh.vertices()
        .map(|vertex| *mesh.vertex_position(vertex).unwrap())
        .collect()
}

#[test]
fn rescaling_values_keeps_the_surface_and_scales_the_metric() {
    let ball = sphere([0.1, -0.2, 0.05], 1.2);
    let scaled = ScaleValue::new(ball, 3.0).unwrap();
    let claims = check_claims("scaled values", &scaled);
    assert_eq!(claims.metric, Metric::Lipschitz(3.0));
    assert_eq!(claims.support, ball.capabilities().support);
    // The zero set is the same, so extraction finds the same surface.
    let (plain, rescaled) = (surface(&ball), surface(&scaled));
    assert_eq!(plain.len(), rescaled.len());
    for (a, b) in plain.iter().zip(&rescaled) {
        assert!(
            exedra_math::norm(exedra_math::sub(*a, *b)) < 1e-4,
            "{a:?} vs {b:?}"
        );
    }
    // Resizing the geometry instead keeps distances distances.
    let resized = UniformScale::new(ball, 3.0).unwrap().capabilities();
    assert_eq!(resized.metric, Metric::ExactDistance);
    assert_eq!(
        ScaleValue::new(ball, -1.0),
        Err(FieldError::NonPositive {
            parameter: "factor"
        })
    );
}

#[test]
fn offsets_grow_support_only_from_exact_distances() {
    let ball = sphere([0.0; 3], 1.0);
    let grown = Offset::new(ball, 0.5).unwrap();
    let claims = check_claims("grown", &grown);
    // Outside stays exact, inside need not, so only the bound survives.
    assert_eq!(claims.metric, Metric::Lipschitz(1.0));
    let Support::Bounded(b) = claims.support else {
        panic!("an exact distance bounds the offset");
    };
    assert!(b.max[0] >= 1.5 && b.max[0] < 1.5001);
    // The same values, so the same surface, as a larger sphere.
    let larger = surface(&sphere([0.0; 3], 1.5));
    let offset = surface(&grown);
    assert_eq!(larger.len(), offset.len());
    for (a, b) in larger.iter().zip(&offset) {
        assert!(exedra_math::norm(exedra_math::sub(*a, *b)) < 1e-4, "{a:?} vs {b:?}");
    }

    // A half-weighted distance is still Lipschitz, but its offset reaches
    // twice as far as the distance suggests: support cannot be bounded.
    let half = ScaleValue::new(ball, 0.5).unwrap();
    let spread = Offset::new(half, 0.5).unwrap();
    assert_eq!(check_claims("spread", &spread).support, Support::Unknown);
    assert!(value(&spread, [1.9, 0.0, 0.0]) < 0.0);

    // Shrinking never leaves the original interior, whatever the metric.
    let shrunk = Offset::new(half, -0.2).unwrap();
    assert_eq!(
        check_claims("shrunk", &shrunk).support,
        ball.capabilities().support
    );
    assert_eq!(
        Offset::new(ball, f32::NAN),
        Err(FieldError::NonFinite {
            parameter: "distance"
        })
    );
}

#[test]
fn transforms_reject_unusable_parameters() {
    let ball = sphere([0.0; 3], 1.0);
    assert_eq!(
        UniformScale::new(ball, 0.0),
        Err(FieldError::NonPositive {
            parameter: "factor"
        })
    );
    assert_eq!(
        RigidTransform3::new([0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        Err(FieldError::NotOrthogonal { parameter: "axes" })
    );
    assert_eq!(
        RigidTransform3::new([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        Err(FieldError::DegenerateDirection {
            parameter: "x_axis"
        })
    );
}

#[test]
fn skewed_frames_bound_far_shapes() {
    // Axes within the accepted 1e-4 of perpendicular, and a frame origin a
    // million units away that brings a far local sphere back to the world
    // origin. There, the transposed axes miss the true inverse by ninety
    // units, while world coordinates, and any margin scaled by them, are small.
    let skew = 9.0e-5;
    let frame = RigidTransform3::new(
        [0.0, -1.0e6, 0.0],
        [1.0, 0.0, 0.0],
        [skew, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    )
    .unwrap();
    let far = Transform3::new(sphere([0.0, 1.0e6, 0.0], 1.0), frame);
    let Support::Bounded(b) = far.capabilities().support else {
        panic!("a bounded sphere stays bounded");
    };
    assert!(value(&far, [0.0; 3]) < 0.0);
    assert!(
        b.contains([0.0; 3]),
        "the world origin is inside but outside {b:?}"
    );
    // A frame slightly off at the origin's scale still bounds sampled points.
    let near = Transform3::new(
        BoxField::new([1.0, -0.5, 0.0], [1.5, 0.5, 0.75]).unwrap(),
        RigidTransform3::new([0.5; 3], [1.0, 0.0, 0.0], [skew, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap(),
    );
    check_claims("skewed", &near);
}

#[test]
fn self_overlapping_and_vanishing_shapes_drop_claims() {
    // Major radius 1, minor 2: the origin reads -1 but is sqrt(3) deep.
    let spindle = TorusField::new([0.0; 3], [0.0, 0.0, 1.0], 1.0, 2.0).unwrap();
    assert_eq!(
        check_claims("spindle", &spindle).metric,
        Metric::Lipschitz(1.0)
    );
    let ring = TorusField::new([0.0; 3], [0.0, 0.0, 1.0], 2.0, 1.0).unwrap();
    assert_eq!(ring.capabilities().metric, Metric::ExactDistance);

    // Two opposing half spaces need not meet at all.
    let up = HalfSpaceField::new([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]).unwrap();
    let down = HalfSpaceField::new([0.0, 0.0, -1.0], [0.0, 0.0, -1.0]).unwrap();
    assert_eq!(
        Intersection::new(up, down).capabilities().support,
        Support::Unknown
    );
    assert_eq!(
        Difference::new(up, up).capabilities().support,
        Support::Unknown
    );
    assert_eq!(
        Difference::new(up, sphere([0.0; 3], 1.0))
            .capabilities()
            .support,
        Support::Unbounded
    );
    assert_eq!(
        Offset::new(up, -0.5).unwrap().capabilities().support,
        Support::Unknown
    );

    // Tiny value scales round up rather than underflow to zero.
    let tiny = ScaleValue::new(
        ScaleValue::new(sphere([0.0; 3], 1.0), 1e-30).unwrap(),
        1e-30,
    )
    .unwrap();
    let Metric::Lipschitz(bound) = tiny.capabilities().metric else {
        panic!("a scaled distance keeps a bound");
    };
    assert!(bound > 0.0);
    assert_eq!(
        Metric::Lipschitz(3.0).scaled(1.0 / 3.0),
        Metric::Lipschitz(1.0_f32.next_up())
    );
}

/// A caller field that states nothing.
struct Opaque;

impl ScalarField for Opaque {
    fn eval_interval(&self, _: &Aabb) -> Option<[f32; 2]> {
        None
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        for (point, out) in points.iter().zip(out) {
            *out = point[0];
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        for (point, out) in points.iter().zip(out) {
            *out = [point[0], 1.0, 0.0, 0.0];
        }
    }
}

#[test]
fn unknown_fields_poison_claims_that_depend_on_them() {
    let ball = sphere([0.0; 3], 1.0);
    assert_eq!(Opaque.capabilities(), FieldCapabilities::UNKNOWN);
    let union = Union::new(ball, Opaque).capabilities();
    assert_eq!(union, FieldCapabilities::UNKNOWN);
    // Intersecting with a bounded field still bounds the interior.
    assert_eq!(
        Intersection::new(ball, Opaque).capabilities().support,
        ball.capabilities().support
    );
    // A blend with an unknown child cannot bound its growth.
    assert_eq!(
        SmoothUnion::new(ball, Opaque, 0.5).capabilities().support,
        Support::Unknown
    );
    let profile = RectField2d::new([0.0; 2], [1.0; 2]).unwrap();
    assert!(matches!(
        profile.capabilities().support,
        Support::Bounded(Aabb2 { .. })
    ));
}
