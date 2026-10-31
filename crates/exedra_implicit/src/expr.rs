// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fields as data: a closed tree of the built-in shapes, profiles,
//! combinations and placements.
//!
//! Formulas are Rust types, chosen when the program is compiled. An
//! [`Expr`] is a value, chosen at run time, so it can be stored, compared,
//! serialized by a consumer, and built from data such as a recipe. Each
//! variant holds the built-in type it names, with its parameters checked by
//! that type's constructor, and evaluates exactly as that type does: values,
//! gradients, bounds, creases, regions and claims all carry over.

use alloc::boxed::Box;

use crate::claims::{Claims, ProfileClaims};
use crate::formula::{Formula, Profile};
use crate::number::Number;
use crate::{
    Capsule, Circle, Cuboid, Cylinder, Difference, Extrude, HalfPlane, HalfSpace, Intersection,
    Placed, Rectangle, Revolve, RoundCone, Scale, SmoothUnion, SmoothUnionN, Sphere, Tagged, Torus,
    Translate, Union,
};

/// A profile in the plane as data.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileExpr {
    /// A disk.
    Circle(Circle),
    /// An axis-aligned rectangle.
    Rectangle(Rectangle),
    /// The half of the plane on one side of a line.
    HalfPlane(HalfPlane),
}

impl Profile for ProfileExpr {
    fn eval<N: Number>(&self, p: [N; 2]) -> N {
        match self {
            Self::Circle(circle) => circle.eval(p),
            Self::Rectangle(rectangle) => rectangle.eval(p),
            Self::HalfPlane(half) => half.eval(p),
        }
    }

    fn claims(&self) -> ProfileClaims {
        match self {
            Self::Circle(circle) => circle.claims(),
            Self::Rectangle(rectangle) => rectangle.claims(),
            Self::HalfPlane(half) => half.claims(),
        }
    }
}

/// A field as data.
///
/// ```
/// use exedra_implicit::{Cuboid, Cylinder, Difference, Expr, Formula, Tagged};
///
/// // A box drilled through, its two parts tagged for the faces they make.
/// let drilled = Expr::from(Difference::new(
///     Expr::from(Tagged::new(Expr::from(Cuboid::new([0.0; 3], [0.6; 3]).unwrap()), 1)),
///     Expr::from(Tagged::new(
///         Expr::from(Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.3, 1.0).unwrap()),
///         2,
///     )),
/// ));
/// assert!(drilled.eval([0.0; 3]) > 0.0);
/// assert!(drilled.eval([0.5, 0.5, 0.0]) < 0.0);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A ball.
    Sphere(Sphere),
    /// A box.
    Cuboid(Cuboid),
    /// A finite cylinder.
    Cylinder(Cylinder),
    /// A torus.
    Torus(Torus),
    /// The solid on one side of a plane.
    HalfSpace(HalfSpace),
    /// Everything within a radius of a segment.
    Capsule(Capsule),
    /// The hull of two balls.
    RoundCone(RoundCone),
    /// A profile extruded along `z`.
    Extrude(Extrude<ProfileExpr>),
    /// A profile revolved about `y`.
    Revolve(Revolve<ProfileExpr>),
    /// Everything inside either part.
    Union(Box<Union<Self, Self>>),
    /// Everything inside both parts.
    Intersection(Box<Intersection<Self, Self>>),
    /// Everything inside the first part and outside the second.
    Difference(Box<Difference<Self, Self>>),
    /// Two parts unioned with the crease between them rounded away.
    SmoothUnion(Box<SmoothUnion<Self, Self>>),
    /// Any number of parts blended symmetrically.
    Blend(SmoothUnionN<Self>),
    /// A part moved.
    Translate(Box<Translate<Self>>),
    /// A part scaled about the origin.
    Scale(Box<Scale<Self>>),
    /// A part turned and moved.
    Placed(Box<Placed<Self>>),
    /// A part labelled with a region.
    Tagged(Box<Tagged<Self>>),
}

impl Formula for Expr {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        match self {
            Self::Sphere(f) => f.eval(p),
            Self::Cuboid(f) => f.eval(p),
            Self::Cylinder(f) => f.eval(p),
            Self::Torus(f) => f.eval(p),
            Self::HalfSpace(f) => f.eval(p),
            Self::Capsule(f) => f.eval(p),
            Self::RoundCone(f) => f.eval(p),
            Self::Extrude(f) => f.eval(p),
            Self::Revolve(f) => f.eval(p),
            Self::Union(f) => f.eval(p),
            Self::Intersection(f) => f.eval(p),
            Self::Difference(f) => f.eval(p),
            Self::SmoothUnion(f) => f.eval(p),
            Self::Blend(f) => f.eval(p),
            Self::Translate(f) => f.eval(p),
            Self::Scale(f) => f.eval(p),
            Self::Placed(f) => f.eval(p),
            Self::Tagged(f) => f.eval(p),
        }
    }

    fn claims(&self) -> Claims {
        match self {
            Self::Sphere(f) => f.claims(),
            Self::Cuboid(f) => f.claims(),
            Self::Cylinder(f) => f.claims(),
            Self::Torus(f) => f.claims(),
            Self::HalfSpace(f) => f.claims(),
            Self::Capsule(f) => f.claims(),
            Self::RoundCone(f) => f.claims(),
            Self::Extrude(f) => f.claims(),
            Self::Revolve(f) => f.claims(),
            Self::Union(f) => f.claims(),
            Self::Intersection(f) => f.claims(),
            Self::Difference(f) => f.claims(),
            Self::SmoothUnion(f) => f.claims(),
            Self::Blend(f) => f.claims(),
            Self::Translate(f) => f.claims(),
            Self::Scale(f) => f.claims(),
            Self::Placed(f) => f.claims(),
            Self::Tagged(f) => f.claims(),
        }
    }
}

