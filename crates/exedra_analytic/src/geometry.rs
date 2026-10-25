// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-independent f64 curves, surfaces and parameter domains.
//!
//! Circle and cylinder angles are radians; plane coordinates and cylinder
//! height are model lengths. Derivatives are with respect to those parameters,
//! without normalization. Supporting geometry is untrimmed: body topology owns
//! the bounded portions and their oriented uses. Texture coordinates are not
//! surface parameters.

use exedra_math::{FrameError, Placement3};

/// Absolute tolerance for rigid frame axis lengths and dot products.
pub const FRAME_TOLERANCE: f64 = 1e-12;

/// A finite, increasing parameter interval with a finite positive span.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ParameterRange {
    min: f64,
    max: f64,
}

impl ParameterRange {
    /// The unit parameter interval.
    pub const UNIT: Self = Self { min: 0.0, max: 1.0 };

    /// Checks the bounds and their span.
    ///
    /// # Errors
    /// Returns [`GeometryError::Range`] for non-finite, reversed or equal bounds,
    /// or a span that overflows.
    pub fn new(min: f64, max: f64) -> Result<Self, GeometryError> {
        if !min.is_finite() || !max.is_finite() || !(max - min).is_finite() || min >= max {
            return Err(GeometryError::Range { min, max });
        }
        Ok(Self { min, max })
    }

    /// The lower and upper bounds.
    #[must_use]
    pub const fn bounds(self) -> [f64; 2] {
        [self.min, self.max]
    }

    /// The finite positive span.
    #[must_use]
    pub fn span(self) -> f64 {
        self.max - self.min
    }

    /// Whether `parameter` lies in this closed interval.
    #[must_use]
    pub fn contains(self, parameter: f64) -> bool {
        self.min <= parameter && parameter <= self.max
    }

    /// Maps a fraction in `[0,1]` to a parameter, preserving both endpoints.
    #[must_use]
    pub fn at(self, fraction: f64) -> Option<f64> {
        if !(0.0..=1.0).contains(&fraction) {
            return None;
        }
        Some(if fraction == 1.0 {
            self.max
        } else {
            self.min + fraction * self.span()
        })
    }
}

/// A finite rectangular surface parameter domain; trims bound the face within it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ParameterDomain {
    /// U bounds, in the supporting surface's parameter units.
    pub u: ParameterRange,
    /// V bounds, in the supporting surface's parameter units.
    pub v: ParameterRange,
}

impl ParameterDomain {
    /// Whether both coordinates lie in their closed intervals.
    #[must_use]
    pub fn contains(self, uv: [f64; 2]) -> bool {
        self.u.contains(uv[0]) && self.v.contains(uv[1])
    }
}

/// A line `origin + parameter * direction`; direction need not have unit length.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    origin: [f64; 3],
    direction: [f64; 3],
}

impl Line {
    /// The point at parameter zero.
    #[must_use]
    pub const fn origin(&self) -> [f64; 3] {
        self.origin
    }
    /// Displacement for one parameter unit, also the first derivative.
    #[must_use]
    pub const fn direction(&self) -> [f64; 3] {
        self.direction
    }
}

/// A circle in a rigid frame's XY plane, parameterized by radians from local +X.
#[derive(Clone, Debug, PartialEq)]
pub struct Circle {
    frame: Placement3,
    radius: f64,
}

impl Circle {
    /// The circle's rigid frame.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// The positive radius in model units.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
}

/// An untrimmed, validated 3D curve independent of any construction operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// An unbounded line.
    Line(Line),
    /// A periodic circle; its period is `TAU` radians.
    Circle(Circle),
}

/// Curve position and its first two parameter derivatives.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CurveEvaluation {
    /// Position in the curve's coordinate system.
    pub point: [f64; 3],
    /// First derivative, with respect to the curve parameter.
    pub first: [f64; 3],
    /// Second derivative, with respect to the curve parameter twice.
    pub second: [f64; 3],
}

