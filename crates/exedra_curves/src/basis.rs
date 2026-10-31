// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cubic bases as explicit matrices.

/// A cubic basis: how four controls weight a point of one span.
///
/// A span's point at parameter `u` in `[0, 1]` is
///
/// ```text
/// P(u) = [u^3  u^2  u  1] * M * [C0 C1 C2 C3]^T
/// ```
///
/// for the basis matrix `M` ([`Self::matrix`]) and the span's four controls
/// `C0..C3`. The matrices are the `RenderMan` Interface's (`RiBezierBasis`,
/// `RiBSplineBasis`, `RiCatmullRomBasis`, `RiHermiteBasis`; `RenderMan`
/// Interface Specification 3.2, section 5.5, "Cubic basis"), which is also
/// the table `OpenUSD`'s `UsdGeomBasisCurves` documents ("Cubic Vertex
/// Interpolation", `pxr/usd/usdGeom/basisCurves.h`) and its Storm renderer
/// evaluates (`pxr/imaging/hdSt/shaders/basisCurves.glslfx`,
/// `Curves.BezierBasis`, `Curves.BsplineBasis`, `Curves.CatmullRomBasis`;
/// Storm lists each span's controls in reverse and its weights to match).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CubicBasis {
    /// Bernstein polynomials: the span passes through `C0` and `C3`, with
    /// `C1` and `C2` its tangent handles. Consecutive spans share a control
    /// ([`Self::step`] 3).
    ///
    /// ```text
    ///     | -1  3 -3  1 |
    /// M = |  3 -6  3  0 |
    ///     | -3  3  0  0 |
    ///     |  1  0  0  0 |
    /// ```
    Bezier,
    /// The uniform cubic B-spline: C2-continuous, through none of its
    /// controls. Consecutive spans overlap by three ([`Self::step`] 1).
    ///
    /// ```text
    ///           | -1  3 -3  1 |
    /// M = 1/6 * |  3 -6  3  0 |
    ///           | -3  0  3  0 |
    ///           |  1  4  1  0 |
    /// ```
    BSpline,
    /// The uniform Catmull-Rom spline: C1, through `C1` at `u = 0` and `C2`
    /// at `u = 1` with tangents `(C2 - C0) / 2` and `(C3 - C1) / 2`
    /// ([`Self::step`] 1).
    ///
    /// ```text
    ///           | -1  3 -3  1 |
    /// M = 1/2 * |  2 -5  4 -1 |
    ///           | -1  0  1  0 |
    ///           |  0  2  0  0 |
    /// ```
    CatmullRom,
    /// Cubic Hermite: the controls are `[P0, T0, P1, T1]`, two points and
    /// their tangents; the span runs from `P0` to `P1`. Spans of a Hermite
    /// chain share a point and its tangent ([`Self::step`] 2, over
    /// interleaved point and tangent controls).
    ///
    /// ```text
    ///     |  2  1 -2  1 |
    /// M = | -3 -2  3 -1 |
    ///     |  0  1  0  0 |
    ///     |  1  0  0  0 |
    /// ```
    Hermite,
}

const BEZIER: [[f64; 4]; 4] = [
    [-1.0, 3.0, -3.0, 1.0],
    [3.0, -6.0, 3.0, 0.0],
    [-3.0, 3.0, 0.0, 0.0],
    [1.0, 0.0, 0.0, 0.0],
];

const SIXTH: f64 = 1.0 / 6.0;

const BSPLINE: [[f64; 4]; 4] = [
    [-SIXTH, 3.0 * SIXTH, -3.0 * SIXTH, SIXTH],
    [3.0 * SIXTH, -6.0 * SIXTH, 3.0 * SIXTH, 0.0],
    [-3.0 * SIXTH, 0.0, 3.0 * SIXTH, 0.0],
    [SIXTH, 4.0 * SIXTH, SIXTH, 0.0],
];

const CATMULL_ROM: [[f64; 4]; 4] = [
    [-0.5, 1.5, -1.5, 0.5],
    [1.0, -2.5, 2.0, -0.5],
    [-0.5, 0.0, 0.5, 0.0],
    [0.0, 1.0, 0.0, 0.0],
];

const HERMITE: [[f64; 4]; 4] = [
    [2.0, 1.0, -2.0, 1.0],
    [-3.0, -2.0, 3.0, -1.0],
    [0.0, 1.0, 0.0, 0.0],
    [1.0, 0.0, 0.0, 0.0],
];

impl CubicBasis {
    /// The basis matrix `M`, row by row: row 0 holds the `u^3`
    /// coefficients of the four controls, row 3 the constant terms.
    #[must_use]
    pub const fn matrix(self) -> [[f64; 4]; 4] {
        match self {
            Self::Bezier => BEZIER,
            Self::BSpline => BSPLINE,
            Self::CatmullRom => CATMULL_ROM,
            Self::Hermite => HERMITE,
        }
    }

