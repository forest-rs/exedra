// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Further exact primitives, after Íñigo Quilez's
//! [distance functions](https://iquilezles.org/articles/distfunctions/).
//!
//! Each primitive sits at `center` in a fixed orientation with `z` up; place
//! and turn it with [`Transform3`](crate::Transform3). Values are computed in
//! `f64`. Each claims an exact distance within its bounds, except where its
//! parameters make it overlap itself. Evaluation shares the machinery of
//! [`FnField`](crate::function::FnField): intervals follow from the distance
//! claim, and gradients are central differences of the `f64` distance.

use exedra_math::Real;

use crate::capability::{Capabilities, FieldCapabilities, Metric, exact_about};
use crate::function::{eval_gradients, eval_points, lipschitz_interval};
use crate::spatial::Aabb;
use crate::{FieldError, ScalarField, check_finite, check_positive};

/// The edges of a box: a frame of square bars.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxFrameField {
    center: [f32; 3],
    half_extents: [f32; 3],
    thickness: f32,
}

/// A chain link: a torus in the `xy` plane stretched along `y`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinkField {
    center: [f32; 3],
    half_length: f32,
    major_radius: f32,
    minor_radius: f32,
}

/// A regular octahedron with vertices on the axes.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct OctahedronField {
    center: [f32; 3],
    radius: f32,
}

/// A square pyramid standing on its base.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PyramidField {
    base_center: [f32; 3],
    half_base: f32,
    height: f32,
}

/// A hexagonal prism along `z`, flat sides facing `±y`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HexPrismField {
    center: [f32; 3],
    apothem: f32,
    half_height: f32,
}

/// A cone frustum along `z`, with possibly different end radii.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CappedConeField {
    center: [f32; 3],
    half_height: f32,
    bottom_radius: f32,
    top_radius: f32,
}

impl BoxFrameField {
    /// Creates a frame around the box `center ± half_extents`, with bars
    /// `thickness` wide inside it.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite, the extents and
    /// thickness are positive, and the thickness is at most every extent.
    pub fn new(
        center: [f32; 3],
        half_extents: [f32; 3],
        thickness: f32,
    ) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        for extent in half_extents {
            check_positive("half_extents", extent)?;
        }
        check_positive("thickness", thickness)?;
        if half_extents.iter().any(|&extent| thickness > extent) {
            return Err(FieldError::NonPositive {
                parameter: "half_extents - thickness",
            });
        }
        Ok(Self {
            center,
            half_extents,
            thickness,
        })
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        let b = widen(self.half_extents);
        let e = f64::from(self.thickness);
        let p: [f64; 3] = core::array::from_fn(|i| p[i].abs() - b[i]);
        let q: [f64; 3] = core::array::from_fn(|i| (p[i] + e).abs() - e);
        let bar = |v: [f64; 3]| length(v.map(|c| c.max(0.0))) + v[0].max(v[1]).max(v[2]).min(0.0);
        bar([p[0], q[1], q[2]])
            .min(bar([q[0], p[1], q[2]]))
            .min(bar([q[0], q[1], p[2]]))
    }

    fn claims(&self) -> FieldCapabilities {
        exact_about(self.center, self.half_extents)
    }
}

impl LinkField {
    /// Creates a link whose straight sides run `half_length` either way
    /// along `y`, bending with `major_radius` and made of a round bar of
    /// `minor_radius`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite, the length is
    /// not negative and both radii are positive.
    pub fn new(
        center: [f32; 3],
        half_length: f32,
        major_radius: f32,
        minor_radius: f32,
    ) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        crate::check_nonnegative("half_length", half_length)?;
        check_positive("major_radius", major_radius)?;
        check_positive("minor_radius", minor_radius)?;
        Ok(Self {
            center,
            half_length,
            major_radius,
            minor_radius,
        })
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        let q = [p[0], (p[1].abs() - f64::from(self.half_length)).max(0.0)];
        let ring = length2([q[0], q[1]]) - f64::from(self.major_radius);
        length2([ring, p[2]]) - f64::from(self.minor_radius)
    }

    fn claims(&self) -> FieldCapabilities {
        let reach = self.major_radius + self.minor_radius;
        let claims = exact_about(
            self.center,
            [reach, self.half_length + reach, self.minor_radius],
        );
        // As for a spindle torus, a bar thicker than its bend overlaps itself.
        if self.minor_radius > self.major_radius {
            return Capabilities {
                metric: Metric::Lipschitz(1.0),
                ..claims
            };
        }
        claims
    }
}

