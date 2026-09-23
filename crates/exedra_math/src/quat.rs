// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rotation quaternions.

use crate::{FrameError, Placement3, Real, add, cross, scale};

/// A quaternion `w + xi + yj + zk`, used for rotations.
///
/// Conventions:
///
/// - **Hamilton algebra**, stored `x, y, z, w` (glTF's order).
/// - **Active rotations of column vectors**, matching [`Placement3`]: a unit
///   quaternion `q` maps `v` to `q v q*`, and [`Quat::compose`]`(outer,
///   inner)` rotates by `inner` first, like `outer * inner` placements.
/// - `q` and `-q` are the same rotation; [`Quat::slerp`] takes the shorter arc.
///
/// Operations that assume a unit quaternion say so; [`Quat::normalize`] and
/// the checked constructors produce one. Arithmetic uses only `+`, `-`, `*`,
/// `/` and `sqrt` except where trigonometry is inherent (axis–angle
/// construction and slerp), which uses [`Real::sin_cos`] and [`Real::acos`].
///
/// # Example
///
/// ```
/// use exedra_math::Quat;
///
/// let quarter = core::f64::consts::FRAC_PI_2;
/// let q = Quat::from_axis_angle([0.0, 0.0, 1.0], quarter).expect("unit axis");
/// let v = q.rotate([1.0, 0.0, 0.0]);
/// assert!((v[0]).abs() < 1e-15 && (v[1] - 1.0).abs() < 1e-15);
/// ```
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Quat<T> {
    /// The `i` component.
    pub x: T,
    /// The `j` component.
    pub y: T,
    /// The `k` component.
    pub z: T,
    /// The real component.
    pub w: T,
}

impl<T: Real> Quat<T> {
    /// The identity rotation.
    pub const IDENTITY: Self = Self {
        x: T::ZERO,
        y: T::ZERO,
        z: T::ZERO,
        w: T::ONE,
    };

    /// A quaternion from its components.
    #[inline]
    #[must_use]
    pub const fn new(x: T, y: T, z: T, w: T) -> Self {
        Self { x, y, z, w }
    }

    /// The rotation by `radians` about `axis` (right-handed), or `None` when
    /// `axis` is degenerate in the sense of [`crate::normalize`] or the result
    /// is not finite.
    #[must_use]
    pub fn from_axis_angle(axis: [T; 3], radians: T) -> Option<Self> {
        let axis = crate::normalize(axis)?;
        let (s, c) = (radians / two::<T>()).sin_cos();
        let q = Self::new(axis[0] * s, axis[1] * s, axis[2] * s, c);
        q.is_finite().then_some(q)
    }

    /// The vector part `[x, y, z]`.
    #[inline]
    #[must_use]
    pub const fn vector(self) -> [T; 3] {
        [self.x, self.y, self.z]
    }

    /// Four-component dot product, evaluated as `((x·x' + y·y') + z·z') + w·w'`.
    #[inline]
    #[must_use]
    pub fn dot(self, other: Self) -> T {
        self.x * other.x + self.y * other.y + self.z * other.z + self.w * other.w
    }

    /// Euclidean norm `sqrt(dot(self, self))`.
    #[inline]
    #[must_use]
    pub fn norm(self) -> T {
        self.dot(self).sqrt()
    }