    /// How many controls the next span of a chain starts after this one's
    /// first: 3 for Bezier, 1 for the B-spline and Catmull-Rom (`OpenUSD`'s
    /// `vstep`, `pxr/usd/usdGeom/basisCurves.h`, "Segment Indexing"), 2 for
    /// Hermite over interleaved `[P, T]` controls.
    #[must_use]
    pub const fn step(self) -> usize {
        match self {
            Self::Bezier => 3,
            Self::BSpline | Self::CatmullRom => 1,
            Self::Hermite => 2,
        }
    }

    /// The weight of each control at `u`: `[u^3 u^2 u 1] * M`.
    ///
    /// Evaluated as `((m0 * u + m1) * u + m2) * u + m3` per column, a fixed
    /// order, so equal inputs give equal bits.
    ///
    /// This is the power form, not a factored Bernstein form: near a
    /// span's end a weight that is mathematically tiny comes out as
    /// rounding of either sign (the Bezier `C0` weight at
    /// `u = 1 - 1e-14` is `-2.2e-16`, not `1e-42`), so weights can leave
    /// `[0, 1]` by that much. An evaluated point stays within a few units
    /// in the last place of its largest control, wherever the controls
    /// are translated.
    #[must_use]
    pub fn weights(self, u: f64) -> [f64; 4] {
        let m = self.matrix();
        core::array::from_fn(|c| ((m[0][c] * u + m[1][c]) * u + m[2][c]) * u + m[3][c])
    }

    /// The weight of each control in the derivative `dP/du` at `u`:
    /// `[3u^2 2u 1 0] * M`.
    #[must_use]
    pub fn derivative_weights(self, u: f64) -> [f64; 4] {
        let m = self.matrix();
        core::array::from_fn(|c| (3.0 * m[0][c] * u + 2.0 * m[1][c]) * u + m[2][c])
    }

    /// The weight of each control in the second derivative `d2P/du2` at
    /// `u`: `[6u 2 0 0] * M`.
    #[must_use]
    pub fn second_derivative_weights(self, u: f64) -> [f64; 4] {
        let m = self.matrix();
        core::array::from_fn(|c| 6.0 * m[0][c] * u + 2.0 * m[1][c])
    }

    /// The span's point at `u` from its four controls, of any dimension
    /// (a position, a width, a color).
    #[must_use]
    pub fn evaluate<const N: usize>(self, controls: [[f64; N]; 4], u: f64) -> [f64; N] {
        combine(self.weights(u), controls)
    }

    /// The span's derivative `dP/du` at `u` from its four controls.
    #[must_use]
    pub fn derivative<const N: usize>(self, controls: [[f64; N]; 4], u: f64) -> [f64; N] {
        combine(self.derivative_weights(u), controls)
    }

    /// The span's second derivative `d2P/du2` at `u` from its four
    /// controls.
    #[must_use]
    pub fn second_derivative<const N: usize>(self, controls: [[f64; N]; 4], u: f64) -> [f64; N] {
        combine(self.second_derivative_weights(u), controls)
    }

    /// The span's point and its first two derivatives at `u`, each with
    /// respect to the span parameter `u` (not a chain's or a format's own
    /// parameter): the same values, bit for bit, as [`Self::evaluate`],
    /// [`Self::derivative`] and [`Self::second_derivative`].
    #[must_use]
    pub fn evaluation<const N: usize>(self, controls: [[f64; N]; 4], u: f64) -> CurveEvaluation<N> {
        CurveEvaluation {
            point: self.evaluate(controls, u),
            first: self.derivative(controls, u),
            second: self.second_derivative(controls, u),
        }
    }
}

/// A curve's point and its first two derivatives at one parameter, each
/// with respect to the parameter the producing function names.
///
/// The fields follow `exedra_analytic`'s `CurveEvaluation`, generalized
/// to values of any dimension, so positions and attributes evaluate alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveEvaluation<const N: usize> {
    /// The point (or value).
    pub point: [f64; N],
    /// The first derivative.
    pub first: [f64; N],
    /// The second derivative.
    pub second: [f64; N],
}