impl Curve {
    /// Creates a line with a finite origin and finite, nonzero direction.
    ///
    /// # Errors
    /// Returns [`GeometryError::NonFinite`] or [`GeometryError::ZeroDirection`].
    pub fn line(origin: [f64; 3], direction: [f64; 3]) -> Result<Self, GeometryError> {
        if !finite(origin) || !finite(direction) {
            return Err(GeometryError::NonFinite);
        }
        if direction == [0.0; 3] {
            return Err(GeometryError::ZeroDirection);
        }
        Ok(Self::Line(Line { origin, direction }))
    }

    /// Creates a circle in a finite right-handed orthonormal frame.
    ///
    /// # Errors
    /// Returns a frame violation or [`GeometryError::Radius`].
    pub fn circle(frame: Placement3, radius: f64) -> Result<Self, GeometryError> {
        rigid(frame)?;
        check_radius(radius)?;
        Ok(Self::Circle(Circle { frame, radius }))
    }

    /// The parameter period; lines are not periodic.
    #[must_use]
    pub fn period(&self) -> Option<f64> {
        match self {
            Self::Line(_) => None,
            Self::Circle(_) => Some(core::f64::consts::TAU),
        }
    }

    /// Evaluates without clamping or wrapping the supplied parameter.
    ///
    /// Returns `None` for non-finite input or an overflowed result.
    #[must_use]
    pub fn evaluate(&self, parameter: f64) -> Option<CurveEvaluation> {
        if !parameter.is_finite() {
            return None;
        }
        let result = match self {
            Self::Line(line) => CurveEvaluation {
                point: core::array::from_fn(|i| line.origin[i] + parameter * line.direction[i]),
                first: line.direction,
                second: [0.0; 3],
            },
            Self::Circle(circle) => {
                let (sin, cos) = (libm::sin(parameter), libm::cos(parameter));
                let r = circle.radius;
                CurveEvaluation {
                    point: circle.frame.transform_point([r * cos, r * sin, 0.0]),
                    first: vector(circle.frame, [-r * sin, r * cos, 0.0]),
                    second: vector(circle.frame, [-r * cos, -r * sin, 0.0]),
                }
            }
        };
        (finite(result.point) && finite(result.first) && finite(result.second)).then_some(result)
    }
}

/// A plane parameterized by model lengths along a rigid frame's X/Y axes.
#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    frame: Placement3,
}

impl Plane {
    /// The plane's rigid frame; local +Z is the geometric normal.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
}

/// A cylinder about a rigid frame's Z axis, with U in radians and V in lengths.
#[derive(Clone, Debug, PartialEq)]
pub struct Cylinder {
    frame: Placement3,
    radius: f64,
}

impl Cylinder {
    /// The cylinder's rigid frame.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// The positive radius in model units.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
}

/// Untrimmed supporting geometry. A face separately owns its trim and sense.
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    /// A plane with no periodic parameter direction.
    Plane(Plane),
    /// A cylinder periodic in U with period `TAU`.
    Cylinder(Cylinder),
}

/// Surface position, first and second partials, and geometric unit normal.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SurfaceEvaluation {
    /// Position in the surface's coordinate system.
    pub point: [f64; 3],
    /// Partial derivative with respect to U.
    pub du: [f64; 3],
    /// Partial derivative with respect to V.
    pub dv: [f64; 3],
    /// Second partial with respect to U twice.
    pub duu: [f64; 3],
    /// Mixed U/V partial.
    pub duv: [f64; 3],
    /// Second partial with respect to V twice.
    pub dvv: [f64; 3],
    /// Normal of the supporting geometry; face orientation is applied separately.
    pub normal: [f64; 3],
}

impl Surface {
    /// Creates a plane from a finite right-handed orthonormal frame.
    ///
    /// # Errors
    /// Returns [`GeometryError::Frame`] for an invalid rigid frame.
    pub fn plane(frame: Placement3) -> Result<Self, GeometryError> {
        rigid(frame)?;
        Ok(Self::Plane(Plane { frame }))
    }