    /// Whether every component is finite.
    #[inline]
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }

    /// The unit quaternion along `self`, or `None` when degenerate, with the
    /// same rule as [`crate::normalize`]: a non-finite norm, a norm at or
    /// below `sqrt(MIN_POSITIVE)`, or a non-finite result.
    #[must_use]
    pub fn normalize(self) -> Option<Self> {
        let length = self.norm();
        if !length.is_finite() || length <= T::MIN_POSITIVE.sqrt() {
            return None;
        }
        let q = self.scale(T::ONE / length);
        q.is_finite().then_some(q)
    }

    /// The conjugate `w - xi - yj - zk`; for a unit quaternion, the inverse
    /// rotation.
    #[inline]
    #[must_use]
    pub fn conjugate(self) -> Self {
        Self::new(T::ZERO - self.x, T::ZERO - self.y, T::ZERO - self.z, self.w)
    }

    /// The Hamilton product `self * inner`: rotating by the result rotates by
    /// `inner` first, then by `self`.
    #[must_use]
    pub fn compose(self, inner: Self) -> Self {
        let (a, b) = (self, inner);
        Self::new(
            a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
            a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        )
    }

    /// Rotates `v` by this quaternion, assumed unit, as
    /// `v + 2w(u × v) + 2u × (u × v)` with `u` the vector part.
    #[must_use]
    pub fn rotate(self, v: [T; 3]) -> [T; 3] {
        let u = self.vector();
        let t = scale(cross(u, v), two::<T>());
        add(add(v, scale(t, self.w)), cross(u, t))
    }

    /// The rotation matrix of this quaternion, assumed unit, as rows acting on
    /// column vectors.
    #[must_use]
    pub fn to_rotation_rows(self) -> [[T; 3]; 3] {
        let Self { x, y, z, w } = self;
        let two = two::<T>();
        let one = T::ONE;
        [
            [
                one - two * (y * y + z * z),
                two * (x * y - z * w),
                two * (x * z + y * w),
            ],
            [
                two * (x * y + z * w),
                one - two * (x * x + z * z),
                two * (y * z - x * w),
            ],
            [
                two * (x * z - y * w),
                two * (y * z + x * w),
                one - two * (x * x + y * y),
            ],
        ]
    }

    /// The unit quaternion of a rotation matrix given as rows, with a
    /// non-negative real part.
    ///
    /// Uses Shepperd's method: the largest of the four diagonal combinations
    /// selects the component recovered by a square root, which keeps the
    /// result well conditioned for every rotation. `rows` must be a rotation
    /// (orthonormal, determinant `+1`); other matrices yield an unspecified
    /// quaternion. [`Placement3::rotation`] validates before converting.
    /// Returns `None` only when `rows` contains non-finite values or the
    /// arithmetic overflows.
    #[must_use]
    pub fn from_rotation_rows(rows: [[T; 3]; 3]) -> Option<Self> {
        let [[m00, m01, m02], [m10, m11, m12], [m20, m21, m22]] = rows;
        let one = T::ONE;
        let trace = m00 + m11 + m22;
        let q = if trace >= m00 && trace >= m11 && trace >= m22 {
            let s = (one + trace).sqrt() * two::<T>(); // 4w
            Self::new(
                (m21 - m12) / s,
                (m02 - m20) / s,
                (m10 - m01) / s,
                s / four::<T>(),
            )
        } else if m00 >= m11 && m00 >= m22 {
            let s = (one + m00 - m11 - m22).sqrt() * two::<T>(); // 4x
            Self::new(
                s / four::<T>(),
                (m01 + m10) / s,
                (m02 + m20) / s,
                (m21 - m12) / s,
            )
        } else if m11 >= m22 {
            let s = (one + m11 - m00 - m22).sqrt() * two::<T>(); // 4y
            Self::new(
                (m01 + m10) / s,
                s / four::<T>(),
                (m12 + m21) / s,
                (m02 - m20) / s,
            )
        } else {
            let s = (one + m22 - m00 - m11).sqrt() * two::<T>(); // 4z
            Self::new(
                (m02 + m20) / s,
                (m12 + m21) / s,
                s / four::<T>(),
                (m10 - m01) / s,
            )
        };
        let q = q.normalize()?;
        Some(if q.w < T::ZERO {
            q.scale(T::ZERO - one)
        } else {
            q
        })
    }

    /// Spherical linear interpolation from `self` (`t = 0`) to `other`
    /// (`t = 1`) along the shorter arc, returning a unit quaternion.
    ///
    /// Both inputs are normalized first; `None` when either is degenerate.
    /// When the two rotations coincide to within `sqrt(MIN_POSITIVE)` in
    /// `sin θ`, the normalized linear interpolation is returned instead, which
    /// is the limit of slerp there. `t` outside `[0, 1]` extrapolates, and a
    /// NaN `t` yields `None`.
    #[must_use]
    pub fn slerp(self, other: Self, t: T) -> Option<Self> {
        let a = self.normalize()?;
        let mut b = other.normalize()?;
        let mut cos_theta = a.dot(b);
        if cos_theta < T::ZERO {
            b = b.scale(T::ZERO - T::ONE);
            cos_theta = T::ZERO - cos_theta;
        }
        if cos_theta > T::ONE {
            cos_theta = T::ONE;
        }
        let theta = cos_theta.acos();
        let (sin_theta, _) = theta.sin_cos();
        let (wa, wb) = if sin_theta <= T::MIN_POSITIVE.sqrt() {
            (T::ONE - t, t)
        } else {
            let (sa, _) = ((T::ONE - t) * theta).sin_cos();
            let (sb, _) = (t * theta).sin_cos();
            (sa / sin_theta, sb / sin_theta)
        };
        a.scale(wa).add(b.scale(wb)).normalize()
    }

    fn scale(self, factor: T) -> Self {
        Self::new(
            self.x * factor,
            self.y * factor,
            self.z * factor,
            self.w * factor,
        )
    }

    fn add(self, other: Self) -> Self {
        Self::new(
            self.x + other.x,
            self.y + other.y,
            self.z + other.z,
            self.w + other.w,
        )
    }
}