/// `w0 * c0 + w1 * c1 + w2 * c2 + w3 * c3`, summed in that order.
fn combine<const N: usize>(weights: [f64; 4], controls: [[f64; N]; 4]) -> [f64; N] {
    core::array::from_fn(|k| {
        weights[0] * controls[0][k]
            + weights[1] * controls[1][k]
            + weights[2] * controls[2][k]
            + weights[3] * controls[3][k]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [CubicBasis; 4] = [
        CubicBasis::Bezier,
        CubicBasis::BSpline,
        CubicBasis::CatmullRom,
        CubicBasis::Hermite,
    ];

    #[test]
    fn interpolating_bases_reproduce_constants() {
        // Bezier, B-spline and Catmull-Rom weights sum to one at every `u`
        // (affine invariance); Hermite's point weights do.
        for basis in ALL {
            for k in 0..=8 {
                let u = f64::from(k) / 8.0;
                let w = basis.weights(u);
                let sum = match basis {
                    CubicBasis::Hermite => w[0] + w[2],
                    _ => w.iter().sum(),
                };
                assert!((sum - 1.0).abs() < 1e-15, "{basis:?} at {u}: {sum}");
            }
        }
    }

    #[test]
    fn hand_computed_points() {
        let c = [[0.0], [1.0], [3.0], [6.0]];
        // Bezier at 1/2: (C0 + 3 C1 + 3 C2 + C3) / 8.
        assert_eq!(
            CubicBasis::Bezier.evaluate(c, 0.5),
            [(3.0 + 9.0 + 6.0) / 8.0]
        );
        // B-spline at 0: (C0 + 4 C1 + C2) / 6; at 1: (C1 + 4 C2 + C3) / 6.
        let near = |a: [f64; 1], b: f64| (a[0] - b).abs() < 1e-15;
        assert!(near(
            CubicBasis::BSpline.evaluate(c, 0.0),
            (4.0 + 3.0) / 6.0
        ));
        assert!(near(
            CubicBasis::BSpline.evaluate(c, 1.0),
            (1.0 + 12.0 + 6.0) / 6.0
        ));
        // Catmull-Rom passes through C1 and C2 with tangents (C2 - C0) / 2
        // and (C3 - C1) / 2.
        assert_eq!(CubicBasis::CatmullRom.evaluate(c, 0.0), [1.0]);
        assert_eq!(CubicBasis::CatmullRom.evaluate(c, 1.0), [3.0]);
        assert_eq!(CubicBasis::CatmullRom.derivative(c, 0.0), [1.5]);
        assert_eq!(CubicBasis::CatmullRom.derivative(c, 1.0), [2.5]);
        // Hermite: P0 = 0, T0 = 1, P1 = 3, T1 = 6.
        assert_eq!(CubicBasis::Hermite.evaluate(c, 0.0), [0.0]);
        assert_eq!(CubicBasis::Hermite.evaluate(c, 1.0), [3.0]);
        assert_eq!(CubicBasis::Hermite.derivative(c, 0.0), [1.0]);
        assert_eq!(CubicBasis::Hermite.derivative(c, 1.0), [6.0]);
        // Bezier end tangents: 3 (C1 - C0) and 3 (C3 - C2).
        assert_eq!(CubicBasis::Bezier.derivative(c, 0.0), [3.0]);
        assert_eq!(CubicBasis::Bezier.derivative(c, 1.0), [9.0]);
    }

    #[test]
    fn steps_match_the_shared_controls() {
        assert_eq!(CubicBasis::Bezier.step(), 3);
        assert_eq!(CubicBasis::BSpline.step(), 1);
        assert_eq!(CubicBasis::CatmullRom.step(), 1);
        assert_eq!(CubicBasis::Hermite.step(), 2);
    }

    #[test]
    fn translated_points_stay_within_a_few_ulps() {
        // Bezier against de Casteljau's convex combinations, near both ends
        // and in the middle, with controls translated far from the origin.
        let lerp = |a: f64, b: f64, u: f64| a + (b - a) * u;
        for offset in [0.0, 1.0e3, 1.0e6] {
            let c = [
                [offset + 0.25],
                [offset - 0.5],
                [offset + 1.0],
                [offset - 0.75],
            ];
            let ulp = f64::EPSILON * (offset + 1.0);
            for u in [1e-12, 0.25, 0.5, 0.75, 1.0 - 1e-12, 1.0 - 1e-14] {
                let [a, b, d] = [0, 1, 2].map(|i| lerp(c[i][0], c[i + 1][0], u));
                let [e, f] = [lerp(a, b, u), lerp(b, d, u)];
                let reference = lerp(e, f, u);
                let got = CubicBasis::Bezier.evaluate(c, u)[0];
                assert!(
                    (got - reference).abs() <= 16.0 * ulp,
                    "{offset} {u}: {got} {reference}"
                );
            }
        }
    }

    #[test]
    fn second_derivatives_match_hand_values_and_differences() {
        let c = [[0.0], [1.0], [3.0], [6.0]];
        // Bezier: P'' = 6 (C0 - 2 C1 + C2) at 0 and 6 (C1 - 2 C2 + C3) at 1.
        assert_eq!(CubicBasis::Bezier.second_derivative(c, 0.0), [6.0]);
        assert_eq!(CubicBasis::Bezier.second_derivative(c, 1.0), [6.0]);
        // Every basis: the second derivative is the derivative's slope,
        // exact for a cubic by a central difference.
        let h = 1.0 / 1024.0;
        for basis in ALL {
            for k in 1..8 {
                let u = f64::from(k) / 8.0;
                let e = basis.evaluation(c, u);
                let slope =
                    (basis.derivative(c, u + h)[0] - basis.derivative(c, u - h)[0]) / (2.0 * h);
                assert!((e.second[0] - slope).abs() < 1e-9, "{basis:?} at {u}");
                assert_eq!(e.point, basis.evaluate(c, u));
                assert_eq!(e.first, basis.derivative(c, u));
            }
        }
    }
}
