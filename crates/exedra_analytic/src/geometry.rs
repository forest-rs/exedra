// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-independent f64 curves, surfaces and parameter domains.
//!
//! Circle and cylinder angles are radians; plane coordinates and cylinder
//! height are model lengths. Derivatives are with respect to those parameters,
//! without normalization. Ruled patches use normalized unit parameters and
//! certify that bounded square's regularity. Other supporting geometry is
//! untrimmed: body topology owns
//! the bounded portions and their oriented uses. Texture coordinates are not
//! surface parameters.

use exedra_math::{FrameError, Placement3};

pub mod intersection;
mod ruled;
mod sphere;
pub use ruled::{CurveSpan, Ruled};
pub use sphere::Sphere;
pub(crate) use sphere::{
    cone_generator_gap, cone_pole_bounds, cone_pole_gap, harmonic_gap as sphere_harmonic_gap,
    pole_gap as sphere_pole_gap,
};

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

/// A periodic ellipse in a rigid frame's XY plane.
/// Its parameter is radians: `(a cos(t), b sin(t), 0)`, with `a >= b > 0`.
#[derive(Clone, Debug, PartialEq)]
pub struct Ellipse {
    frame: Placement3,
    semi_major: f64,
    semi_minor: f64,
}
impl Ellipse {
    /// Rigid frame whose X/Y axes are the major/minor directions.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Positive finite major semiaxis, in model units.
    #[must_use]
    pub const fn semi_major(&self) -> f64 {
        self.semi_major
    }
    /// Positive finite minor semiaxis, in model units.
    #[must_use]
    pub const fn semi_minor(&self) -> f64 {
        self.semi_minor
    }
    /// Angle of a point's scaled projection onto the ellipse frame, in `[-PI, PI]`.
    /// This does not classify the point against the ellipse or its plane.
    /// Returns `None` for non-finite arithmetic or a zero radial projection.
    #[must_use]
    pub fn parameter(&self, point: [f64; 3]) -> Option<f64> {
        let local = frame_coordinates(self.frame, point)?;
        let x = local[0] / self.semi_major;
        let y = local[1] / self.semi_minor;
        (x.is_finite() && y.is_finite() && (x != 0.0 || y != 0.0)).then(|| libm::atan2(y, x))
    }
}

