// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{Orientation, Orientation3d, dyadic_product_sum};

/// Exact sign of `dot(normal, point) - distance` for finite binary64 inputs.
///
/// Coefficients are used as authored, without normalization or a tolerance.
/// A floating-point error filter handles separated points; uncertain signs,
/// overflow and underflow fall back to exact dyadic accumulation. All finite
/// exponents are supported, even when the dot product is not representable.
/// Returns `None` for non-finite inputs. With a zero normal this still evaluates
/// the algebraic sign, but does not describe a geometric plane.
#[must_use]
pub fn plane_side(normal: [f64; 3], distance: f64, point: [f64; 3]) -> Option<Orientation3d> {
    if normal
        .into_iter()
        .chain(point)
        .chain([distance])
        .any(|x| !x.is_finite())
    {
        return None;
    }
    let products = core::array::from_fn::<_, 3, _>(|i| normal[i] * point[i]);
    let sum = (products[0] + products[1]) + products[2];
    let value = sum - distance;
    let magnitude = products.iter().map(|x| x.abs()).sum::<f64>() + distance.abs();
    // Three products and three additions have error <= gamma(6) times the
    // exact magnitude sum. 16 unit roundoffs also cover rounding that sum and
    // this bound. Underflowing products/bounds use the exact path instead.
    let bound = (8.0 * f64::EPSILON) * magnitude;
    if products
        .iter()
        .enumerate()
        .all(|(i, p)| p.is_normal() || normal[i] == 0.0 || point[i] == 0.0)
        && value.is_finite()
        && bound.is_normal()
        && value.abs() > bound
    {
        return Some(if value > 0.0 {
            Orientation3d::Above
        } else {
            Orientation3d::Below
        });
    }
    let sign = dyadic_product_sum(&[
        (normal[0], point[0]),
        (normal[1], point[1]),
        (normal[2], point[2]),
        (-distance, 1.0),
    ])?;
    Some(match sign {
        Orientation::Ccw => Orientation3d::Above,
        Orientation::Cw => Orientation3d::Below,
        Orientation::Collinear => Orientation3d::Coplanar,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_oblique_boundary_and_adjacent_values_have_exact_ownership() {
        for normal in [[1.0, 1.0, 1.0], [2.0, 2.0, 2.0], [-1.0, -1.0, -1.0]] {
            let distance = 3.0 * normal[0];
            assert_eq!(
                plane_side(normal, distance, [1.0; 3]),
                Some(Orientation3d::Coplanar)
            );
            let above = if normal[0] > 0.0 {
                Orientation3d::Above
            } else {
                Orientation3d::Below
            };
            let below = if normal[0] > 0.0 {
                Orientation3d::Below
            } else {
                Orientation3d::Above
            };
            assert_eq!(
                plane_side(normal, distance, [1.0, 1.0, 1.0_f64.next_up()]),
                Some(above)
            );
            assert_eq!(
                plane_side(normal, distance, [1.0, 1.0, 1.0_f64.next_down()]),
                Some(below)
            );
        }
    }

    #[test]
    fn cancellation_overflow_underflow_and_invalid_inputs() {
        let tiny = f64::from_bits(1);
        assert_eq!(
            plane_side([1.0; 3], f64::MAX, [f64::MAX, tiny, 0.0]),
            Some(Orientation3d::Above)
        );
        assert_eq!(
            plane_side([f64::MAX, -f64::MAX, 1.0], 0.0, [f64::MAX, f64::MAX, tiny]),
            Some(Orientation3d::Above)
        );
        assert_eq!(
            plane_side([tiny, 0.0, 0.0], 0.0, [tiny, 0.0, 0.0]),
            Some(Orientation3d::Above)
        );
        assert_eq!(
            plane_side([-tiny, 0.0, 0.0], 0.0, [tiny, 0.0, 0.0]),
            Some(Orientation3d::Below)
        );
        assert_eq!(plane_side([1.0; 3], f64::NAN, [0.0; 3]), None);
        assert_eq!(plane_side([1.0; 3], 0.0, [f64::INFINITY; 3]), None);
        assert_eq!(plane_side([f64::INFINITY; 3], 0.0, [0.0; 3]), None);
    }

    #[test]
    fn filtered_sign_matches_exact_accumulation_across_exponents() {
        let mut state = 17_u64;
        let mut value = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            f64::from_bits((state & 0x800f_ffff_ffff_ffff) | (((state >> 52) % 2047) << 52))
        };
        for _ in 0..4096 {
            let normal = [value(), value(), value()];
            let point = [value(), value(), value()];
            let distance = value();
            let exact = dyadic_product_sum(&[
                (normal[0], point[0]),
                (normal[1], point[1]),
                (normal[2], point[2]),
                (-distance, 1.0),
            ])
            .unwrap();
            let expected = match exact {
                Orientation::Ccw => Orientation3d::Above,
                Orientation::Cw => Orientation3d::Below,
                Orientation::Collinear => Orientation3d::Coplanar,
            };
            assert_eq!(plane_side(normal, distance, point), Some(expected));
        }
    }
}