macro_rules! from_profiles {
    ($($variant:ident),*) => {$(
        impl From<$variant> for ProfileExpr {
            fn from(profile: $variant) -> Self {
                Self::$variant(profile)
            }
        }
    )*};
}

from_profiles!(Circle, Rectangle, HalfPlane);

macro_rules! from_leaves {
    ($($variant:ident($ty:ty)),*) => {$(
        impl From<$ty> for Expr {
            fn from(field: $ty) -> Self {
                Self::$variant(field)
            }
        }
    )*};
}

from_leaves!(
    Sphere(Sphere),
    Cuboid(Cuboid),
    Cylinder(Cylinder),
    Torus(Torus),
    HalfSpace(HalfSpace),
    Capsule(Capsule),
    RoundCone(RoundCone),
    Extrude(Extrude<ProfileExpr>),
    Revolve(Revolve<ProfileExpr>),
    Blend(SmoothUnionN<Expr>)
);

macro_rules! from_boxed {
    ($($variant:ident($ty:ty)),*) => {$(
        impl From<$ty> for Expr {
            fn from(field: $ty) -> Self {
                Self::$variant(Box::new(field))
            }
        }
    )*};
}

from_boxed!(
    Union(Union<Expr, Expr>),
    Intersection(Intersection<Expr, Expr>),
    Difference(Difference<Expr, Expr>),
    SmoothUnion(SmoothUnion<Expr, Expr>),
    Translate(Translate<Expr>),
    Scale(Scale<Expr>),
    Placed(Placed<Expr>),
    Tagged(Tagged<Expr>)
);

#[cfg(test)]
mod tests {
    use alloc::vec;

    use exedra_math::Placement3;

    use super::{Expr, ProfileExpr};
    use crate::{
        Circle, Cuboid, Difference, Dual, Extrude, Formula, Placed, ScalarField, SmoothUnionN,
        Sphere, Tagged, Union,
    };

    /// The same solid as a typed formula and as data.
    #[test]
    fn data_evaluates_as_its_types_do() {
        let ball = Sphere::new([0.3, 0.0, 0.0], 0.5).unwrap();
        let block = Cuboid::new([0.0; 3], [0.4, 0.3, 0.2]).unwrap();
        let rod = Extrude::new(Circle::new([0.0, 0.0], 0.1).unwrap(), 0.6).unwrap();
        let typed = Difference::new(Union::new(ball, Tagged::new(block, 7)), rod);
        let data = Expr::from(Difference::new(
            Expr::from(Union::new(
                Expr::from(ball),
                Expr::from(Tagged::new(Expr::from(block), 7)),
            )),
            Expr::from(
                Extrude::new(
                    ProfileExpr::from(Circle::new([0.0, 0.0], 0.1).unwrap()),
                    0.6,
                )
                .unwrap(),
            ),
        ));
        for p in [
            [0.0; 3],
            [0.35, 0.05, 0.1],
            [-0.4, 0.3, 0.2],
            [0.9, -0.1, 0.0],
        ] {
            assert_eq!(typed.eval(p), data.eval(p));
            let (mut a, mut b) = ([Dual::default()], [Dual::default()]);
            typed.eval_gradients(&[p], &mut a);
            data.eval_gradients(&[p], &mut b);
            assert_eq!(a, b);
            assert_eq!(typed.eval_crease(p), data.eval_crease(p));
        }
        assert_eq!(Formula::claims(&typed), Formula::claims(&data));
    }

    #[test]
    fn blends_and_placements_hold_data() {
        let parts = vec![
            Expr::from(Sphere::new([-0.3, 0.0, 0.0], 0.4).unwrap()),
            Expr::from(Sphere::new([0.3, 0.0, 0.0], 0.4).unwrap()),
        ];
        let blend = Expr::from(SmoothUnionN::new(parts, 0.1).unwrap());
        assert!(blend.eval([0.0; 3]) < -0.1);
        let turned = Expr::from(
            Placed::new(
                blend.clone(),
                Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 0.0, 0.0, 1.0),
            )
            .unwrap(),
        );
        assert!((turned.eval([0.0, 0.3, 1.0]) - blend.eval([0.3, 0.0, 0.0])).abs() < 1e-12);
        assert_eq!(turned.clone(), turned);
    }
}