#[inline]
fn two<T: Real>() -> T {
    T::ONE + T::ONE
}

#[inline]
fn four<T: Real>() -> T {
    two::<T>() + two::<T>()
}

impl Placement3 {
    /// The placement that rotates by `rotation` (normalized first), then
    /// translates by `translation`; `None` when `rotation` is degenerate.
    #[must_use]
    pub fn from_rotation_translation(rotation: Quat<f64>, translation: [f64; 3]) -> Option<Self> {
        let r = rotation.normalize()?.to_rotation_rows();
        Some(Self {
            rows: [
                [r[0][0], r[0][1], r[0][2], translation[0]],
                [r[1][0], r[1][1], r[1][2], translation[1]],
                [r[2][0], r[2][1], r[2][2], translation[2]],
            ],
        })
    }

    /// The rotation of a rigid placement as a unit quaternion.
    ///
    /// The linear part must be a finite, right-handed orthonormal frame within
    /// `tolerance`, checked exactly as [`Self::try_from_orthonormal_axes`]
    /// checks its axes (the columns here); scale, shear and reflection are
    /// rejected rather than silently orthogonalized.
    ///
    /// # Errors
    /// Returns the [`FrameError`] of the first failed check.
    pub fn rotation(&self, tolerance: f64) -> Result<Quat<f64>, FrameError> {
        let r = &self.rows;
        let column = |c: usize| [r[0][c], r[1][c], r[2][c]];
        Self::try_from_orthonormal_axes(
            column(0),
            column(1),
            column(2),
            [r[0][3], r[1][3], r[2][3]],
            tolerance,
        )?;
        // For a finite frame that passed validation, Shepperd's selection
        // recovers a component of at least 1/2, so normalization succeeds.
        Ok(Quat::from_rotation_rows([
            [r[0][0], r[0][1], r[0][2]],
            [r[1][0], r[1][1], r[1][2]],
            [r[2][0], r[2][1], r[2][2]],
        ])
        .expect("a validated rotation frame always converts"))
    }
}

#[cfg(test)]
mod tests {
    use core::f64::consts::{FRAC_PI_2, PI};

    use super::Quat;
    use crate::{FrameError, Placement3};