/// An untrimmed, validated 3D curve independent of any construction operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// An unbounded line.
    Line(Line),
    /// A periodic circle; its period is `TAU` radians.
    Circle(Circle),
    /// A periodic ellipse; its period is `TAU` radians.
    Ellipse(Ellipse),
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

    /// Creates an ellipse in a rigid frame, with ordered positive finite semiaxes.
    /// Equality is allowed and represents a circle with ellipse parameterization.
    /// # Errors
    /// Returns a frame violation or [`GeometryError::EllipseAxes`].
    pub fn ellipse(
        frame: Placement3,
        semi_major: f64,
        semi_minor: f64,
    ) -> Result<Self, GeometryError> {
        rigid(frame)?;
        if !semi_major.is_finite()
            || !semi_minor.is_finite()
            || semi_major < semi_minor
            || semi_minor <= 0.0
        {
            return Err(GeometryError::EllipseAxes {
                semi_major,
                semi_minor,
            });
        }
        Ok(Self::Ellipse(Ellipse {
            frame,
            semi_major,
            semi_minor,
        }))
    }

    pub(crate) fn harmonic(&self) -> Option<(Placement3, [f64; 2])> {
        match self {
            Self::Line(_) => None,
            Self::Circle(c) => Some((c.frame(), [c.radius(); 2])),
            Self::Ellipse(e) => Some((e.frame(), [e.semi_major(), e.semi_minor()])),
        }
    }

    pub(crate) fn sampling_radius(&self) -> Option<f64> {
        match self {
            Self::Line(_) => None,
            Self::Circle(c) => Some(c.radius()),
            Self::Ellipse(e) => Some(e.semi_major() * (1.0 + 4.0 * FRAME_TOLERANCE)),
        }
    }

    /// The parameter period; lines are not periodic.
    #[must_use]
    pub fn period(&self) -> Option<f64> {
        match self {
            Self::Line(_) => None,
            Self::Circle(_) | Self::Ellipse(_) => Some(core::f64::consts::TAU),
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
            Self::Ellipse(ellipse) => {
                let (sin, cos) = (libm::sin(parameter), libm::cos(parameter));
                let (a, b) = (ellipse.semi_major, ellipse.semi_minor);
                CurveEvaluation {
                    point: ellipse.frame.transform_point([a * cos, b * sin, 0.0]),
                    first: vector(ellipse.frame, [-a * sin, b * cos, 0.0]),
                    second: vector(ellipse.frame, [-a * cos, -b * sin, 0.0]),
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

/// One nappe of a cone about a rigid frame's Z axis.
///
/// U is radians and V is axial height. The radius is `radius + slope * V`;
/// only the strictly positive-radius portion is regular and evaluable.
#[derive(Clone, Debug, PartialEq)]
pub struct Cone {
    frame: Placement3,
    radius: f64,
    slope: f64,
}
impl Cone {
    /// The rigid frame, with the cone axis along local +Z.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Positive radius at axial height zero.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
    /// Signed change in radius per unit axial height.
    #[must_use]
    pub const fn slope(&self) -> f64 {
        self.slope
    }
    /// Signed angle between a generator and the local +Z axis, in radians.
    ///
    /// Its tangent is the radial slope. Zero denotes cylindrical geometry.
    /// For extreme finite slopes, rounding can produce exactly `±PI/2`;
    /// consumers requiring a regular angle representation must check that limit.
    #[must_use]
    pub fn semi_angle(&self) -> f64 {
        libm::atan(self.slope)
    }
    /// Canonical finite axial parameter of the apex, when this is not a cylinder.
    /// This is a position-only limit; [`Self::radius_at`] remains restricted to
    /// the regular positive-radius nappe. Division rounding is included in body
    /// collapsed-boundary agreement checks.
    #[must_use]
    pub fn apex_parameter(&self) -> Option<f64> {
        let v = -self.radius / self.slope;
        (self.slope != 0.0 && v.is_finite()).then_some(v)
    }
    /// Finite apex position without inventing a unique apex normal.
    #[must_use]
    pub fn apex_point(&self) -> Option<[f64; 3]> {
        let point = self
            .frame
            .transform_point([0.0, 0.0, self.apex_parameter()?]);
        finite(point).then_some(point)
    }
    /// Radius at a height, or `None` at/past the apex or on overflow.
    #[must_use]
    pub fn radius_at(&self, height: f64) -> Option<f64> {
        if Some(height) == self.apex_parameter() {
            return None;
        }
        let radius = self.radius + self.slope * height;
        (radius.is_finite() && radius > 0.0).then_some(radius)
    }
}

/// A regular ring torus about a rigid frame's Z axis, with both angles in radians.
/// Major radius is strictly larger than the positive minor radius, so the whole
/// supporting surface is regular. Horns, spindles and poles are excluded.
#[derive(Clone, Debug, PartialEq)]
pub struct Torus {
    frame: Placement3,
    major_radius: f64,
    minor_radius: f64,
}
impl Torus {
    /// The rigid frame, with the major circle in local XY.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Radius from the axis to the center of the generating circle.
    #[must_use]
    pub const fn major_radius(&self) -> f64 {
        self.major_radius
    }
    /// Radius of the generating circle.
    #[must_use]
    pub const fn minor_radius(&self) -> f64 {
        self.minor_radius
    }
}

/// Supporting geometry. A face separately owns its trim and sense.
/// Ruled patches have a certified unit domain; other families are untrimmed
/// within their documented regular regions.
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    /// A plane with no periodic parameter direction.
    Plane(Plane),
    /// A cylinder periodic in U with period `TAU`.
    Cylinder(Cylinder),
    /// A cone periodic in U, restricted to its regular positive-radius nappe.
    Cone(Cone),
    /// A regular ring torus, periodic in both U and V with period `TAU`.
    Torus(Torus),
    /// A sphere, periodic in U, with latitude V in `[-PI/2, PI/2]`.
    /// Its coordinate poles have unique geometric normals.
    Sphere(Sphere),
    /// A certified regular unit patch interpolating two bounded source curves.
    Ruled(Ruled),
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
    /// Creates a ruled unit patch between bounded line/circle curves.
    /// Coordinates are body-local; U follows each normalized source traversal
    /// and V interpolates between them. Evaluation is restricted to `[0,1]²`.
    ///
    /// # Errors
    /// Returns [`GeometryError::NonRegularRuled`] if bounded interval analysis
    /// cannot certify a nonzero normal throughout the patch.
    pub fn ruled(a: CurveSpan, b: CurveSpan) -> Result<Self, GeometryError> {
        Ruled::new(a, b).map(Self::Ruled)
    }
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

    /// Creates a cone with positive base radius and a finite radial slope.
    ///
    /// A zero slope is permitted and has cylindrical geometry. Evaluation
    /// refuses heights at or past the apex; body domains must avoid it.
    ///
    /// # Errors
    /// Returns a frame violation, [`GeometryError::Radius`] or
    /// [`GeometryError::NonFinite`] for a non-finite slope.
    pub fn cone(frame: Placement3, radius: f64, slope: f64) -> Result<Self, GeometryError> {
        rigid(frame)?;
        check_radius(radius)?;
        if !slope.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        Ok(Self::Cone(Cone {
            frame,
            radius,
            slope,
        }))
    }

    /// Creates spherical supporting geometry with a positive finite radius.
    /// Longitude U is periodic; latitude V is in `[-PI/2, PI/2]`.
    /// # Errors
    /// Returns a checked frame violation or [`GeometryError::Radius`].
    pub fn sphere(frame: Placement3, radius: f64) -> Result<Self, GeometryError> {
        Sphere::new(frame, radius).map(Self::Sphere)
    }

    /// Creates a regular ring torus: `major_radius > minor_radius > 0`.
    ///
    /// `P(u,v) = ((R+r*cos(v))*cos(u), (R+r*cos(v))*sin(u), r*sin(v))`
    /// in the supplied frame. The outward normal follows `du × dv`.
    ///
    /// # Errors
    /// Returns a frame/radius violation or [`GeometryError::TorusRadii`] for
    /// a nonregular torus or an overflowing outer radius.
    pub fn torus(
        frame: Placement3,
        major_radius: f64,
        minor_radius: f64,
    ) -> Result<Self, GeometryError> {
        rigid(frame)?;
        check_radius(major_radius)?;
        check_radius(minor_radius)?;
        if major_radius <= minor_radius || !(major_radius + minor_radius).is_finite() {
            return Err(GeometryError::TorusRadii {
                major_radius,
                minor_radius,
            });
        }
        Ok(Self::Torus(Torus {
            frame,
            major_radius,
            minor_radius,
        }))
    }

    /// The supporting geometry's rigid frame.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        match self {
            Self::Plane(p) => p.frame,
            Self::Cylinder(c) => c.frame,
            Self::Cone(c) => c.frame,
            Self::Torus(t) => t.frame,
            Self::Sphere(s) => s.frame(),
            Self::Ruled(_) => Placement3::IDENTITY,
        }
    }

    /// Parameter periods in U/V, independently of any face's bounded domain.
    #[must_use]
    pub fn periods(&self) -> [Option<f64>; 2] {
        match self {
            Self::Plane(_) | Self::Ruled(_) => [None, None],
            Self::Cylinder(_) | Self::Cone(_) | Self::Sphere(_) => {
                [Some(core::f64::consts::TAU), None]
            }
            Self::Torus(_) => [Some(core::f64::consts::TAU); 2],
        }
    }

    /// Parameters of the closest point on the untrimmed supporting geometry.
    ///
    /// Cylinder/cone U is the principal angle in `[-pi,pi]`. Cone V is the
    /// orthogonal meridian projection; a closest point at the apex returns
    /// `None` because U has no unique value. A point on the axis
    /// has no unique closest angle and returns `None`, as do non-finite inputs
    /// and overflow. Neither a face domain nor its trims are classified.
    /// Spheres use radial projection in their defining frame and return `None`
    /// on the polar axis, where longitude is not unique. Ruled patches return
    /// `None`: a closest-point inverse is not provided.
    #[must_use]
    pub fn parameters(&self, point: [f64; 3]) -> Option<[f64; 2]> {
        if let Self::Sphere(sphere) = self {
            return sphere.parameters(point);
        }
        if !finite(point) {
            return None;
        }
        let frame = self.frame();
        let origin = frame.rows.map(|r| r[3]);
        let delta = core::array::from_fn::<_, 3, _>(|i| point[i] - origin[i]);
        let local = core::array::from_fn::<_, 3, _>(|i| {
            delta[0] * frame.rows[0][i] + delta[1] * frame.rows[1][i] + delta[2] * frame.rows[2][i]
        });
        if !finite(local) {
            return None;
        }
        match self {
            Self::Ruled(_) | Self::Sphere(_) => None,
            Self::Plane(_) => Some([local[0], local[1]]),
            Self::Torus(t) => {
                let radial = libm::hypot(local[0], local[1]);
                let offset = radial - t.major_radius;
                if radial == 0.0 || !radial.is_finite() || (offset == 0.0 && local[2] == 0.0) {
                    return None;
                }
                Some([
                    libm::atan2(local[1], local[0]),
                    libm::atan2(local[2], offset),
                ])
            }
            Self::Cylinder(_) | Self::Cone(_) => {
                let radial = libm::hypot(local[0], local[1]);
                if radial == 0.0 || !radial.is_finite() {
                    return None;
                }
                let height = if let Self::Cone(cone) = self {
                    let norm = libm::hypot(1.0, cone.slope);
                    let (sin, cos) = (cone.slope / norm, 1.0 / norm);
                    let height = cos * (cos * local[2] + sin * (radial - cone.radius));
                    cone.radius_at(height)?;
                    height
                } else {
                    local[2]
                };
                Some([libm::atan2(local[1], local[0]), height])
            }
        }
    }

    /// Evaluates without face-trim classification or parameter clamping.
    /// Ruled patches accept only their certified unit square. Spheres accept
    /// latitude in `[-PI/2, PI/2]`, including both coordinate poles; their normal
    /// remains unique although the U derivative vanishes there.
    /// Returns `None` for non-finite input or an overflowed result.
    #[must_use]
    pub fn evaluate(&self, uv: [f64; 2]) -> Option<SurfaceEvaluation> {
        if !uv.into_iter().all(f64::is_finite) {
            return None;
        }
        if let Self::Ruled(ruled) = self {
            return ruled.evaluation(uv);
        }
        if let Self::Sphere(sphere) = self {
            return sphere.evaluation(uv);
        }
        let frame = self.frame();
        let (point, du, dv, duu, duv, dvv, normal) = match self {
            Self::Plane(_) => (
                [uv[0], uv[1], 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                [0.0, 0.0, 1.0],
            ),
            Self::Cylinder(_) | Self::Cone(_) => {
                let (radius, slope) = match self {
                    Self::Cylinder(c) => (c.radius, 0.0),
                    Self::Cone(c) => (c.radius_at(uv[1])?, c.slope),
                    Self::Plane(_) | Self::Torus(_) | Self::Sphere(_) | Self::Ruled(_) => {
                        unreachable!()
                    }
                };
                let (sin, cos) = (libm::sin(uv[0]), libm::cos(uv[0]));
                let norm = libm::hypot(1.0, slope);
                (
                    [radius * cos, radius * sin, uv[1]],
                    [-radius * sin, radius * cos, 0.0],
                    [slope * cos, slope * sin, 1.0],
                    [-radius * cos, -radius * sin, 0.0],
                    [-slope * sin, slope * cos, 0.0],
                    [0.0; 3],
                    [cos / norm, sin / norm, -slope / norm],
                )
            }
            Self::Torus(t) => {
                let (su, cu) = (libm::sin(uv[0]), libm::cos(uv[0]));
                let (sv, cv) = (libm::sin(uv[1]), libm::cos(uv[1]));
                let r = t.minor_radius;
                let radial = t.major_radius + r * cv;
                (
                    [radial * cu, radial * su, r * sv],
                    [-radial * su, radial * cu, 0.0],
                    [-r * sv * cu, -r * sv * su, r * cv],
                    [-radial * cu, -radial * su, 0.0],
                    [r * sv * su, -r * sv * cu, 0.0],
                    [-r * cv * cu, -r * cv * su, -r * sv],
                    [cv * cu, cv * su, sv],
                )
            }
            Self::Ruled(_) | Self::Sphere(_) => unreachable!(),
        };
        let result = SurfaceEvaluation {
            point: frame.transform_point(point),
            du: vector(frame, du),
            dv: vector(frame, dv),
            duu: vector(frame, duu),
            duv: vector(frame, duv),
            dvv: vector(frame, dvv),
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
    /// A ruled patch's regularity could not be certified on this U interval.
    NonRegularRuled {
        /// Normalized source interval where the conservative check stopped.
        interval: [f64; 2],
    },
    /// A point or vector contains a non-finite component.
    NonFinite,
    /// A line's direction has no nonzero component.
    ZeroDirection,
    /// A radius must be positive and finite.
    Radius {
        /// The supplied radius.
        radius: f64,
    },
    /// An ellipse requires finite `semi_major >= semi_minor > 0`.
    EllipseAxes {
        /// Supplied major semiaxis.
        semi_major: f64,
        /// Supplied minor semiaxis.
        semi_minor: f64,
    },
    /// A regular torus needs `major_radius > minor_radius` and a finite sum.
    TorusRadii {
        /// Supplied major radius.
        major_radius: f64,
        /// Supplied minor radius.
        minor_radius: f64,
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
            Self::NonRegularRuled { interval } => write!(
                f,
                "ruled surface regularity could not be certified on {interval:?}"
            ),
            Self::NonFinite => write!(f, "geometry coordinates must be finite"),
            Self::ZeroDirection => write!(f, "line direction must be nonzero"),
            Self::Radius { radius } => write!(f, "radius {radius} must be positive and finite"),
            Self::EllipseAxes {
                semi_major,
                semi_minor,
            } => write!(
                f,
                "ellipse semiaxes {semi_major}, {semi_minor} require finite major >= minor > 0"
            ),
            Self::TorusRadii {
                major_radius,
                minor_radius,
            } => write!(
                f,
                "torus radii {major_radius}, {minor_radius} require a regular ring and finite outer radius"
            ),
            Self::Range { min, max } => write!(
                f,
                "parameter bounds [{min}, {max}] need a finite positive span"
            ),
            Self::Frame(error) => write!(f, "geometry frame: {error}"),
        }
    }
}
impl core::error::Error for GeometryError {}

pub(crate) fn frame_coordinates(frame: Placement3, point: [f64; 3]) -> Option<[f64; 3]> {
    let axis = |i| frame.rows.map(|r| r[i]);
    let delta = exedra_math::sub(point, axis(3));
    let yz = exedra_math::cross(axis(1), axis(2));
    let zx = exedra_math::cross(axis(2), axis(0));
    let xy = exedra_math::cross(axis(0), axis(1));
    let det = exedra_math::dot(axis(0), yz);
    let values = [yz, zx, xy].map(|v| exedra_math::dot(delta, v) / det);
    (det.is_finite() && det != 0.0 && values.into_iter().all(f64::is_finite)).then_some(values)
}

pub(crate) fn ellipse_support_gap(ellipse: &Ellipse, surface: &Surface) -> f64 {
    use exedra_math::{add, cross, dot, scale, sub};
    let norm = |v: [f64; 3]| libm::hypot(libm::hypot(v[0], v[1]), v[2]);
    let ef = ellipse.frame();
    let sf = surface.frame();
    let center = ef.rows.map(|r| r[3]);
    let a = ef.rows.map(|r| r[0] * ellipse.semi_major());
    let b = ef.rows.map(|r| r[1] * ellipse.semi_minor());
    let bound = match surface {
        Surface::Plane(_) => {
            let normal = cross(sf.rows.map(|r| r[0]), sf.rows.map(|r| r[1]));
            let length = norm(normal);
            let normal = scale(normal, 1.0 / length);
            dot(sub(center, sf.rows.map(|r| r[3])), normal).abs()
                + libm::hypot(dot(a, normal), dot(b, normal))
        }
        Surface::Cylinder(c) => {
            let Some(center) = frame_coordinates(sf, center) else {
                return f64::INFINITY;
            };
            let zero = Placement3::from_axes(
                sf.rows.map(|r| r[0]),
                sf.rows.map(|r| r[1]),
                sf.rows.map(|r| r[2]),
                [0.0; 3],
            );
            let Some(a) = frame_coordinates(zero, a) else {
                return f64::INFINITY;
            };
            let Some(b) = frame_coordinates(zero, b) else {
                return f64::INFINITY;
            };
            let length = libm::hypot(a[0], a[1]);
            if length == 0.0 || !length.is_finite() {
                return f64::INFINITY;
            }
            let unit = [a[0] / length, a[1] / length];
            let sign = if unit[0] * b[1] - unit[1] * b[0] < 0.0 {
                -1.0
            } else {
                1.0
            };
            let ideal_a = [c.radius() * unit[0], c.radius() * unit[1]];
            let ideal_b = [-sign * c.radius() * unit[1], sign * c.radius() * unit[0]];
            let radial = |x: f64, y: f64| {
                add(
                    scale(sf.rows.map(|r| r[0]), x),
                    scale(sf.rows.map(|r| r[1]), y),
                )
            };
            // Compare whole forward coefficient triples in world coordinates.
            // An approximate orthonormal transpose is not an inverse here.
            norm(radial(center[0], center[1]))
                + norm(radial(a[0] - ideal_a[0], a[1] - ideal_a[1]))
                + norm(radial(b[0] - ideal_b[0], b[1] - ideal_b[1]))
        }
        Surface::Sphere(sphere) => sphere::harmonic_gap(sphere, center, a, b),
        Surface::Cone(_) | Surface::Torus(_) | Surface::Ruled(_) => f64::INFINITY,
    };
    let magnitude = ef
        .rows
        .iter()
        .chain(&sf.rows)
        .map(|r| r[3].abs())
        .fold(ellipse.semi_major(), f64::max);
    bound + 256.0 * f64::EPSILON * magnitude
}

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
    frame.transform_vector(p)
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
    fn inverse_support_parameters_use_radians_without_trim_or_domain_clamping() {
        let frame = Placement3::rotate_y_then_translate(0.7, 2.0, -3.0, 4.0);
        for surface in [
            Surface::plane(frame).unwrap(),
            Surface::cylinder(frame, 7.0).unwrap(),
        ] {
            let uv = [-1.2, 8.0];
            let sample = surface.evaluate(uv).unwrap();
            let found = surface.parameters(sample.point).unwrap();
            assert!((found[0] - uv[0]).abs() < 1e-12);
            assert!((found[1] - uv[1]).abs() < 1e-12);
        }
        let cylinder = Surface::cylinder(Placement3::IDENTITY, 7.0).unwrap();
        assert_eq!(cylinder.parameters([0.0, 0.0, 3.0]), None);
        assert_eq!(cylinder.parameters([f64::NAN, 0.0, 0.0]), None);
        assert_eq!(cylinder.parameters([14.0, 0.0, 3.0]), Some([0.0, 3.0]));
    }

    #[test]
    fn cone_partials_and_closest_parameters_match_independent_differences() {
        let frame = Placement3::rotate_y_then_translate(0.7, 2.0, -3.0, 4.0);
        for slope in [-0.8, 0.0, 0.4] {
            let cone = Surface::cone(frame, 5.0, slope).unwrap();
            let uv = [0.7, 1.0];
            let e = cone.evaluate(uv).unwrap();
            let h = 1e-5;
            for axis in 0..2 {
                let mut a = uv;
                a[axis] -= h;
                let mut b = uv;
                b[axis] += h;
                let (a, b) = (cone.evaluate(a).unwrap(), cone.evaluate(b).unwrap());
                close(
                    core::array::from_fn(|i| (b.point[i] - a.point[i]) / (2.0 * h)),
                    if axis == 0 { e.du } else { e.dv },
                    1e-8,
                );
                close(
                    core::array::from_fn(|i| (b.du[i] - a.du[i]) / (2.0 * h)),
                    if axis == 0 { e.duu } else { e.duv },
                    1e-8,
                );
            }
            assert!(
                (libm::hypot(libm::hypot(e.normal[0], e.normal[1]), e.normal[2]) - 1.0).abs()
                    < 1e-12
            );
            let found = cone.parameters(e.point).unwrap();
            assert!((found[0] - uv[0]).abs() < 1e-12 && (found[1] - uv[1]).abs() < 1e-12);
            let off = core::array::from_fn(|i| e.point[i] + 0.25 * e.normal[i]);
            let found = cone.parameters(off).unwrap();
            close(cone.evaluate(found).unwrap().point, e.point, 1e-12);
        }
        let cone = Surface::cone(Placement3::IDENTITY, 1.0, -0.5).unwrap();
        assert!(cone.evaluate([0.0, 2.0]).is_none());
        assert!(cone.evaluate([0.0, 3.0]).is_none());
        assert!(cone.parameters([0.0; 3]).is_none());
        assert!(Surface::cone(Placement3::IDENTITY, 1.0, f64::INFINITY).is_err());
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

    #[test]
    fn torus_has_two_periodic_angles_and_checked_first_and_second_partials() {
        let frame = Placement3::rotate_y_then_translate(0.6, 2.0, -3.0, 4.0);
        let surface = Surface::torus(frame, 10.0, 2.0).unwrap();
        assert_eq!(surface.periods(), [Some(core::f64::consts::TAU); 2]);
        for uv in [[0.7, 1.2], [-2.0, -2.5], [0.0, core::f64::consts::PI]] {
            let e = surface.evaluate(uv).unwrap();
            let h = 1e-5;
            for axis in 0..2 {
                let mut a = uv;
                a[axis] -= h;
                let mut b = uv;
                b[axis] += h;
                let (a, b) = (surface.evaluate(a).unwrap(), surface.evaluate(b).unwrap());
                close(
                    core::array::from_fn(|i| (b.point[i] - a.point[i]) / (2.0 * h)),
                    if axis == 0 { e.du } else { e.dv },
                    1e-8,
                );
                close(
                    core::array::from_fn(|i| (b.du[i] - a.du[i]) / (2.0 * h)),
                    if axis == 0 { e.duu } else { e.duv },
                    1e-8,
                );
                close(
                    core::array::from_fn(|i| (b.dv[i] - a.dv[i]) / (2.0 * h)),
                    if axis == 0 { e.duv } else { e.dvv },
                    1e-8,
                );
            }
            let found = surface.parameters(e.point).unwrap();
            close(surface.evaluate(found).unwrap().point, e.point, 1e-12);
            let off = exedra_math::add(e.point, exedra_math::scale(e.normal, 0.25));
            close(
                surface
                    .evaluate(surface.parameters(off).unwrap())
                    .unwrap()
                    .point,
                e.point,
                1e-12,
            );
            close(
                surface
                    .evaluate([
                        uv[0] + core::f64::consts::TAU,
                        uv[1] - core::f64::consts::TAU,
                    ])
                    .unwrap()
                    .point,
                e.point,
                1e-12,
            );
        }
        let surface = Surface::torus(Placement3::IDENTITY, 10.0, 2.0).unwrap();
        assert!(surface.parameters([0.0, 0.0, 1.0]).is_none());
        assert!(surface.parameters([10.0, 0.0, 0.0]).is_none());
        assert!(Surface::torus(Placement3::IDENTITY, 2.0, 2.0).is_err());
        assert!(Surface::torus(Placement3::IDENTITY, 1.0, 2.0).is_err());
    }
}

#[cfg(test)]
mod cone_angle_tests {
    use super::*;
    #[test]
    fn signed_angle_preserves_cylindrical_and_rounded_limit_cases() {
        for slope in [0.0, -0.0, 0.5, -0.5, 1.0, -1.0, f64::MAX, -f64::MAX] {
            let Surface::Cone(cone) = Surface::cone(Placement3::IDENTITY, 2.0, slope).unwrap()
            else {
                unreachable!()
            };
            let angle = cone.semi_angle();
            assert_eq!(angle.is_sign_negative(), slope.is_sign_negative());
            assert!(angle.abs() <= core::f64::consts::FRAC_PI_2);
            if slope.abs() <= 1.0 {
                assert!((libm::tan(angle) - slope).abs() <= f64::EPSILON);
            }
        }
    }
}
