// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Texture-coordinate mapping independent of mesh ownership and image resources.
//!
//! Coordinates are dimensionless repeats; they are never clamped or wrapped.
//! Built-in primary and secondary corner channels share this math. Texture-specific
//! transforms and image sampling remain the responsibility of the consumer.

#[cfg(not(feature = "libm"))]
use crate::math::FloatExt;
use crate::{BoxPlane, DEFAULT_BOX_NORMAL_EPSILON, box_plane_from_normal};
use alloc::vec::Vec;
use exedra_math::{Placement3, Real, cross, dot, normalize, sub};

/// Built-in corner UV channel. Texture references select the corresponding set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UvSet {
    /// Primary texture coordinates (`TEXCOORD_0`).
    #[default]
    Primary,
    /// Secondary texture coordinates (`TEXCOORD_1`).
    Secondary,
}
impl UvSet {
    /// The kernel attribute key carrying this channel.
    #[must_use]
    pub const fn key(self) -> crate::attributes::AttrKey<[f32; 2]> {
        match self {
            Self::Primary => crate::attr::CORNER_UV,
            Self::Secondary => crate::attr::CORNER_UV1,
        }
    }
}

/// How angular coordinates are lifted across a face's wrapping boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UvSeamPolicy {
    /// Keep every defined angular coordinate in `[0, 1)`.
    Preserve,
    /// Cut at the largest angular gap, minimizing each face's span. Values
    /// may leave `[0, 1]`; use a repeating sampler. Topology is never split.
    #[default]
    Continuous,
}

/// Recovery for angular coordinates undefined at a pole or projection axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UvPolePolicy {
    /// Refuse any undefined angle.
    Refuse,
    /// Use the mean of the face's defined angles, after seam correction.
    /// Refuse when an entire face has no defined angle on that axis.
    #[default]
    FaceAverage,
}

/// Explicit channel selection and angular recovery used by mesh authoring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UvMappingOptions {
    /// Channel read by existing-coordinate projection; ignored by other modes.
    pub source: UvSet,
    /// Channel to author.
    pub target: UvSet,
    /// Angular boundary policy.
    pub seams: UvSeamPolicy,
    /// Undefined angular-coordinate policy.
    pub poles: UvPolePolicy,
}

/// One corner's input to a projection, independent of mesh IDs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvInput {
    /// Mesh-space position; ignored by existing-coordinate projection.
    pub position: [f32; 3],
    /// Surface normal for box/per-face projection, of any length. A zero
    /// normal falls back to +Z. Other modes ignore this value.
    pub normal: [f64; 3],
    /// Authored source UVs, used only by existing-coordinate projection.
    pub existing: Option<[f32; 2]>,
}

/// A checked face mapping and evidence for its angular recovery.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvFaceMapping {
    /// Final UVs in the input corner order.
    pub uvs: Vec<[f32; 2]>,
    /// Corners whose defined angle was lifted by an integer turn.
    pub seam_corners_adjusted: u64,
    /// Corners with one or more undefined angles filled from face neighbors.
    pub pole_corners_repaired: u64,
    /// A degenerate surface normal used the +Z projection fallback.
    pub normal_fallback: bool,
    /// A periodic face still spans more than half a turn after correction.
    /// Such coarse faces may need subdivision or a different projection.
    pub wide_periodic_axes: [bool; 2],
}

/// A projection refusal with an index into the supplied face-corner slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UvFaceError {
    /// Offending zero-based input corner, or `None` for invalid parameters.
    pub corner: Option<usize>,
    /// The violated projection invariant.
    pub error: UvMappingError,
}
impl core::fmt::Display for UvFaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "UV input {:?}: {}", self.corner, self.error)
    }
}
impl core::error::Error for UvFaceError {}

/// Perspective projector in mesh coordinates, looking along `u × v`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerspectiveUv {
    /// Projector origin and orthonormal image axes.
    pub frame: UvFrame,
    /// Positive focal scales in UV units: `uv = 0.5 + focal * xy / depth`.
    /// For vertical field of view `a`, Y is `0.5 / tan(a/2)` and X is Y/aspect.
    pub focal: [f64; 2],
    /// Nonnegative near depth in mesh units. Corners at or behind it are
    /// refused; this mapping does not clip geometry or test visibility.
    pub near: f64,
}

/// A row-major affine mapping of UV coordinates.
///
/// `uv = matrix * source + offset`. Singular transforms, including zero scale,
/// are permitted. Evaluation uses `f64` and checks the final `f32` boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    /// Finite linear mapping, in texture repeats per source unit.
    pub matrix: [[f64; 2]; 2],
    /// Finite translation in texture repeats.
    pub offset: [f64; 2],
}

impl UvTransform {
    /// Unmodified coordinates.
    pub const IDENTITY: Self = Self {
        matrix: [[1.0, 0.0], [0.0, 1.0]],
        offset: [0.0; 2],
    };

    /// Builds `scale * (rotate(source) + offset)`.
    ///
    /// Rotation is counterclockwise about UV zero, in radians. Offset is
    /// deliberately applied before scale. Negative scales mirror an axis.
    pub fn rotate_offset_scale(
        radians: f64,
        offset: [f64; 2],
        scale: [f64; 2],
    ) -> Result<Self, UvMappingError> {
        if !radians.is_finite() || !offset.into_iter().chain(scale).all(f64::is_finite) {
            return Err(UvMappingError::InvalidTransform);
        }
        let (s, c) = sin_cos(radians);
        let result = Self {
            matrix: [[scale[0] * c, -scale[0] * s], [scale[1] * s, scale[1] * c]],
            offset: [scale[0] * offset[0], scale[1] * offset[1]],
        };
        result.validate()?;
        Ok(result)
    }

