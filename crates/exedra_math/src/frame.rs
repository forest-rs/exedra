// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{Placement3, dot};

/// Why authored axes cannot define a right-handed orthonormal frame.
/// Axis indices are `0 = X`, `1 = Y`, `2 = Z`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum FrameError {
    /// The dimensionless tolerance is not finite or outside `[0, 1)`.
    InvalidTolerance,
    /// An origin coordinate is not finite.
    NonFiniteOrigin,
    /// An authored axis contains a nonfinite coordinate.
    NonFiniteAxis {
        /// Index of the authored axis.
        axis: usize,
    },
    /// An axis's squared length differs from one by more than the tolerance.
    NonUnitAxis {
        /// Index of the authored axis.
        axis: usize,
        /// Measured squared length; may overflow for very large finite axes.
        squared_length: f64,
    },
    /// Two axes have an absolute dot product greater than the tolerance.
    NonOrthogonalAxes {
        /// First authored axis index.
        first: usize,
        /// Second authored axis index.
        second: usize,
        /// Measured dot product.
        dot_product: f64,
    },
    /// The checked basis is singular or left-handed.
    NonPositiveDeterminant {
        /// Measured determinant of the authored basis.
        determinant: f64,
    },
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTolerance => f.write_str("frame tolerance must be finite and in [0, 1)"),
            Self::NonFiniteOrigin => f.write_str("frame origin must be finite"),
            Self::NonFiniteAxis { axis } => write!(f, "frame axis {axis} must be finite"),
            Self::NonUnitAxis {
                axis,
                squared_length,
            } => write!(
                f,
                "frame axis {axis} has squared length {squared_length}, expected one"
            ),
            Self::NonOrthogonalAxes {
                first,
                second,
                dot_product,
            } => write!(
                f,
                "frame axes {first} and {second} have dot product {dot_product}, expected zero"
            ),
            Self::NonPositiveDeterminant { determinant } => {
                write!(f, "frame determinant {determinant} must be positive")
            }
        }
    }
}
impl core::error::Error for FrameError {}

impl Placement3 {
    /// Constructs a finite, right-handed orthonormal frame from authored axes.
    ///
    /// Checks absolute errors in squared axis lengths and pairwise dot products
    /// against `tolerance`, then requires a strictly positive determinant.
    /// The dimensionless tolerance must be finite and in `[0, 1)`. Zero requests
    /// exact comparisons. No axis is normalized, inferred, swapped or repaired;
    /// successful output preserves all supplied values, as [`Self::from_axes`]
    /// does. Editing the returned placement requires checking it again.
    ///
    /// Use [`Self::from_axes`] for intentional scale, shear or reflection. This
    /// constructor expresses the stricter contract of a geometric frame.
    ///
    /// # Errors
    /// Returns the first failed check: tolerance, finite origin/axes, unit axes,
    /// perpendicular axes, then positive determinant. Axis checks use X/Y/Z order.
    ///
    /// # Example
    ///
    /// An elevation drawn across and up, extruded toward the viewer:
    /// ```
    /// use exedra_math::Placement3;
    /// let frame = Placement3::try_from_orthonormal_axes(
    ///     [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0],
    ///     [2.0, 0.0, 3.0], 1e-12,
    /// )?;
    /// assert_eq!(frame.linear_determinant(), 1.0);
    /// # Ok::<(), exedra_math::FrameError>(())
    /// ```
    pub fn try_from_orthonormal_axes(
        x_axis: [f64; 3],
        y_axis: [f64; 3],
        z_axis: [f64; 3],
        origin: [f64; 3],
        tolerance: f64,
    ) -> Result<Self, FrameError> {
        if !tolerance.is_finite() || !(0.0..1.0).contains(&tolerance) {
            return Err(FrameError::InvalidTolerance);
        }
        if origin.iter().any(|v| !v.is_finite()) {
            return Err(FrameError::NonFiniteOrigin);
        }
        let axes = [x_axis, y_axis, z_axis];
        for (axis, values) in axes.iter().enumerate() {
            if values.iter().any(|v| !v.is_finite()) {
                return Err(FrameError::NonFiniteAxis { axis });
            }
        }
        for (axis, &values) in axes.iter().enumerate() {
            let squared_length = dot(values, values);
            if !squared_length.is_finite() || (squared_length - 1.0).abs() > tolerance {
                return Err(FrameError::NonUnitAxis {
                    axis,
                    squared_length,
                });
            }
        }
        for (first, second) in [(0, 1), (0, 2), (1, 2)] {
            let dot_product = dot(axes[first], axes[second]);
            if dot_product.abs() > tolerance {
                return Err(FrameError::NonOrthogonalAxes {
                    first,
                    second,
                    dot_product,
                });
            }
        }
        let placement = Self::from_axes(x_axis, y_axis, z_axis, origin);
        let determinant = placement.linear_determinant();
        if determinant <= 0.0 {
            return Err(FrameError::NonPositiveDeterminant { determinant });
        }
        Ok(placement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const X: [f64; 3] = [1.0, 0.0, 0.0];
    const Y: [f64; 3] = [0.0, 1.0, 0.0];
    const Z: [f64; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn authored_elevation_and_rotations_preserve_their_axes() {
        for angle in [0.0, 0.1, 1.7, -2.3] {
            let frame = Placement3::rotate_z_then_translate(angle, 2.0, 3.0, 4.0);
            let [a, b, c] = [0, 1, 2].map(|i| frame.rows.map(|r| r[i]));
            assert_eq!(
                Placement3::try_from_orthonormal_axes(a, b, c, [2.0, 3.0, 4.0], 1e-12),
                Ok(frame)
            );
        }
        assert!(
            Placement3::try_from_orthonormal_axes(X, Z, [0.0, -1.0, 0.0], [0.0; 3], 0.0).is_ok()
        );
    }

    #[test]
    fn errors_identify_the_authored_mistake() {
        let make = |axes: [[f64; 3]; 3], tolerance| {
            Placement3::try_from_orthonormal_axes(axes[0], axes[1], axes[2], [0.0; 3], tolerance)
        };
        assert_eq!(
            make([X, Z, Y], 0.0),
            Err(FrameError::NonPositiveDeterminant { determinant: -1.0 })
        );
        assert_eq!(
            make([X, X, Z], 0.0),
            Err(FrameError::NonOrthogonalAxes {
                first: 0,
                second: 1,
                dot_product: 1.0
            })
        );
        for length in [0.0, 2.0, f64::MAX] {
            assert!(matches!(
                make([X, [0.0, length, 0.0], Z], 1e-12),
                Err(FrameError::NonUnitAxis { axis: 1, .. })
            ));
        }
        assert_eq!(
            make([X, Y, [0.0, 0.0, f64::NAN]], 1e-12),
            Err(FrameError::NonFiniteAxis { axis: 2 })
        );
        for tolerance in [-1.0, 1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                make([X, Y, Z], tolerance),
                Err(FrameError::InvalidTolerance)
            );
        }
        assert_eq!(
            Placement3::try_from_orthonormal_axes(X, Y, Z, [f64::INFINITY, 0.0, 0.0], 0.0),
            Err(FrameError::NonFiniteOrigin)
        );
        let almost_x = [1.0, 1e-8, 0.0];
        assert!(make([almost_x, Y, Z], 1e-7).is_ok());
        assert!(matches!(
            make([almost_x, Y, Z], 1e-9),
            Err(FrameError::NonOrthogonalAxes { .. })
        ));
    }
}