    fn close(a: [f64; 3], b: [f64; 3], tolerance: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= tolerance)
    }

    fn close_q(a: Quat<f64>, b: Quat<f64>, tolerance: f64) -> bool {
        // q and -q are one rotation.
        let d = a.dot(b).abs();
        (d - 1.0).abs() <= tolerance
    }

    #[test]
    fn axis_angle_rotations_follow_the_right_hand_rule() {
        let z = Quat::from_axis_angle([0.0, 0.0, 2.0], FRAC_PI_2).expect("axis");
        assert!(
            close(z.rotate([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0], 1e-15),
            "+X turns to +Y about +Z"
        );
        let x = Quat::from_axis_angle([1.0, 0.0, 0.0], FRAC_PI_2).expect("axis");
        assert!(
            close(x.rotate([0.0, 1.0, 0.0]), [0.0, 0.0, 1.0], 1e-15),
            "+Y turns to +Z about +X"
        );
        assert_eq!(
            Quat::from_axis_angle([0.0; 3], 1.0),
            None,
            "degenerate axis"
        );
        let half =
            Quat::from_axis_angle([0.0_f32, 1.0, 0.0], core::f32::consts::PI).expect("f32 axis");
        let v = half.rotate([1.0, 0.0, 0.0]);
        assert!((v[0] + 1.0).abs() < 1e-6, "f32 half turn: {v:?}");
    }

    #[test]
    fn compose_applies_the_inner_rotation_first() {
        let about_x = Quat::from_axis_angle([1.0, 0.0, 0.0], FRAC_PI_2).expect("x");
        let about_z = Quat::from_axis_angle([0.0, 0.0, 1.0], FRAC_PI_2).expect("z");
        let v = [0.0, 1.0, 0.0];
        let composed = about_z.compose(about_x).rotate(v);
        let sequential = about_z.rotate(about_x.rotate(v));
        assert!(
            close(composed, sequential, 1e-15),
            "{composed:?} vs {sequential:?}"
        );
        assert!(close(composed, [0.0, 0.0, 1.0], 1e-15), "{composed:?}");
        let inverse = about_x.compose(about_x.conjugate());
        assert!(
            close_q(inverse, Quat::IDENTITY, 1e-15),
            "q q* is the identity"
        );
    }

    #[test]
    fn matrices_round_trip_for_every_shepperd_branch() {
        // Angles near 0, π/2 and π about each axis exercise all four branches.
        for axis in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, -2.0, 0.5],
        ] {
            for angle in [0.0, 0.3, FRAC_PI_2, 2.5, PI, -1.2] {
                let q = Quat::from_axis_angle(axis, angle).expect("axis");
                let rows = q.to_rotation_rows();
                let back = Quat::from_rotation_rows(rows).expect("rotation");
                assert!(back.w >= 0.0, "canonical sign");
                assert!(
                    close_q(q, back, 1e-14),
                    "{axis:?} {angle}: {q:?} vs {back:?}"
                );
                let v = [0.3, -1.7, 2.2];
                let by_matrix = [
                    rows[0][0] * v[0] + rows[0][1] * v[1] + rows[0][2] * v[2],
                    rows[1][0] * v[0] + rows[1][1] * v[1] + rows[1][2] * v[2],
                    rows[2][0] * v[0] + rows[2][1] * v[1] + rows[2][2] * v[2],
                ];
                assert!(
                    close(by_matrix, q.rotate(v), 1e-14),
                    "matrix agrees with rotate"
                );
            }
        }
    }

    #[test]
    fn placements_convert_through_validated_frames() {
        let q = Quat::from_axis_angle([0.2, 1.0, -0.4], 1.1).expect("axis");
        let t = [3.0, -2.0, 0.5];
        let placement = Placement3::from_rotation_translation(q, t).expect("unit");
        assert_eq!(placement.rows.map(|row| row[3]), t);
        let back = placement.rotation(1e-12).expect("rigid");
        assert!(close_q(q, back, 1e-14), "{q:?} vs {back:?}");
        let turned = Placement3::rotate_z_then_translate(FRAC_PI_2, 0.0, 0.0, 0.0)
            .rotation(1e-12)
            .expect("rigid");
        let expected = Quat::from_axis_angle([0.0, 0.0, 1.0], FRAC_PI_2).expect("z");
        assert!(close_q(turned, expected, 1e-15), "{turned:?}");

        let mut scaled = Placement3::IDENTITY;
        scaled.rows[0][0] = 2.0;
        assert!(
            matches!(
                scaled.rotation(1e-12),
                Err(FrameError::NonUnitAxis { axis: 0, .. })
            ),
            "scale is refused, not orthogonalized"
        );
        let mut mirrored = Placement3::IDENTITY;
        mirrored.rows[2][2] = -1.0;
        assert!(
            matches!(
                mirrored.rotation(1e-12),
                Err(FrameError::NonPositiveDeterminant { .. })
            ),
            "reflection is refused"
        );
        assert_eq!(
            Placement3::from_rotation_translation(Quat::new(0.0, 0.0, 0.0, 0.0), t),
            None,
            "degenerate rotation"
        );
    }

    #[test]
    fn slerp_takes_the_short_arc_at_constant_speed() {
        let a = Quat::IDENTITY;
        let b = Quat::from_axis_angle([0.0, 0.0, 1.0], FRAC_PI_2).expect("z");
        let mid = a.slerp(b, 0.5).expect("slerp");
        let expected = Quat::from_axis_angle([0.0, 0.0, 1.0], FRAC_PI_2 / 2.0).expect("z");
        assert!(close_q(mid, expected, 1e-15), "{mid:?}");
        assert!(close_q(a.slerp(b, 0.0).expect("t0"), a, 1e-15), "t = 0");
        assert!(close_q(a.slerp(b, 1.0).expect("t1"), b, 1e-15), "t = 1");
        let negated = Quat::new(-b.x, -b.y, -b.z, -b.w);
        let short = a.slerp(negated, 0.5).expect("slerp");
        assert!(close_q(short, expected, 1e-15), "-q takes the short arc");
        let same = b.slerp(b, 0.3).expect("coincident");
        assert!(close_q(same, b, 1e-15), "coincident rotations");
        assert!((mid.norm() - 1.0).abs() < 1e-15, "unit result");
        assert_eq!(
            a.slerp(Quat::new(0.0, 0.0, 0.0, 0.0), 0.5),
            None,
            "degenerate input"
        );
    }

    /// Pins output bits under `libm`, which is pure Rust and therefore
    /// portable; downstream golden fixtures depend on these operations.
    #[cfg(feature = "libm")]
    #[test]
    fn libm_results_are_pinned() {
        let bits = |v: Quat<f64>| [v.x, v.y, v.z, v.w].map(f64::to_bits);
        let q = Quat::from_axis_angle([0.3, -0.2, 0.9], 0.7).expect("axis");
        let r = Quat::from_axis_angle([-1.0, 0.4, 0.1], 2.1).expect("axis");
        assert_eq!(
            bits(q),
            [
                0x3fbb_2979_e22a_07f3,
                0xbfb2_1ba6_96c6_aff8,
                0x3fd4_5f1b_699f_85f6,
                0x3fee_0f57_5d0d_e5b7,
            ]
        );
        assert_eq!(
            bits(q.slerp(r, 0.37).expect("slerp")),
            [
                0xbfd2_77a3_b6e1_0881,
                0x3fb8_40a1_a540_daa0,
                0x3fd1_3c73_dc68_6d04,
                0x3fed_3ec1_0dd9_f7b3,
            ]
        );
        assert_eq!(
            q.to_rotation_rows().map(|row| row.map(f64::to_bits)),
            [
                [
                    0x3fe9_3207_ee1d_f341,
                    0xbfe3_9de4_4d58_6686,
                    0xbfb0_ba8e_92b4_2280,
                ],
                [
                    0x3fe2_a7f7_7e05_499e,
                    0x3fe8_cb8f_ed10_a736,
                    0xbfcf_4774_b8db_cd88,
                ],
                [
                    0x3fc9_a7ed_dd32_198c,
                    0x3fc3_c05b_00f6_72aa,
                    0x3fee_f594_ca10_a0b0,
                ],
            ]
        );
        let f = Quat::<f32>::from_axis_angle([0.3, -0.2, 0.9], 0.7).expect("axis");
        assert_eq!(
            [f.x, f.y, f.z, f.w].map(f32::to_bits),
            [0x3dd9_4bd0, 0xbd90_dd36, 0x3ea2_f8dc, 0x3f70_7abb]
        );
    }
}