    /// Builds a translation in UV units.
    pub fn translation(offset: [f64; 2]) -> Result<Self, UvMappingError> {
        Self::rotate_offset_scale(0.0, offset, [1.0; 2])
    }
    /// Builds a rotation about UV zero, in radians.
    pub fn rotation(radians: f64) -> Result<Self, UvMappingError> {
        Self::rotate_offset_scale(radians, [0.0; 2], [1.0; 2])
    }
    /// Builds a nonuniform scale about UV zero.
    pub fn scale(scale: [f64; 2]) -> Result<Self, UvMappingError> {
        Self::rotate_offset_scale(0.0, [0.0; 2], scale)
    }
    /// Re-centers this transform on `pivot`: `pivot + M*(uv-pivot) + offset`.
    pub fn around_pivot(mut self, pivot: [f64; 2]) -> Result<Self, UvMappingError> {
        if !pivot.into_iter().all(f64::is_finite) {
            return Err(UvMappingError::InvalidTransform);
        }
        self.offset = core::array::from_fn(|i| {
            pivot[i] + self.offset[i] - self.matrix[i][0] * pivot[0] - self.matrix[i][1] * pivot[1]
        });
        self.validate()?;
        Ok(self)
    }
    /// Composes transforms in call order: `self.then(next)` applies `self` first.
    pub fn then(self, next: Self) -> Result<Self, UvMappingError> {
        self.validate()?;
        next.validate()?;
        let result = Self {
            matrix: core::array::from_fn(|i| {
                core::array::from_fn(|j| {
                    next.matrix[i][0] * self.matrix[0][j] + next.matrix[i][1] * self.matrix[1][j]
                })
            }),
            offset: core::array::from_fn(|i| {
                next.matrix[i][0] * self.offset[0]
                    + next.matrix[i][1] * self.offset[1]
                    + next.offset[i]
            }),
        };
        result.validate()?;
        Ok(result)
    }
    /// Fits positive finite source extents into the unit square. When preserving
    /// aspect, centers the shorter dimension and leaves unused space on its sides.
    pub fn fit_bounds(
        min: [f64; 2],
        max: [f64; 2],
        preserve_aspect: bool,
    ) -> Result<Self, UvMappingError> {
        let span = [max[0] - min[0], max[1] - min[1]];
        if !min.into_iter().chain(max).chain(span).all(f64::is_finite)
            || span.into_iter().any(|v| v <= 0.0)
        {
            return Err(UvMappingError::InvalidBounds);
        }
        let scale = if preserve_aspect {
            [1.0 / span[0].max(span[1]); 2]
        } else {
            span.map(|v| 1.0 / v)
        };
        let result = Self {
            matrix: [[scale[0], 0.0], [0.0, scale[1]]],
            offset: core::array::from_fn(|i| (1.0 - span[i] * scale[i]) * 0.5 - min[i] * scale[i]),
        };
        result.validate()?;
        Ok(result)
    }

    /// Checks that all coefficients are finite.
    pub fn validate(&self) -> Result<(), UvMappingError> {
        if self
            .matrix
            .iter()
            .flatten()
            .chain(&self.offset)
            .all(|v| v.is_finite())
        {
            Ok(())
        } else {
            Err(UvMappingError::InvalidTransform)
        }
    }

    /// Transforms coordinates, refusing nonfinite inputs and unrepresentable results.
    pub fn apply(&self, uv: [f64; 2]) -> Result<[f32; 2], UvMappingError> {
        self.validate()?;
        self.apply_coordinates(uv)
    }

    fn apply_coordinates(&self, uv: [f64; 2]) -> Result<[f32; 2], UvMappingError> {
        if !uv.into_iter().all(f64::is_finite) {
            return Err(UvMappingError::NumericLimit);
        }
        narrow(core::array::from_fn(|i| {
            self.matrix[i][0] * uv[0] + self.matrix[i][1] * uv[1] + self.offset[i]
        }))
    }
}

impl Default for UvTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// An orthonormal projection frame in mesh coordinates.
///
/// Projection is `[dot(position - origin, u), dot(position - origin, v)]`.
/// Rotate both axes to orient the plane freely in 3D.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvFrame {
    /// Finite projection origin in mesh units.
    pub origin: [f64; 3],
    /// Unit direction of increasing U.
    pub u: [f64; 3],
    /// Unit direction of increasing V, perpendicular to U.
    pub v: [f64; 3],
}

impl UvFrame {
    /// XY projection about the mesh origin.
    pub const XY: Self = Self {
        origin: [0.0; 3],
        u: [1.0, 0.0, 0.0],
        v: [0.0, 1.0, 0.0],
    };
    /// XZ projection about the mesh origin.
    pub const XZ: Self = Self {
        origin: [0.0; 3],
        u: [1.0, 0.0, 0.0],
        v: [0.0, 0.0, 1.0],
    };
    /// YZ projection about the mesh origin.
    pub const YZ: Self = Self {
        origin: [0.0; 3],
        u: [0.0, 1.0, 0.0],
        v: [0.0, 0.0, 1.0],
    };

    /// Constructs a deterministic frame from a finite nonzero normal.
    ///
    /// U is +X projected onto the plane, unless the unit normal's X component
    /// exceeds 0.9 in magnitude, when +Y is used. V is `normal × U`.
    /// This choice has a seam at that threshold; use explicit axes when a
    /// continuously varying orientation is required. Normal length is ignored.
    pub fn from_normal(origin: [f64; 3], normal: [f64; 3]) -> Result<Self, UvMappingError> {
        // Scale before normalization so very small/large finite inputs work.
        let magnitude = normal.into_iter().map(f64::abs).fold(0.0_f64, f64::max);
        if !normal.into_iter().all(f64::is_finite) || magnitude == 0.0 {
            return Err(UvMappingError::InvalidFrame);
        }
        let n = normalize(normal.map(|x| x / magnitude)).ok_or(UvMappingError::InvalidFrame)?;
        let seed = if n[0].abs() > 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let u = normalize(sub(seed, n.map(|x| x * dot(seed, n))))
            .ok_or(UvMappingError::InvalidFrame)?;
        let frame = Self {
            origin,
            u,
            v: cross(n, u),
        };
        frame.validate()?;
        Ok(frame)
    }