impl OctahedronField {
    /// Creates an octahedron with vertices `radius` from `center` along
    /// each axis.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite and the radius is
    /// positive.
    pub fn new(center: [f32; 3], radius: f32) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        check_positive("radius", radius)?;
        Ok(Self { center, radius })
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        let s = f64::from(self.radius);
        let p = p.map(f64::abs);
        let m = p[0] + p[1] + p[2] - s;
        let q = if 3.0 * p[0] < m {
            p
        } else if 3.0 * p[1] < m {
            [p[1], p[2], p[0]]
        } else if 3.0 * p[2] < m {
            [p[2], p[0], p[1]]
        } else {
            return m * 0.577_350_269_189_625_8;
        };
        let k = (0.5 * (q[2] - q[1] + s)).clamp(0.0, s);
        length([q[0], q[1] - s + k, q[2] - k])
    }

    fn claims(&self) -> FieldCapabilities {
        exact_about(self.center, [self.radius; 3])
    }
}

impl PyramidField {
    /// Creates a pyramid on a square base of half width `half_base`
    /// centered at `base_center`, with its apex `height` above.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite and the half
    /// base and height are positive.
    pub fn new(base_center: [f32; 3], half_base: f32, height: f32) -> Result<Self, FieldError> {
        check_finite("base_center", base_center)?;
        check_positive("half_base", half_base)?;
        check_positive("height", height)?;
        Ok(Self {
            base_center,
            half_base,
            height,
        })
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        // A convex solid: inside, the nearest face plane; outside, the
        // nearest face. Quilez's closed form holds only above the base.
        let (a, h) = (f64::from(self.half_base), f64::from(self.height));
        let apex = [0.0, 0.0, h];
        let corners = [[a, a, 0.0], [-a, a, 0.0], [-a, -a, 0.0], [a, -a, 0.0]];
        let slant = sqrt(h * h + a * a);
        let planes = [
            -p[2],
            (h * p[0] + a * p[2] - a * h) / slant,
            (-h * p[0] + a * p[2] - a * h) / slant,
            (h * p[1] + a * p[2] - a * h) / slant,
            (-h * p[1] + a * p[2] - a * h) / slant,
        ];
        let deepest = planes.into_iter().fold(f64::NEG_INFINITY, f64::max);
        if deepest <= 0.0 {
            return deepest;
        }
        let outside = [p[0].abs() - a, p[1].abs() - a].map(|v| v.max(0.0));
        let mut nearest = outside[0] * outside[0] + outside[1] * outside[1] + p[2] * p[2];
        for side in 0..4 {
            let triangle = [corners[side], corners[(side + 1) % 4], apex];
            nearest = nearest.min(triangle_distance2(p, triangle));
        }
        sqrt(nearest)
    }

    fn claims(&self) -> FieldCapabilities {
        let half_height = 0.5 * self.height;
        let center = [
            self.base_center[0],
            self.base_center[1],
            self.base_center[2] + half_height,
        ];
        exact_about(center, [self.half_base, self.half_base, half_height])
    }
}

impl HexPrismField {
    /// Creates a hexagonal prism whose flat sides lie `apothem` from its
    /// axis, reaching `half_height` either way along `z`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite and the apothem
    /// and half height are positive.
    pub fn new(center: [f32; 3], apothem: f32, half_height: f32) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        check_positive("apothem", apothem)?;
        check_positive("half_height", half_height)?;
        Ok(Self {
            center,
            apothem,
            half_height,
        })
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        const K: [f64; 3] = [-0.866_025_403_784_438_6, 0.5, 0.577_350_269_189_625_8];
        let (r, h) = (f64::from(self.apothem), f64::from(self.half_height));
        let mut p = p.map(f64::abs);
        let fold = 2.0 * (K[0] * p[0] + K[1] * p[1]).min(0.0);
        p[0] -= fold * K[0];
        p[1] -= fold * K[1];
        let edge = [p[0] - p[0].clamp(-K[2] * r, K[2] * r), p[1] - r];
        let d = [length2(edge) * (p[1] - r).signum(), p[2] - h];
        d[0].max(d[1]).min(0.0) + length2(d.map(|v| v.max(0.0)))
    }

    fn claims(&self) -> FieldCapabilities {
        // Corners lie at the circumradius, `2 / sqrt(3)` times the apothem.
        let corner = self.apothem * 1.154_700_6;
        exact_about(self.center, [corner, corner, self.half_height])
    }
}

