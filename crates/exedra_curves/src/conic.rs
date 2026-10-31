// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lines and conic arcs as exact NURBS curves.

use alloc::vec::Vec;
use core::f64::consts::{FRAC_PI_2, TAU};

use exedra_math::Real;

use crate::NurbsCurve;

impl<const N: usize> NurbsCurve<N> {
    /// The straight segment from `a` to `b` over the domain `[0, 1]`, of
    /// order 2, its parameter proportional to length. `None` unless both
    /// ends are finite.
    #[must_use]
    pub fn line(a: [f64; N], b: [f64; N]) -> Option<Self> {
        Self::new(2, alloc::vec![0.0, 0.0, 1.0, 1.0], alloc::vec![a, b], None).ok()
    }

    /// The arc `center + cos(t) x_axis + sin(t) y_axis` for `t` from
    /// `angles[0]` to `angles[1]`, exactly, as a rational quadratic
    /// (Piegl and Tiller, *The NURBS Book*, 2nd ed., algorithm A7.1).
    ///
    /// Perpendicular axes of equal length give a circle of that radius;
    /// perpendicular axes of different lengths, an ellipse with those
    /// semiaxes; any two independent axes, the ellipse that is their affine
    /// image of a circle, since NURBS keep their shape under affine maps.
    ///
    /// The arc is split into one piece per quarter turn or part of one,
    /// each with weights `1, cos(d / 2), 1` for its sweep `d`, and the
    /// domain is `angles`, with a knot at every piece's start angle: the
    /// curve passes through the arc's point at each knot, but between
    /// knots its parameter is not proportional to angle.
    ///
    /// The angles' sines and cosines come from `exedra_math`'s `sin_cos`,
    /// so the controls can differ in their last bits between platforms
    /// unless its `libm` backend is used.
    ///
    /// `None` unless the inputs are finite and the sweep `angles[1] -
    /// angles[0]` is positive and at most a full turn.
    #[must_use]
    pub fn arc(
        center: [f64; N],
        x_axis: [f64; N],
        y_axis: [f64; N],
        angles: [f64; 2],
    ) -> Option<Self> {
        let [start, end] = angles;
        let sweep = end - start;
        if !(sweep > 0.0 && sweep <= TAU) || ![center, x_axis, y_axis].iter().all(|v| finite(*v)) {
            return None;
        }
        // Quarter turns or parts of one; a turn rounded just past a
        // quarter's multiple still counts as that many.
        let pieces = (1..=4)
            .find(|&k: &u32| sweep <= f64::from(k) * FRAC_PI_2 * (1.0 + 1e-15))
            .unwrap_or(4);
        let step = sweep / f64::from(pieces);
        // Each piece is built from the directions at its rounded end
        // angles alone: its middle control is where the tangents there
        // meet, `(u0 + u1) / (1 + cos d)` for the turn `d` between them,
        // weighted `cos(d / 2) = sqrt((1 + cos d) / 2)`, with `cos d` taken
        // from the directions themselves. So each piece is exact however
        // coarsely its angles round, as they do far from zero.
        let direction = |t: f64| {
            let (sin, cos) = <f64 as Real>::sin_cos(t);
            [cos, sin]
        };
        let at = |[cos, sin]: [f64; 2]| -> [f64; N] {
            core::array::from_fn(|i| center[i] + (cos * x_axis[i] + sin * y_axis[i]))
        };
        let mut points: Vec<[f64; N]> = Vec::with_capacity(2 * pieces as usize + 1);
        let mut weights = Vec::with_capacity(points.capacity());
        let mut knots = alloc::vec![start; 3];
        let mut from = direction(start);
        for k in 1..=pieces {
            let a = if k == pieces {
                end
            } else {
                start + step * f64::from(k)
            };
            let to = direction(a);
            let turn = 1.0 + from[0] * to[0] + from[1] * to[1];
            let middle = [(from[0] + to[0]) / turn, (from[1] + to[1]) / turn];
            points.push(at(from));
            weights.push(1.0);
            points.push(at(middle));
            weights.push(<f64 as Real>::sqrt(0.5 * turn));
            if k < pieces {
                knots.extend([a, a]);
            }
            from = to;
        }
        points.push(at(from));
        weights.push(1.0);
        knots.extend([end; 3]);
        Self::new(3, knots, points, Some(weights)).ok()
    }
}

fn finite<const N: usize>(v: [f64; N]) -> bool {
    v.iter().all(|c| c.is_finite())
}

#[cfg(test)]
mod tests {
    use core::f64::consts::{FRAC_PI_2, PI, TAU};

    use crate::NurbsCurve;