    /// Applies a rigid placement to the projection frame. Scaling and shearing
    /// are refused; use the UV transform for texture density and distortion.
    pub fn placed(self, placement: &Placement3) -> Result<Self, UvMappingError> {
        let axes: [[f64; 3]; 3] =
            core::array::from_fn(|j| core::array::from_fn(|i| placement.rows[i][j]));
        if !placement.is_finite()
            || (placement.linear_determinant() - 1.0).abs() > 1e-6
            || axes
                .iter()
                .any(|&axis| (dot(axis, axis) - 1.0).abs() > 1e-6)
            || dot(axes[0], axes[1]).abs() > 1e-6
            || dot(axes[0], axes[2]).abs() > 1e-6
            || dot(axes[1], axes[2]).abs() > 1e-6
        {
            return Err(UvMappingError::InvalidFrame);
        }
        let vector = |p: [f64; 3]| {
            core::array::from_fn(|i| {
                dot(
                    [
                        placement.rows[i][0],
                        placement.rows[i][1],
                        placement.rows[i][2],
                    ],
                    p,
                )
            })
        };
        let origin = vector(self.origin);
        let frame = Self {
            origin: core::array::from_fn(|i| origin[i] + placement.rows[i][3]),
            u: vector(self.u),
            v: vector(self.v),
        };
        frame.validate()?;
        Ok(frame)
    }

    /// Checks finite origin and unit, perpendicular axes (tolerance `1e-6`).
    pub fn validate(&self) -> Result<(), UvMappingError> {
        if !self
            .origin
            .into_iter()
            .chain(self.u)
            .chain(self.v)
            .all(f64::is_finite)
            || (dot(self.u, self.u) - 1.0).abs() > 1.0e-6
            || (dot(self.v, self.v) - 1.0).abs() > 1.0e-6
            || dot(self.u, self.v).abs() > 1.0e-6
        {
            return Err(UvMappingError::InvalidFrame);
        }
        Ok(())
    }
}

/// Independent UV orientation on the three pairs of box faces.
///
/// Array indices name the dominant normal axis: X (YZ faces), Y (XZ faces),
/// Z (XY faces). Swapping axes independently provides all eight base layouts.
/// Flips apply after swapping and before the common UV transform. Opposite
/// faces retain the kernel's signed, outward-reading projection convention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoxUv {
    /// Exchange U and V on each pair of faces.
    pub swap_axes: [bool; 3],
    /// Negate U on each pair of faces.
    pub flip_u: [bool; 3],
    /// Negate V on each pair of faces.
    pub flip_v: [bool; 3],
}

impl BoxUv {
    /// Projects a position with an already resolved signed box plane.
    #[must_use]
    pub fn project(&self, position: [f32; 3], plane: BoxPlane) -> [f64; 2] {
        let index = match plane {
            BoxPlane::PosX | BoxPlane::NegX => 0,
            BoxPlane::PosY | BoxPlane::NegY => 1,
            BoxPlane::PosZ | BoxPlane::NegZ => 2,
        };
        let [x, y, z] = position.map(f64::from);
        let mut uv = match plane {
            BoxPlane::PosX => [-z, y],
            BoxPlane::NegX => [z, y],
            BoxPlane::PosY => [x, -z],
            BoxPlane::NegY => [x, z],
            BoxPlane::PosZ => [x, y],
            BoxPlane::NegZ => [-x, y],
        };
        if self.swap_axes[index] {
            uv.swap(0, 1);
        }
        if self.flip_u[index] {
            uv[0] = -uv[0];
        }
        if self.flip_v[index] {
            uv[1] = -uv[1];
        }
        uv
    }
}

/// Source of coordinates before a UV transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UvProjection {
    /// Use authored corner UVs. Missing values are an explicit refusal.
    Existing,
    /// Project onto a freely oriented plane.
    Planar(UvFrame),
    /// Project onto a signed dominant-normal box face.
    Box(BoxUv),
    /// Project onto each face's own plane, through mesh zero. The direct mesh
    /// operation uses the geometric face normal, ignoring shading overrides.
    Face,
    /// Angular U around `frame.u × frame.v`, axial distance V in mesh units.
    /// U starts on `frame.u` and grows toward `frame.v`.
    Cylindrical(UvFrame),
    /// Longitude U as for cylindrical projection; latitude V grows from zero
    /// on the negative axis to one on the positive axis. Radius is ignored.
    Spherical(UvFrame),
    /// U turns around the frame axis; V turns around the tube's centerline.
    /// V zero is the outward radial direction. Radius is in mesh units.
    Toroidal {
        /// Projection origin and radial axes.
        frame: UvFrame,
        /// Positive, finite distance from origin to the tube centerline.
        major_radius: f64,
    },
    /// Perspective image projection. No clipping, occlusion, or image sampling.
    Perspective(PerspectiveUv),
}

/// Projection followed by an affine UV transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvMapping {
    /// Coordinate source.
    pub projection: UvProjection,
    /// Mapping from source coordinates to texture repeats.
    pub transform: UvTransform,
}