    /// Creates a cylinder with positive finite radius in a checked rigid frame.
    ///
    /// # Errors
    /// Returns a frame violation or [`GeometryError::Radius`].
    pub fn cylinder(frame: Placement3, radius: f64) -> Result<Self, GeometryError> {
        rigid(frame)?;
        check_radius(radius)?;
        Ok(Self::Cylinder(Cylinder { frame, radius }))
    }

    /// The supporting geometry's rigid frame.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        match self {
            Self::Plane(p) => p.frame,
            Self::Cylinder(c) => c.frame,
        }
    }

    /// Parameter periods in U/V, independently of any face's bounded domain.
    #[must_use]
    pub fn periods(&self) -> [Option<f64>; 2] {
        match self {
            Self::Plane(_) => [None, None],
            Self::Cylinder(_) => [Some(core::f64::consts::TAU), None],
        }
    }

    /// Evaluates the supporting surface without trim classification or clamping.
    /// Returns `None` for non-finite input or an overflowed result.
    #[must_use]
    pub fn evaluate(&self, uv: [f64; 2]) -> Option<SurfaceEvaluation> {
        if !uv.into_iter().all(f64::is_finite) {
            return None;
        }
        let frame = self.frame();
        let (point, du, duu, normal) = match self {
            Self::Plane(_) => (
                [uv[0], uv[1], 0.0],
                [1.0, 0.0, 0.0],
                [0.0; 3],
                [0.0, 0.0, 1.0],
            ),
            Self::Cylinder(c) => {
                let (sin, cos) = (libm::sin(uv[0]), libm::cos(uv[0]));
                (
                    [c.radius * cos, c.radius * sin, uv[1]],
                    [-c.radius * sin, c.radius * cos, 0.0],
                    [-c.radius * cos, -c.radius * sin, 0.0],
                    [cos, sin, 0.0],
                )
            }
        };
        let result = SurfaceEvaluation {
            point: frame.transform_point(point),
            du: vector(frame, du),
            dv: vector(
                frame,
                if matches!(self, Self::Plane(_)) {
                    [0.0, 1.0, 0.0]
                } else {
                    [0.0, 0.0, 1.0]
                },
            ),
            duu: vector(frame, duu),
            duv: [0.0; 3],
            dvv: [0.0; 3],
            normal: vector(frame, normal),
        };
        [
            result.point,
            result.du,
            result.dv,
            result.duu,
            result.duv,
            result.dvv,
            result.normal,
        ]
        .into_iter()
        .all(finite)
        .then_some(result)
    }
}

/// A violated geometry-construction invariant, including the supplied values.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum GeometryError {
    /// A point or vector contains a non-finite component.
    NonFinite,
    /// A line's direction has no nonzero component.
    ZeroDirection,
    /// A radius must be positive and finite.
    Radius {
        /// The supplied radius.
        radius: f64,
    },
    /// A finite increasing interval with finite span was required.
    Range {
        /// Supplied lower bound.
        min: f64,
        /// Supplied upper bound.
        max: f64,
    },
    /// The placement violates the rigid-frame contract.
    Frame(FrameError),
}

impl core::fmt::Display for GeometryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonFinite => write!(f, "geometry coordinates must be finite"),
            Self::ZeroDirection => write!(f, "line direction must be nonzero"),
            Self::Radius { radius } => write!(f, "radius {radius} must be positive and finite"),
            Self::Range { min, max } => write!(
                f,
                "parameter bounds [{min}, {max}] need a finite positive span"
            ),
            Self::Frame(error) => write!(f, "geometry frame: {error}"),
        }
    }
}
impl core::error::Error for GeometryError {}