    #[test]
    fn arcs_are_exact_circles_and_ellipses() {
        let center = [1.0, -2.0, 0.5];
        for (x, y, a, b) in [
            ([2.0, 0.0, 0.0], [0.0, 2.0, 0.0], 2.0, 2.0),
            ([3.0, 0.0, 0.0], [0.0, 0.0, 1.0], 3.0, 1.0),
        ] {
            for angles in [
                [0.0, FRAC_PI_2],
                [0.3, 2.0],
                [-1.0, 3.5],
                [0.0, TAU],
                [1.0, 1.0 + TAU],
            ] {
                let arc = NurbsCurve::arc(center, x, y, angles).unwrap();
                assert_eq!(arc.domain(), angles);
                let view = arc.view();
                for i in 0..=200 {
                    let t = angles[0] + (angles[1] - angles[0]) * f64::from(i) / 200.0;
                    let p = view.evaluate(t);
                    // On the conic: (u / a)^2 + (v / b)^2 = 1 in its axes.
                    let d: [f64; 3] = core::array::from_fn(|k| p[k] - center[k]);
                    let u = exedra_math::dot(d, x) / a;
                    let v = exedra_math::dot(d, y) / b;
                    let r = (u / a).hypot(v / b);
                    assert!((r - 1.0).abs() < 1e-14, "{angles:?} at {t}: {r}");
                }
                // The ends, and every knot, are the conic's points there.
                for &t in arc.knots() {
                    let want: [f64; 3] =
                        core::array::from_fn(|k| center[k] + t.cos() * x[k] + t.sin() * y[k]);
                    let got = view.evaluate(t);
                    assert!(exedra_math::norm(exedra_math::sub(got, want)) < 1e-14);
                }
            }
        }
    }

    #[test]
    fn arcs_take_one_piece_per_quarter_turn() {
        let pieces = |sweep: f64| {
            let arc = NurbsCurve::arc([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, sweep]);
            (arc.unwrap().points().len() - 1) / 2
        };
        assert_eq!(pieces(FRAC_PI_2), 1);
        assert_eq!(pieces(PI), 2);
        assert_eq!(pieces(PI + 0.1), 3);
        assert_eq!(pieces(TAU), 4);
        // A full turn closes.
        let full = NurbsCurve::arc([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, TAU]).unwrap();
        assert_eq!(full.points().first(), Some(&[1.0, 0.0, 0.0]));
        let last = full.points().last().unwrap();
        assert!((last[0] - 1.0).abs() < 1e-15 && last[1].abs() < 1e-15);
    }

    #[test]
    fn degenerate_arcs_and_lines_are_refused() {
        let (x, y) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!(NurbsCurve::arc([0.0; 3], x, y, [1.0, 1.0]).is_none());
        assert!(NurbsCurve::arc([0.0; 3], x, y, [0.0, 7.0]).is_none());
        assert!(NurbsCurve::arc([0.0; 3], x, y, [0.0, f64::NAN]).is_none());
        assert!(NurbsCurve::arc([f64::INFINITY, 0.0, 0.0], x, y, [0.0, 1.0]).is_none());
        let line = NurbsCurve::line([0.0; 3], [2.0, 4.0, 6.0]).unwrap();
        assert_eq!(line.view().evaluate(0.25), [0.5, 1.0, 1.5]);
        assert!(NurbsCurve::line([0.0; 3], [f64::NAN, 0.0, 0.0]).is_none());
    }

    #[test]
    fn plane_arcs_are_space_arcs_flattened() {
        let plane = NurbsCurve::arc([1.0, 2.0], [3.0, 0.0], [0.0, 1.0], [0.25, 4.0]).unwrap();
        let space = NurbsCurve::arc(
            [1.0, 2.0, 0.0],
            [3.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.25, 4.0],
        )
        .unwrap();
        assert_eq!(plane.knots(), space.knots());
        assert_eq!(plane.weights(), space.weights());
        for (p, q) in plane.points().iter().zip(space.points()) {
            assert_eq!(*p, [q[0], q[1]]);
        }
    }

    #[test]
    fn arcs_far_from_angle_zero_stay_on_the_circle() {
        // Angles near 2^50 are a quarter apart, coarser than the pieces'
        // middles; every piece is still an exact arc between its ends.
        let start = 2.0_f64.powi(50);
        for sweep in [1.25, 3.0, TAU] {
            let arc =
                NurbsCurve::arc([0.0; 2], [1.0, 0.0], [0.0, 1.0], [start, start + sweep]).unwrap();
            let view = arc.view();
            let [a, b] = arc.domain();
            let mut t = a;
            while t <= b {
                let [x, y] = view.evaluate(t);
                let r = x.hypot(y);
                assert!((r - 1.0).abs() < 1e-14, "{sweep} at {t}: {r}");
                t = t.next_up();
            }
        }
    }
}