impl UvProjection {
    /// Axes expressed as wrapped turns before the affine UV transform.
    #[must_use]
    pub const fn periodic_axes(self) -> [bool; 2] {
        match self {
            Self::Cylindrical(_) | Self::Spherical(_) => [true, false],
            Self::Toroidal { .. } => [true, true],
            _ => [false, false],
        }
    }
    /// Whether this projection uses a surface normal.
    #[must_use]
    pub const fn uses_normals(self) -> bool {
        matches!(self, Self::Box(_) | Self::Face)
    }
    /// Whether this projection reads authored source UVs instead of positions.
    #[must_use]
    pub const fn uses_existing(self) -> bool {
        matches!(self, Self::Existing)
    }
    /// Checks parameters independently of geometry.
    pub fn validate(self) -> Result<(), UvMappingError> {
        match self {
            Self::Planar(frame) | Self::Cylindrical(frame) | Self::Spherical(frame) => {
                frame.validate()
            }
            Self::Toroidal {
                frame,
                major_radius,
            } => {
                frame.validate()?;
                if !major_radius.is_finite() || major_radius <= 0.0 {
                    return Err(UvMappingError::InvalidRadius);
                }
                Ok(())
            }
            Self::Perspective(projector) => {
                projector.frame.validate()?;
                if !projector
                    .focal
                    .into_iter()
                    .all(|x| x.is_finite() && x > 0.0)
                    || !projector.near.is_finite()
                    || projector.near < 0.0
                {
                    return Err(UvMappingError::InvalidProjector);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    uv: [f64; 2],
    undefined: [bool; 2],
    fallback: bool,
    seam: bool,
    pole: bool,
}

/// Reusable storage for per-face projection and angular sorting. Bulk callers
/// retain one scratch value and one `UvFaceMapping` across all faces, so storage
/// grows only when a face exceeds previously seen valence.
#[derive(Debug, Default)]
pub struct UvMappingScratch {
    samples: Vec<Sample>,
    angles: Vec<f64>,
}
impl UvMappingScratch {
    /// Retained capacity bytes, excluding allocator overhead.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.samples.capacity() * size_of::<Sample>() + self.angles.capacity() * size_of::<f64>()
    }
}

impl UvMapping {
    /// Checks projection and transform parameters without inspecting a mesh.
    pub fn validate(&self) -> Result<(), UvMappingError> {
        self.transform.validate()?;
        self.projection.validate()
    }

    fn sample(&self, input: &UvInput) -> Result<Sample, UvMappingError> {
        let mut sample = Sample {
            uv: [0.0; 2],
            undefined: [false; 2],
            fallback: false,
            seam: false,
            pole: false,
        };
        if self.projection.uses_existing() {
            sample.uv = input
                .existing
                .ok_or(UvMappingError::MissingUv)?
                .map(f64::from);
        } else {
            if !input.position.into_iter().all(f32::is_finite) {
                return Err(UvMappingError::NumericLimit);
            }
            let p = input.position.map(f64::from);
            sample.uv = match self.projection {
                UvProjection::Existing => unreachable!("handled existing source"),
                UvProjection::Box(layout) => {
                    if !input.normal.into_iter().all(f64::is_finite) {
                        return Err(UvMappingError::NumericLimit);
                    }
                    let (plane, fallback) =
                        box_plane_from_normal(input.normal, DEFAULT_BOX_NORMAL_EPSILON);
                    sample.fallback = fallback;
                    layout.project(input.position, plane)
                }
                UvProjection::Face => {
                    if !input.normal.into_iter().all(f64::is_finite) {
                        return Err(UvMappingError::NumericLimit);
                    }
                    let frame = match UvFrame::from_normal([0.0; 3], input.normal) {
                        Ok(frame) => frame,
                        Err(_) => {
                            sample.fallback = true;
                            UvFrame::XY
                        }
                    };
                    [dot(p, frame.u), dot(p, frame.v)]
                }
                UvProjection::Planar(frame) => {
                    let p = sub(p, frame.origin);
                    [dot(p, frame.u), dot(p, frame.v)]
                }
                UvProjection::Perspective(projector) => {
                    let p = sub(p, projector.frame.origin);
                    let depth = dot(p, cross(projector.frame.u, projector.frame.v));
                    if !depth.is_finite() {
                        return Err(UvMappingError::NumericLimit);
                    }
                    if depth <= projector.near {
                        return Err(UvMappingError::BehindProjector);
                    }
                    [
                        0.5 + projector.focal[0] * dot(p, projector.frame.u) / depth,
                        0.5 + projector.focal[1] * dot(p, projector.frame.v) / depth,
                    ]
                }
                UvProjection::Cylindrical(frame)
                | UvProjection::Spherical(frame)
                | UvProjection::Toroidal { frame, .. } => {
                    let p = sub(p, frame.origin);
                    let x = dot(p, frame.u);
                    let y = dot(p, frame.v);
                    let z = dot(p, cross(frame.u, frame.v));
                    let radial = hypot(x, y);
                    sample.undefined[0] = radial == 0.0;
                    let u = turn(y, x);
                    let v = match self.projection {
                        UvProjection::Cylindrical(_) => z,
                        UvProjection::Spherical(_) => {
                            if radial == 0.0 && z == 0.0 {
                                return Err(UvMappingError::ProjectionCenter);
                            }
                            atan2(z, radial) / core::f64::consts::PI + 0.5
                        }
                        UvProjection::Toroidal { major_radius, .. } => {
                            sample.undefined[1] = radial == major_radius && z == 0.0;
                            turn(z, radial - major_radius)
                        }
                        _ => unreachable!("angular projection"),
                    };
                    [u, v]
                }
            };
        }
        if !sample.uv.into_iter().all(f64::is_finite) {
            return Err(UvMappingError::NumericLimit);
        }
        Ok(sample)
    }

    // Keep checks outside the arithmetic loop so LLVM can vectorize across
    // corners. Failed batches use the checked scalar path for an exact witness.
    // Arithmetic order matches `sample` then `apply_coordinates`; composing the
    // projection and UV matrices would change floating-point rounding.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "finite f32 storage boundary is checked after the arithmetic loop"
    )]
    fn map_linear_batch(&self, inputs: &[UvInput], output: &mut Vec<[f32; 2]>) -> bool {
        let matrix = self.transform.matrix;
        let offset = self.transform.offset;
        match self.projection {
            UvProjection::Planar(frame) => {
                if !inputs
                    .iter()
                    .all(|input| input.position.into_iter().all(f32::is_finite))
                {
                    return false;
                }
                output.resize(inputs.len(), [0.0; 2]);
                for (input, uv) in inputs.iter().zip(output.iter_mut()) {
                    let p = sub(input.position.map(f64::from), frame.origin);
                    let a = dot(p, frame.u);
                    let b = dot(p, frame.v);
                    *uv = [
                        (matrix[0][0] * a + matrix[0][1] * b + offset[0]) as f32,
                        (matrix[1][0] * a + matrix[1][1] * b + offset[1]) as f32,
                    ];
                }
            }
            UvProjection::Existing => {
                if !inputs.iter().all(|input| {
                    input
                        .existing
                        .is_some_and(|uv| uv.into_iter().all(f32::is_finite))
                }) {
                    return false;
                }
                output.resize(inputs.len(), [0.0; 2]);
                for (input, uv) in inputs.iter().zip(output.iter_mut()) {
                    let [a, b] = input.existing.unwrap_or([0.0; 2]).map(f64::from);
                    *uv = [
                        (matrix[0][0] * a + matrix[0][1] * b + offset[0]) as f32,
                        (matrix[1][0] * a + matrix[1][1] * b + offset[1]) as f32,
                    ];
                }
            }
            _ => return false,
        }
        if output.iter().all(|uv| uv.iter().all(|x| x.is_finite())) {
            true
        } else {
            output.clear();
            false
        }
    }

    fn resolve_shared_normal(&self, inputs: &[UvInput]) -> (Self, bool) {
        let Some(first) = inputs.first() else {
            return (*self, false);
        };
        if !self.projection.uses_normals()
            || !first.normal.into_iter().all(f64::is_finite)
            || !inputs.iter().all(|input| input.normal == first.normal)
        {
            return (*self, false);
        }
        let (mut frame, fallback) = match self.projection {
            UvProjection::Box(layout) => {
                let (plane, fallback) =
                    box_plane_from_normal(first.normal, DEFAULT_BOX_NORMAL_EPSILON);
                let mut frame = match plane {
                    BoxPlane::PosX => UvFrame {
                        u: [0.0, 0.0, -1.0],
                        v: [0.0, 1.0, 0.0],
                        origin: [0.0; 3],
                    },
                    BoxPlane::NegX => UvFrame {
                        u: [0.0, 0.0, 1.0],
                        v: [0.0, 1.0, 0.0],
                        origin: [0.0; 3],
                    },
                    BoxPlane::PosY => UvFrame {
                        v: [0.0, 0.0, -1.0],
                        ..UvFrame::XZ
                    },
                    BoxPlane::NegY => UvFrame::XZ,
                    BoxPlane::PosZ => UvFrame::XY,
                    BoxPlane::NegZ => UvFrame {
                        u: [-1.0, 0.0, 0.0],
                        ..UvFrame::XY
                    },
                };
                let axis = match plane {
                    BoxPlane::PosX | BoxPlane::NegX => 0,
                    BoxPlane::PosY | BoxPlane::NegY => 1,
                    _ => 2,
                };
                if layout.swap_axes[axis] {
                    core::mem::swap(&mut frame.u, &mut frame.v);
                }
                if layout.flip_u[axis] {
                    frame.u = frame.u.map(|x| -x);
                }
                if layout.flip_v[axis] {
                    frame.v = frame.v.map(|x| -x);
                }
                (frame, fallback)
            }
            _ => match UvFrame::from_normal([0.0; 3], first.normal) {
                Ok(frame) => (frame, false),
                Err(_) => (UvFrame::XY, true),
            },
        };
        frame.origin = [0.0; 3];
        (
            Self {
                projection: UvProjection::Planar(frame),
                transform: self.transform,
            },
            fallback,
        )
    }

    /// Maps one corner. Undefined angles are refused because a single corner
    /// has no neighbors from which to recover an orientation. Use `map_face`
    /// for seam correction and pole recovery.
    pub fn map(&self, input: &UvInput) -> Result<[f32; 2], UvMappingError> {
        self.validate()?;
        let sample = self.sample(input)?;
        if let Some(axis) = sample.undefined.iter().position(|&x| x) {
            return Err(UvMappingError::UndefinedAngle {
                axis: if axis == 0 { UvAxis::U } else { UvAxis::V },
            });
        }
        self.transform.apply(sample.uv)
    }

    /// Maps a face without modifying geometry. For repeated faces, retain
    /// scratch storage and use `map_face_into` to avoid per-face allocations.
    pub fn map_face(
        &self,
        inputs: &[UvInput],
        seams: UvSeamPolicy,
        poles: UvPolePolicy,
    ) -> Result<UvFaceMapping, UvFaceError> {
        let mut output = UvFaceMapping::default();
        self.map_face_into(
            inputs,
            seams,
            poles,
            &mut UvMappingScratch::default(),
            &mut output,
        )?;
        Ok(output)
    }

    /// Maps a face into reusable storage. Output is reset before evaluation;
    /// on refusal it is not a completed mapping. No topology or mesh is mutated.
    pub fn map_face_into(
        &self,
        inputs: &[UvInput],
        seams: UvSeamPolicy,
        poles: UvPolePolicy,
        scratch: &mut UvMappingScratch,
        output: &mut UvFaceMapping,
    ) -> Result<(), UvFaceError> {
        output.uvs.clear();
        output.seam_corners_adjusted = 0;
        output.pole_corners_repaired = 0;
        output.normal_fallback = false;
        output.wide_periodic_axes = [false; 2];
        scratch.samples.clear();
        self.validate().map_err(|error| UvFaceError {
            corner: None,
            error,
        })?;
        // Resolve a shared face normal once; varying shading normals retain
        // the general per-corner box path.
        let (resolved, fallback) = self.resolve_shared_normal(inputs);
        output.normal_fallback = fallback;
        if !self.projection.periodic_axes().into_iter().any(|axis| axis) {
            if resolved.map_linear_batch(inputs, &mut output.uvs) {
                return Ok(());
            }
            // Perspective and varying normals retain checked scalar evaluation.
            for (corner, input) in inputs.iter().enumerate() {
                let sample = resolved.sample(input).map_err(|error| UvFaceError {
                    corner: Some(corner),
                    error,
                })?;
                output.uvs.push(
                    self.transform
                        .apply_coordinates(sample.uv)
                        .map_err(|error| UvFaceError {
                            corner: Some(corner),
                            error,
                        })?,
                );
                output.normal_fallback |= sample.fallback;
            }
            return Ok(());
        }
        for (corner, input) in inputs.iter().enumerate() {
            scratch
                .samples
                .push(resolved.sample(input).map_err(|error| UvFaceError {
                    corner: Some(corner),
                    error,
                })?);
        }
        for (axis, periodic) in self.projection.periodic_axes().into_iter().enumerate() {
            if !periodic {
                continue;
            }
            scratch.angles.clear();
            for sample in &scratch.samples {
                if !sample.undefined[axis] {
                    scratch.angles.push(sample.uv[axis]);
                }
            }
            let undefined = scratch
                .samples
                .iter()
                .position(|sample| sample.undefined[axis]);
            if let Some(corner) = undefined
                && (poles == UvPolePolicy::Refuse || scratch.angles.is_empty())
            {
                return Err(UvFaceError {
                    corner: Some(corner),
                    error: UvMappingError::UndefinedAngle {
                        axis: if axis == 0 { UvAxis::U } else { UvAxis::V },
                    },
                });
            }
            let mut shift = 0.0;
            let mut cut = 0.0;
            if seams == UvSeamPolicy::Continuous && !scratch.angles.is_empty() {
                let first = scratch.angles[0];
                scratch.angles.sort_unstable_by(f64::total_cmp);
                let mut gap = -1.0;
                for i in 0..scratch.angles.len() {
                    let next = (i + 1) % scratch.angles.len();
                    let size = scratch.angles[next] + if next == 0 { 1.0 } else { 0.0 }
                        - scratch.angles[i];
                    if size > gap {
                        gap = size;
                        cut = scratch.angles[next];
                    }
                }
                if first < cut {
                    shift = 1.0;
                }
            }
            let mut sum = 0.0;
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for sample in &mut scratch.samples {
                if sample.undefined[axis] {
                    continue;
                }
                let old = sample.uv[axis];
                sample.uv[axis] += if old < cut { 1.0 } else { 0.0 } - shift;
                sample.seam |= sample.uv[axis] != old;
                sum += sample.uv[axis];
                min = min.min(sample.uv[axis]);
                max = max.max(sample.uv[axis]);
            }
            output.wide_periodic_axes[axis] = max - min > 0.5 + 1.0e-12;
            if undefined.is_some() {
                let mean = sum / scratch.angles.len() as f64;
                for sample in &mut scratch.samples {
                    if sample.undefined[axis] {
                        sample.uv[axis] = mean;
                        sample.pole = true;
                    }
                }
            }
        }
        for (corner, sample) in scratch.samples.iter().enumerate() {
            // Validation above is shared across the face; avoid rechecking the
            // transform for every corner in this bulk path.
            output.uvs.push(
                self.transform
                    .apply_coordinates(sample.uv)
                    .map_err(|error| UvFaceError {
                        corner: Some(corner),
                        error,
                    })?,
            );
            output.normal_fallback |= sample.fallback;
            output.seam_corners_adjusted += u64::from(sample.seam);
            output.pole_corners_repaired += u64::from(sample.pole);
        }
        Ok(())
    }
}

fn hypot(x: f64, y: f64) -> f64 {
    let scale = x.abs().max(y.abs());
    if scale == 0.0 {
        0.0
    } else {
        scale * Real::sqrt((x / scale) * (x / scale) + (y / scale) * (y / scale))
    }
}
fn turn(y: f64, x: f64) -> f64 {
    let t = atan2(y, x) / core::f64::consts::TAU;
    if t < 0.0 { t + 1.0 } else { t }
}
fn atan2(y: f64, x: f64) -> f64 {
    #[cfg(feature = "libm")]
    {
        libm::atan2(y, x)
    }
    #[cfg(not(feature = "libm"))]
    {
        y.atan2_ext(x)
    }
}
fn sin_cos(x: f64) -> (f64, f64) {
    #[cfg(feature = "libm")]
    {
        (libm::sin(x), libm::cos(x))
    }
    #[cfg(not(feature = "libm"))]
    {
        (x.sin_ext(), x.cos_ext())
    }
}

/// The UV axis associated with an undefined angular coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvAxis {
    /// Horizontal texture axis.
    U,
    /// Vertical texture axis.
    V,
}