pub(crate) fn rigid(frame: Placement3) -> Result<(), GeometryError> {
    let column = |axis: usize| frame.rows.map(|row| row[axis]);
    Placement3::try_from_orthonormal_axes(
        column(0),
        column(1),
        column(2),
        column(3),
        FRAME_TOLERANCE,
    )
    .map(|_| ())
    .map_err(GeometryError::Frame)
}
pub(crate) fn vector(frame: Placement3, p: [f64; 3]) -> [f64; 3] {
    frame
        .rows
        .map(|row| row[0] * p[0] + row[1] * p[1] + row[2] * p[2])
}
fn finite(p: [f64; 3]) -> bool {
    p.into_iter().all(f64::is_finite)
}
fn check_radius(radius: f64) -> Result<(), GeometryError> {
    if radius.is_finite() && radius > 0.0 {
        Ok(())
    } else {
        Err(GeometryError::Radius { radius })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3], tolerance: f64) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < tolerance, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn cylinder_derivatives_have_radian_and_length_units_in_a_placed_frame() {
        let frame = Placement3::rotate_y_then_translate(0.7, 2.0, -3.0, 4.0);
        let surface = Surface::cylinder(frame, 10.0).unwrap();
        let uv = [0.4, 2.0];
        let e = surface.evaluate(uv).unwrap();
        let speed = libm::hypot(libm::hypot(e.du[0], e.du[1]), e.du[2]);
        assert!((speed - 10.0).abs() < 1e-12);
        assert_eq!(surface.periods(), [Some(core::f64::consts::TAU), None]);
        let h = 1e-5;
        for axis in 0..2 {
            let mut lo = uv;
            lo[axis] -= h;
            let mut hi = uv;
            hi[axis] += h;
            let (lo, hi) = (surface.evaluate(lo).unwrap(), surface.evaluate(hi).unwrap());
            close(
                core::array::from_fn(|i| (hi.point[i] - lo.point[i]) / (2.0 * h)),
                if axis == 0 { e.du } else { e.dv },
                1e-8,
            );
            close(
                core::array::from_fn(|i| (hi.du[i] - lo.du[i]) / (2.0 * h)),
                if axis == 0 { e.duu } else { e.duv },
                1e-8,
            );
        }
        close(
            e.normal,
            vector(frame, [libm::cos(uv[0]), libm::sin(uv[0]), 0.0]),
            1e-12,
        );
    }

    #[test]
    fn supporting_geometry_is_independent_of_a_face_or_profile() {
        let line = Curve::line([3.0, 2.0, 1.0], [4.0, 0.0, 0.0]).unwrap();
        assert_eq!(line.evaluate(0.5).unwrap().point, [5.0, 2.0, 1.0]);
        assert_eq!(line.evaluate(2.0).unwrap().point, [11.0, 2.0, 1.0]);
        let circle = Curve::circle(Placement3::translate(0.0, 0.0, 7.0), 2.0).unwrap();
        close(
            circle.evaluate(core::f64::consts::FRAC_PI_2).unwrap().point,
            [0.0, 2.0, 7.0],
            1e-12,
        );
        let plane = Surface::plane(Placement3::IDENTITY).unwrap();
        assert_eq!(plane.evaluate([-2.0, 8.0]).unwrap().point, [-2.0, 8.0, 0.0]);
        assert!(plane.evaluate([f64::NAN, 0.0]).is_none());
        assert!(line.evaluate(f64::INFINITY).is_none());
    }

    #[test]
    fn invalid_inputs_fail_before_geometry_is_created() {
        assert_eq!(
            Curve::line([0.0; 3], [0.0; 3]),
            Err(GeometryError::ZeroDirection)
        );
        assert!(matches!(
            Surface::cylinder(Placement3::IDENTITY, -1.0),
            Err(GeometryError::Radius { radius: -1.0 })
        ));
        assert!(
            Surface::plane(Placement3::from_axes(
                [2.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0; 3]
            ))
            .is_err()
        );
        for (min, max) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (-f64::MAX, f64::MAX),
            (0.0, f64::INFINITY),
        ] {
            assert!(matches!(
                ParameterRange::new(min, max),
                Err(GeometryError::Range { .. })
            ));
        }
        let range = ParameterRange::new(-2.0, 3.0).unwrap();
        assert_eq!(range.at(0.0), Some(-2.0));
        assert_eq!(range.at(1.0), Some(3.0));
        assert_eq!(range.at(0.5), Some(0.5));
        assert_eq!(range.at(f64::NAN), None);
    }
}