impl CappedConeField {
    /// Creates a frustum reaching `half_height` either way along `z`, with
    /// `bottom_radius` at its lower end and `top_radius` at its upper.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite, the half height
    /// is positive, and the radii are not negative and not both zero.
    pub fn new(
        center: [f32; 3],
        half_height: f32,
        bottom_radius: f32,
        top_radius: f32,
    ) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        check_positive("half_height", half_height)?;
        crate::check_nonnegative("bottom_radius", bottom_radius)?;
        crate::check_nonnegative("top_radius", top_radius)?;
        check_positive("bottom_radius + top_radius", bottom_radius + top_radius)?;
        Ok(Self {
            center,
            half_height,
            bottom_radius,
            top_radius,
        })
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        let h = f64::from(self.half_height);
        let (r1, r2) = (f64::from(self.bottom_radius), f64::from(self.top_radius));
        let q = [length2([p[0], p[1]]), p[2]];
        let k1 = [r2, h];
        let k2 = [r2 - r1, 2.0 * h];
        let end = if q[1] < 0.0 { r1 } else { r2 };
        let ca = [q[0] - q[0].min(end), q[1].abs() - h];
        let along =
            ((k1[0] - q[0]) * k2[0] + (k1[1] - q[1]) * k2[1]) / (k2[0] * k2[0] + k2[1] * k2[1]);
        let t = along.clamp(0.0, 1.0);
        let cb = [q[0] - k1[0] + k2[0] * t, q[1] - k1[1] + k2[1] * t];
        let sign = if cb[0] < 0.0 && ca[1] < 0.0 {
            -1.0
        } else {
            1.0
        };
        let squared = |v: [f64; 2]| v[0] * v[0] + v[1] * v[1];
        sign * sqrt(squared(ca).min(squared(cb)))
    }

    fn claims(&self) -> FieldCapabilities {
        let radius = self.bottom_radius.max(self.top_radius);
        exact_about(self.center, [radius, radius, self.half_height])
    }
}

trait Origin {
    fn origin(&self) -> [f32; 3];
}

macro_rules! centered {
    ($($field:ty),*) => {$(
        impl Origin for $field {
            fn origin(&self) -> [f32; 3] {
                self.center
            }
        }
    )*};
}

centered!(
    BoxFrameField,
    LinkField,
    OctahedronField,
    HexPrismField,
    CappedConeField
);

impl Origin for PyramidField {
    fn origin(&self) -> [f32; 3] {
        self.base_center
    }
}

macro_rules! exact_primitive {
    ($($field:ty),*) => {$(
        impl ScalarField for $field {
            fn capabilities(&self) -> FieldCapabilities {
                self.claims()
            }

            fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
                lipschitz_interval(|p| self.distance(p), self.origin(), 1.0, bounds)
            }

            fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
                eval_points(|p| self.distance(p), self.origin(), points, out);
            }

            fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
                eval_gradients(|p| self.distance(p), self.origin(), points, out);
            }
        }
    )*};
}

exact_primitive!(
    BoxFrameField,
    LinkField,
    OctahedronField,
    PyramidField,
    HexPrismField,
    CappedConeField
);

/// Squared distance from `p` to a triangle, after Quilez's `udTriangle`.
fn triangle_distance2(p: [f64; 3], [a, b, c]: [[f64; 3]; 3]) -> f64 {
    let sub = |u: [f64; 3], v: [f64; 3]| core::array::from_fn::<f64, 3, _>(|i| u[i] - v[i]);
    let dot = |u: [f64; 3], v: [f64; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let cross = |u: [f64; 3], v: [f64; 3]| {
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ]
    };
    let (ab, pa) = (sub(b, a), sub(p, a));
    let (bc, pb) = (sub(c, b), sub(p, b));
    let (ca, pc) = (sub(a, c), sub(p, c));
    let normal = cross(ab, ca);
    let side = |edge: [f64; 3], from: [f64; 3]| dot(cross(edge, normal), from).signum();
    if side(ab, pa) + side(bc, pb) + side(ca, pc) < 2.0 {
        // Outside the prism over the triangle: the nearest edge.
        let edge = |edge: [f64; 3], from: [f64; 3]| {
            let t = (dot(edge, from) / dot(edge, edge)).clamp(0.0, 1.0);
            let gap = core::array::from_fn::<f64, 3, _>(|i| edge[i] * t - from[i]);
            dot(gap, gap)
        };
        edge(ab, pa).min(edge(bc, pb)).min(edge(ca, pc))
    } else {
        let height = dot(normal, pa);
        height * height / dot(normal, normal)
    }
}

fn length(v: [f64; 3]) -> f64 {
    sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

fn length2(v: [f64; 2]) -> f64 {
    sqrt(v[0] * v[0] + v[1] * v[1])
}

fn widen(point: [f32; 3]) -> [f64; 3] {
    point.map(f64::from)
}

fn sqrt(value: f64) -> f64 {
    Real::sqrt(value)
}