/// Refusal from texture-coordinate mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvMappingError {
    /// A toroidal major radius is nonpositive or nonfinite.
    InvalidRadius,
    /// Focal scales or near depth are outside their valid ranges.
    InvalidProjector,
    /// Fit extents are nonfinite, reversed or have a zero span.
    InvalidBounds,
    /// A corner lies at or behind the projector's near plane. Clip geometry
    /// separately, change the projector, or select only the intended faces.
    BehindProjector,
    /// Spherical direction is undefined at the projection center.
    ProjectionCenter,
    /// An angular coordinate could not be recovered under the chosen policy.
    UndefinedAngle {
        /// Undefined UV axis.
        axis: UvAxis,
    },
    /// A transform coefficient is nonfinite.
    InvalidTransform,
    /// The projection frame is nonfinite or not orthonormal.
    InvalidFrame,
    /// Existing-coordinate mapping encountered a missing UV.
    MissingUv,
    /// Input coordinates or the final UV value exceed numeric limits.
    NumericLimit,
}

impl core::fmt::Display for UvMappingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidRadius => "toroidal radius must be finite and positive",
            Self::InvalidProjector => "invalid perspective focal scales or near depth",
            Self::InvalidBounds => "UV fit needs finite positive extents",
            Self::BehindProjector => "corner is at or behind the projector near plane",
            Self::ProjectionCenter => "spherical direction is undefined at the projection center",
            Self::UndefinedAngle { axis: UvAxis::U } => {
                "undefined U angle; change pole policy, subdivide or choose another projection"
            }
            Self::UndefinedAngle { axis: UvAxis::V } => {
                "undefined V angle; change pole policy, subdivide or choose another projection"
            }
            Self::InvalidTransform => "nonfinite UV transform",
            Self::InvalidFrame => "invalid UV projection frame",
            Self::MissingUv => "missing authored UV coordinates",
            Self::NumericLimit => "UV mapping exceeds numeric limits",
        })
    }
}
impl core::error::Error for UvMappingError {}

