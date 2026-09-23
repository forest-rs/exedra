// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared planes and affine placements over plain coordinate arrays.
//!
//! Rotation constructors prefer libm whenever that feature is enabled. The
//! constructive domain enables it explicitly for stable recipe evaluation.

use crate::trig;

/// A rigid-or-affine placement as a 3x4 row-major matrix (rotation/scale
/// columns plus translation).
///
/// The matrix acts on column vectors: a point `p` maps to `linear * p + t`.
/// Consequently, `outer * inner` applies `inner` first. Rotation constructor
/// documentation names both the axis space and the order in which rotations
/// are applied to avoid relying on ambiguous Euler shorthand.
///
/// General affine constructors do not validate finiteness, rigidity, or
/// invertibility. Use [`Self::try_from_orthonormal_axes`] for checked frames;
/// other callers validate the constraints required by each geometric operation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Placement3 {
    /// Rows of the 3x4 matrix: `[r0x, r0y, r0z, tx]`, etc.
    pub rows: [[f64; 4]; 3],
}

impl Placement3 {
    /// The identity placement.
    pub const IDENTITY: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };

    /// A pure translation.
    #[must_use]
    pub fn translate(x: f64, y: f64, z: f64) -> Self {
        Self {
            rows: [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z]],
        }
    }

    /// Rotation about +X by `radians` (libm trig when enabled), then translation.
    #[must_use]
    pub fn rotate_x_then_translate(radians: f64, x: f64, y: f64, z: f64) -> Self {
        let (s, c) = trig::sincos(radians);
        Self {
            rows: [[1.0, 0.0, 0.0, x], [0.0, c, -s, y], [0.0, s, c, z]],
        }
    }

    /// Builds a placement from local basis axes and a translation.
    ///
    /// The three axes become the matrix columns: transforming local
    /// `[1, 0, 0]`, `[0, 1, 0]`, and `[0, 0, 1]` yields `x_axis`, `y_axis`,
    /// and `z_axis`, respectively. This constructor does not normalize or
    /// validate the axes; the consuming operation supplies its own validation.
    #[must_use]
    pub const fn from_axes(
        x_axis: [f64; 3],
        y_axis: [f64; 3],
        z_axis: [f64; 3],
        translation: [f64; 3],
    ) -> Self {
        Self {
            rows: [
                [x_axis[0], y_axis[0], z_axis[0], translation[0]],
                [x_axis[1], y_axis[1], z_axis[1], translation[1]],
                [x_axis[2], y_axis[2], z_axis[2], translation[2]],
            ],
        }
    }

    /// Rotation about +Z by `radians` (libm trig when enabled), then translation.
    #[must_use]
    pub fn rotate_z_then_translate(radians: f64, x: f64, y: f64, z: f64) -> Self {
        let (s, c) = trig::sincos(radians);
        Self {
            rows: [[c, -s, 0.0, x], [s, c, 0.0, y], [0.0, 0.0, 1.0, z]],
        }
    }

    /// Compatibility spelling for
    /// [`Self::euler_extrinsic_xyz_then_translate`].
    ///
    /// This rotates column vectors about the fixed +X, +Y, and +Z axes, in
    /// that order, before applying `t`. The rotation matrix is
    /// `Rz(rz) * Ry(ry) * Rx(rx)`. Equivalently, it is an intrinsic Z, Y', X''
    /// rotation. Prefer the explicitly named constructor in new code.
    #[must_use]
    #[deprecated(note = "use `euler_extrinsic_xyz_then_translate` or \
                `euler_intrinsic_xyz_then_translate`")]
    pub fn euler_xyz_then_translate(rx: f64, ry: f64, rz: f64, t: [f64; 3]) -> Self {
        Self::euler_extrinsic_xyz_then_translate(rx, ry, rz, t)
    }

    /// Rotates about fixed +X, +Y, and +Z axes, then translates by `t`.
    ///
    /// The rotations are applied to column vectors in X, Y, Z order, so the
    /// composed matrix is `Rz(rz) * Ry(ry) * Rx(rx)`. This is also describable
    /// as an intrinsic Z, Y', X'' rotation.
    ///
    /// # Example
    ///
    /// Fixed-axis quarter turns about X and then Y send +Y to +X:
    ///
    /// ```
    /// use exedra_math::Placement3;
    ///
    /// let q = core::f64::consts::FRAC_PI_2;
    /// let placement =
    ///     Placement3::euler_extrinsic_xyz_then_translate(q, q, 0.0, [0.0; 3]);
    /// let transformed_y = placement.rows.map(|row| row[1]);
    /// for (actual, expected) in transformed_y.into_iter().zip([1.0, 0.0, 0.0]) {
    ///     assert!((actual - expected).abs() < 1e-12);
    /// }
    /// ```
    #[must_use]
    pub fn euler_extrinsic_xyz_then_translate(rx: f64, ry: f64, rz: f64, t: [f64; 3]) -> Self {
        let (sx, cx) = (trig::sin(rx), trig::cos(rx));
        let (sy, cy) = (trig::sin(ry), trig::cos(ry));
        let (sz, cz) = (trig::sin(rz), trig::cos(rz));
        let rows = [
            [
                cz * cy,
                cz * sy * sx - sz * cx,
                cz * sy * cx + sz * sx,
                t[0],
            ],
            [
                sz * cy,
                sz * sy * sx + cz * cx,
                sz * sy * cx - cz * sx,
                t[1],
            ],
            [-sy, cy * sx, cy * cx, t[2]],
        ];
        Self { rows }
    }

    /// Rotates about body +X, +Y', and +Z'' axes, then translates by `t`.
    ///
    /// These intrinsic rotations are applied in X, Y', Z'' order. For column
    /// vectors, the composed matrix is `Rx(rx) * Ry(ry) * Rz(rz)`. This is
    /// also describable as an extrinsic Z, Y, X rotation.
    ///
    /// # Example
    ///
    /// Body-axis quarter turns about X and then Y' send +Y to +Z:
    ///
    /// ```
    /// use exedra_math::Placement3;
    ///
    /// let q = core::f64::consts::FRAC_PI_2;
    /// let placement =
    ///     Placement3::euler_intrinsic_xyz_then_translate(q, q, 0.0, [0.0; 3]);
    /// let transformed_y = placement.rows.map(|row| row[1]);
    /// for (actual, expected) in transformed_y.into_iter().zip([0.0, 0.0, 1.0]) {
    ///     assert!((actual - expected).abs() < 1e-12);
    /// }
    /// ```
    #[must_use]
    pub fn euler_intrinsic_xyz_then_translate(rx: f64, ry: f64, rz: f64, t: [f64; 3]) -> Self {
        let (sx, cx) = (trig::sin(rx), trig::cos(rx));
        let (sy, cy) = (trig::sin(ry), trig::cos(ry));
        let (sz, cz) = (trig::sin(rz), trig::cos(rz));
        let rows = [
            [cy * cz, -cy * sz, sy, t[0]],
            [
                cx * sz + sx * sy * cz,
                cx * cz - sx * sy * sz,
                -sx * cy,
                t[1],
            ],
            [
                sx * sz - cx * sy * cz,
                sx * cz + cx * sy * sz,
                cx * cy,
                t[2],
            ],
        ];
        Self { rows }
    }

    /// Determinant of the linear part; negative means orientation is reversed.
    #[must_use]
    pub fn linear_determinant(&self) -> f64 {
        let r = &self.rows;
        crate::det3([
            [r[0][0], r[0][1], r[0][2]],
            [r[1][0], r[1][1], r[1][2]],
            [r[2][0], r[2][1], r[2][2]],
        ])
    }

    /// Whether every matrix component is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.rows
            .iter()
            .all(|row| row.iter().all(|v| v.is_finite()))
    }
}