#[expect(
    clippy::cast_possible_truncation,
    reason = "checked f64-to-f32 UV storage boundary"
)]
fn narrow(uv: [f64; 2]) -> Result<[f32; 2], UvMappingError> {
    let uv = uv.map(|x| x as f32);
    if uv.into_iter().all(f32::is_finite) {
        Ok(uv)
    } else {
        Err(UvMappingError::NumericLimit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(position: [f32; 3]) -> UvInput {
        UvInput {
            position,
            normal: [0.0, 0.0, 1.0],
            existing: None,
        }
    }
    fn projection(projection: UvProjection) -> UvMapping {
        UvMapping {
            projection,
            transform: UvTransform::IDENTITY,
        }
    }

    #[test]
    fn cylinder_seams_and_poles_are_recovered_before_the_uv_transform() {
        let mut mapping = projection(UvProjection::Cylindrical(UvFrame::XY));
        mapping.transform = UvTransform::scale([3.0, 2.0]).unwrap();
        let face = [
            input([1.0, -0.01, 2.0]),
            input([1.0, 0.01, 2.0]),
            input([0.0, 0.0, 3.0]),
        ];
        let output = mapping
            .map_face(&face, UvSeamPolicy::Continuous, UvPolePolicy::FaceAverage)
            .unwrap();
        assert!((output.uvs[0][0] - output.uvs[1][0]).abs() < 0.02);
        assert_eq!(output.seam_corners_adjusted, 1);
        assert_eq!(output.pole_corners_repaired, 1);
        assert_eq!(output.uvs[2][1], 6.0);
        assert!((output.uvs[2][0] - (output.uvs[0][0] + output.uvs[1][0]) * 0.5).abs() < 1e-6);
        assert!(
            mapping
                .map_face(&face, UvSeamPolicy::Continuous, UvPolePolicy::Refuse)
                .is_err()
        );
        let raw = mapping
            .map_face(&face[..2], UvSeamPolicy::Preserve, UvPolePolicy::Refuse)
            .unwrap();
        assert!(raw.wide_periodic_axes[0]);
        assert_eq!(raw.seam_corners_adjusted, 0);
    }

    #[test]
    fn spherical_latitude_and_undefined_directions_have_explicit_policies() {
        let mapping = projection(UvProjection::Spherical(UvFrame::XY));
        let face = [
            input([1.0, 0.0, 0.0]),
            input([0.0, 1.0, 0.0]),
            input([0.0, 0.0, 1.0]),
        ];
        let result = mapping
            .map_face(&face, UvSeamPolicy::Continuous, UvPolePolicy::FaceAverage)
            .unwrap();
        assert_eq!(
            result.uvs,
            alloc::vec![[0.0, 0.5], [0.25, 0.5], [0.125, 1.0]]
        );
        assert_eq!(result.pole_corners_repaired, 1);
        assert_eq!(
            mapping.map(&input([0.0; 3])),
            Err(UvMappingError::ProjectionCenter)
        );
        let error = mapping
            .map_face(
                &[input([0.0, 0.0, 1.0]); 3],
                UvSeamPolicy::Continuous,
                UvPolePolicy::FaceAverage,
            )
            .unwrap_err();
        assert_eq!(
            error.error,
            UvMappingError::UndefinedAngle { axis: UvAxis::U }
        );
    }

    #[test]
    fn toroidal_projection_corrects_both_periodic_axes() {
        let mapping = projection(UvProjection::Toroidal {
            frame: UvFrame::XY,
            major_radius: 2.0,
        });
        let face = [
            input([2.5, -0.01, -0.01]),
            input([2.5, 0.01, -0.01]),
            input([2.5, 0.01, 0.01]),
            input([2.5, -0.01, 0.01]),
        ];
        let result = mapping
            .map_face(&face, UvSeamPolicy::Continuous, UvPolePolicy::FaceAverage)
            .unwrap();
        for axis in 0..2 {
            let min = result
                .uvs
                .iter()
                .map(|uv| uv[axis])
                .fold(f32::INFINITY, f32::min);
            let max = result
                .uvs
                .iter()
                .map(|uv| uv[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(max - min < 0.02, "axis {axis}: {min}..{max}");
        }
        assert!(result.seam_corners_adjusted >= 2);
        assert!(
            projection(UvProjection::Toroidal {
                frame: UvFrame::XY,
                major_radius: 0.0
            })
            .validate()
            .is_err()
        );
    }

    #[test]
    fn perspective_fit_pivot_and_composition_use_explicit_spaces() {
        let mapping = projection(UvProjection::Perspective(PerspectiveUv {
            frame: UvFrame::XY,
            focal: [1.0, 2.0],
            near: 0.01,
        }));
        assert_eq!(mapping.map(&input([1.0, 0.5, 2.0])).unwrap(), [1.0, 1.0]);
        assert_eq!(
            mapping.map(&input([0.0, 0.0, -1.0])),
            Err(UvMappingError::BehindProjector)
        );
        let rotation = UvTransform::rotation(core::f64::consts::PI)
            .unwrap()
            .around_pivot([0.5; 2])
            .unwrap();
        let transform = rotation
            .then(UvTransform::translation([2.0, 3.0]).unwrap())
            .unwrap();
        assert_eq!(transform.apply([0.0; 2]).unwrap(), [3.0, 4.0]);
        let fit = UvTransform::fit_bounds([2.0, 3.0], [6.0, 5.0], true).unwrap();
        assert_eq!(fit.apply([2.0, 3.0]).unwrap(), [0.0, 0.25]);
        assert_eq!(fit.apply([6.0, 5.0]).unwrap(), [1.0, 0.75]);
        assert!(UvTransform::fit_bounds([0.0; 2], [0.0; 2], false).is_err());
        let frame = UvFrame::XY
            .placed(&Placement3::rotate_x_then_translate(
                core::f64::consts::FRAC_PI_2,
                1.0,
                2.0,
                3.0,
            ))
            .unwrap();
        assert_eq!(frame.origin, [1.0, 2.0, 3.0]);
        assert!(frame.v[2] > 0.999999);
    }
    fn mapped(
        mapping: UvMapping,
        position: [f32; 3],
        existing: Option<[f32; 2]>,
    ) -> Result<[f32; 2], UvMappingError> {
        mapping.map(&UvInput {
            position,
            normal: [0.0, 0.0, 1.0],
            existing,
        })
    }

    #[test]
    fn linear_batches_match_scalar_bits_and_preserve_failure_witnesses() {
        let frame = UvFrame::from_normal([0.125, -0.75, 1.25], [1.0, 2.0, 3.0]).unwrap();
        for projection in [UvProjection::Planar(frame), UvProjection::Existing] {
            let mapping = UvMapping {
                projection,
                transform: UvTransform {
                    matrix: [[1.8, -0.6], [0.35, 1.4]],
                    offset: [0.25, -0.4],
                },
            };
            let mut scratch = UvMappingScratch::default();
            let mut output = UvFaceMapping::default();
            for count in [0, 1, 3, 4, 255, 256, 257, 1025] {
                let mut inputs: Vec<_> = (0..count)
                    .map(|i| UvInput {
                        position: [
                            i as f32 * 0.03125 - 2.0,
                            (i % 17) as f32 * -0.125,
                            (i % 31) as f32 * 0.3,
                        ],
                        normal: [0.0; 3],
                        existing: Some([i as f32 * 0.03125, (i % 13) as f32 * -0.4]),
                    })
                    .collect();
                mapping
                    .map_face_into(
                        &inputs,
                        UvSeamPolicy::Continuous,
                        UvPolePolicy::FaceAverage,
                        &mut scratch,
                        &mut output,
                    )
                    .unwrap();
                assert_eq!(output.uvs.len(), count);
                for (input, uv) in inputs.iter().zip(&output.uvs) {
                    assert_eq!(
                        mapping.map(input).unwrap().map(f32::to_bits),
                        uv.map(f32::to_bits)
                    );
                }
                if let Some(last) = inputs.last_mut() {
                    let error = if projection.uses_existing() {
                        last.existing = None;
                        UvMappingError::MissingUv
                    } else {
                        last.position[1] = f32::INFINITY;
                        UvMappingError::NumericLimit
                    };
                    assert_eq!(
                        mapping.map_face_into(
                            &inputs,
                            UvSeamPolicy::Continuous,
                            UvPolePolicy::FaceAverage,
                            &mut scratch,
                            &mut output
                        ),
                        Err(UvFaceError {
                            corner: Some(count - 1),
                            error
                        })
                    );
                }
            }
            let overflowing = UvMapping {
                transform: UvTransform::scale([f64::MAX; 2]).unwrap(),
                ..mapping
            };
            let inputs = [UvInput {
                position: [5.0; 3],
                normal: [0.0; 3],
                existing: Some([5.0; 2]),
            }; 257];
            assert_eq!(
                overflowing.map_face_into(
                    &inputs,
                    UvSeamPolicy::Continuous,
                    UvPolePolicy::FaceAverage,
                    &mut scratch,
                    &mut output
                ),
                Err(UvFaceError {
                    corner: Some(0),
                    error: UvMappingError::NumericLimit
                })
            );
        }
    }

    #[test]
    fn rotation_then_offset_then_nonuniform_scale() {
        let transform =
            UvTransform::rotate_offset_scale(core::f64::consts::FRAC_PI_2, [5.0, 7.0], [2.0, -3.0])
                .unwrap();
        let uv = transform.apply([2.0, 3.0]).unwrap();
        assert!(
            (uv[0] - 4.0).abs() < 1e-5 && (uv[1] + 27.0).abs() < 1e-5,
            "{uv:?}"
        );
        assert_eq!(
            UvTransform::rotate_offset_scale(0.0, [2.0; 2], [0.0; 2])
                .unwrap()
                .apply([3.0; 2])
                .unwrap(),
            [0.0; 2]
        );
    }

    #[test]
    fn canonical_and_freely_oriented_planes() {
        for (frame, expected) in [
            (UvFrame::XY, [2.0, 3.0]),
            (UvFrame::XZ, [2.0, 5.0]),
            (UvFrame::YZ, [3.0, 5.0]),
        ] {
            let mapping = UvMapping {
                projection: UvProjection::Planar(frame),
                transform: UvTransform::IDENTITY,
            };
            assert_eq!(mapped(mapping, [2.0, 3.0, 5.0], None).unwrap(), expected);
        }
        for scale in [f64::MIN_POSITIVE, 1.0, f64::MAX] {
            let frame = UvFrame::from_normal([2.0, 3.0, 5.0], [0.0, 0.0, scale]).unwrap();
            assert_eq!(frame.u, [1.0, 0.0, 0.0]);
            assert_eq!(frame.v, [0.0, 1.0, 0.0]);
            let mapping = UvMapping {
                projection: UvProjection::Planar(frame),
                transform: UvTransform::IDENTITY,
            };
            assert_eq!(mapped(mapping, [4.0, 7.0, 10.0], None).unwrap(), [2.0, 4.0]);
        }
        let frame = UvFrame::from_normal([0.0; 3], [1.0, 1.0, 1.0]).unwrap();
        assert!(dot(frame.u, [1.0; 3]).abs() < 1e-12);
        assert!(dot(frame.v, [1.0; 3]).abs() < 1e-12);
    }

    #[test]
    fn eight_box_layouts_and_signed_opposite_faces() {
        let cases = [
            (BoxPlane::PosX, [-5.0, 3.0], 0),
            (BoxPlane::NegX, [5.0, 3.0], 0),
            (BoxPlane::PosY, [2.0, -5.0], 1),
            (BoxPlane::NegY, [2.0, 5.0], 1),
            (BoxPlane::PosZ, [2.0, 3.0], 2),
            (BoxPlane::NegZ, [-2.0, 3.0], 2),
        ];
        for bits in 0..8 {
            let swap_axes = core::array::from_fn(|i| bits & (1 << i) != 0);
            let layout = BoxUv {
                swap_axes,
                ..BoxUv::default()
            };
            for (plane, base, axis) in cases {
                let expected = if swap_axes[axis] {
                    [base[1], base[0]]
                } else {
                    base
                };
                assert_eq!(layout.project([2.0, 3.0, 5.0], plane), expected);
                let mirrored = BoxUv {
                    flip_u: [true; 3],
                    flip_v: [true; 3],
                    ..layout
                };
                assert_eq!(
                    mirrored.project([2.0, 3.0, 5.0], plane),
                    expected.map(|x| -x)
                );
            }
        }
    }

    #[test]
    fn refuses_bad_frames_missing_uvs_and_numeric_overflow() {
        assert_eq!(
            UvFrame::from_normal([0.0; 3], [0.0; 3]),
            Err(UvMappingError::InvalidFrame)
        );
        assert!(
            UvFrame {
                u: [2.0, 0.0, 0.0],
                ..UvFrame::XY
            }
            .validate()
            .is_err()
        );
        assert!(
            UvFrame {
                v: [1.0, 0.0, 0.0],
                ..UvFrame::XY
            }
            .validate()
            .is_err()
        );
        let mapping = UvMapping {
            projection: UvProjection::Existing,
            transform: UvTransform::IDENTITY,
        };
        assert_eq!(
            mapped(mapping, [0.0; 3], None),
            Err(UvMappingError::MissingUv)
        );
        assert_eq!(
            mapped(mapping, [f32::NAN; 3], Some([2.0, -3.0])).unwrap(),
            [2.0, -3.0]
        );
        assert_eq!(
            mapped(mapping, [0.0; 3], Some([f32::NAN; 2])),
            Err(UvMappingError::NumericLimit)
        );
        let transform = UvTransform {
            matrix: [[f64::MAX, 0.0], [0.0, 1.0]],
            offset: [0.0; 2],
        };
        assert_eq!(transform.apply([2.0; 2]), Err(UvMappingError::NumericLimit));
        assert!(UvTransform::rotate_offset_scale(f64::NAN, [0.0; 2], [1.0; 2]).is_err());
    }
}