/// A plane in 3D: a normal plus signed distance from the origin.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Plane3 {
    /// Plane normal. It need not be unit length, but must admit finite,
    /// non-degenerate normalization.
    pub normal: [f64; 3],
    /// Signed distance term: the plane is `dot(normal, p) = distance`, so it
    /// must be scaled together with a non-unit normal.
    pub distance: f64,
}

impl Plane3 {
    /// Returns a numerically representable unit-normal form of this plane.
    ///
    /// Merely checking for a nonzero component is insufficient: squaring a
    /// very small normal can underflow to zero, while squaring a large one can
    /// overflow to infinity. These encodings are refused with `None`.
    #[must_use]
    pub fn normalized(&self) -> Option<([f64; 3], f64)> {
        let length = crate::norm(self.normal);
        // `normalize` owns the workspace-wide degeneracy threshold. Compute
        // the components with division afterwards to preserve the canonical
        // arithmetic of existing constructive evaluation.
        crate::normalize(self.normal)?;
        let normal = [
            self.normal[0] / length,
            self.normal[1] / length,
            self.normal[2] / length,
        ];
        let distance = self.distance / length;
        distance.is_finite().then_some((normal, distance))
    }
}

/// Intersects a straddling edge in ascending vertex-ID order. Returns the
/// point and interpolation parameter measured from the caller's first end.
///
/// Each endpoint tuple contains a stable ordering key, a point and its signed
/// plane distance. Callers supply finite straddling endpoints with distinct keys.
/// Zero or nonfinite distance differences return `None`; callers also check the
/// result against their own realization tolerance.
pub fn intersect_plane_edge(
    a: (u32, [f64; 3], f64),
    b: (u32, [f64; 3], f64),
) -> Option<([f64; 3], f64)> {
    let (low, high) = if a.0 < b.0 { (a, b) } else { (b, a) };
    let denominator = low.2 - high.2;
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let t = low.2 / denominator;
    let point = core::array::from_fn(|i| low.1[i] + t * (high.1[i] - low.1[i]));
    Some((point, if a.0 < b.0 { t } else { 1.0 - t }))
}
