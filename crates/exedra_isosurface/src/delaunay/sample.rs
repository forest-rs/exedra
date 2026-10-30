// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Field queries, counted, and the searches built on them.

use core::cell::Cell;

use exedra_implicit::{Aabb, Crease, Dual, Interval, ScalarField, Tie};
use exedra_math::{cross, dot, sub};

use super::FieldWork;

/// A field with every query counted.
pub(super) struct Sampler<'a> {
    field: &'a dyn ScalarField,
    work: Cell<FieldWork>,
}

impl<'a> Sampler<'a> {
    pub(super) fn new(field: &'a dyn ScalarField) -> Self {
        Self {
            field,
            work: Cell::new(FieldWork::default()),
        }
    }

    pub(super) fn work(&self) -> FieldWork {
        self.work.get()
    }

    fn count(&self, update: impl FnOnce(&mut FieldWork)) {
        let mut work = self.work.get();
        update(&mut work);
        self.work.set(work);
    }

    pub(super) fn value(&self, p: [f64; 3]) -> f64 {
        self.count(|w| w.values += 1);
        let mut value = [0.0];
        self.field.eval_points(&[p], &mut value);
        value[0]
    }

    /// The field at each of `points`, in one query.
    pub(super) fn values(&self, points: &[[f64; 3]], out: &mut [f64]) {
        self.count(|w| w.values += points.len() as u64);
        self.field.eval_points(points, out);
    }

    pub(super) fn gradient(&self, p: [f64; 3]) -> Dual {
        self.count(|w| w.gradients += 1);
        let mut row = [Dual::default()];
        self.field.eval_gradients(&[p], &mut row);
        row[0]
    }

    pub(super) fn crease(&self, p: [f64; 3]) -> Crease {
        self.count(|w| w.creases += 1);
        self.field.eval_crease(p)
    }

    pub(super) fn interval(&self, region: &Aabb) -> Interval {
        self.count(|w| w.intervals += 1);
        self.field.eval_interval(region)
    }

    pub(super) fn gradient_interval(&self, region: &Aabb) -> Dual<Interval> {
        self.count(|w| w.intervals += 1);
        self.field.eval_gradient_interval(region)
    }

    /// The crossing on the segment from `a`, inside with value `fa`, to `b`,
    /// outside with value `fb`, to within `tolerance` along it.
    ///
    /// False position with the Illinois rule, bisecting whenever a step
    /// fails to halve the bracket, so it converges at least as fast as
    /// bisection. Returns the evaluated point nearest the surface.
    pub(super) fn search(
        &self,
        a: [f64; 3],
        fa: f64,
        b: [f64; 3],
        fb: f64,
        tolerance: f64,
    ) -> [f64; 3] {
        let (mut lo, mut hi) = (0.0, 1.0);
        let (mut flo, mut fhi) = (fa, fb);
        let length = norm(sub(b, a));
        let mut side = 0;
        let mut width = 1.0_f64;
        let mut best = if fa.abs() <= fb.abs() {
            (0.0, fa)
        } else {
            (1.0, fb)
        };
        while (hi - lo) * length >= tolerance && hi - lo > f64::EPSILON {
            let secant = (lo * fhi - hi * flo) / (fhi - flo);
            let t = if hi - lo > 0.5 * width || !secant.is_finite() {
                0.5 * (lo + hi)
            } else {
                secant.clamp(lo, hi)
            };
            width = hi - lo;
            let f = self.value(lerp(a, b, t));
            if f.abs() < best.1.abs() {
                best = (t, f);
            }
            if f == 0.0 {
                break;
            }
            if f < 0.0 {
                lo = t;
                flo = f;
                if side == -1 {
                    fhi *= 0.5;
                }
                side = -1;
            } else {
                hi = t;
                fhi = f;
                if side == 1 {
                    flo *= 0.5;
                }
                side = 1;
            }
        }
        lerp(a, b, best.0)
    }

    /// The point on the crease nearest `start`, by minimum-norm Newton steps
    /// onto the field's and its nearest tie's zero sets, if it converges
    /// within `reach` of `start`.
    pub(super) fn crease_point(&self, start: [f64; 3], reach: f64) -> Option<[f64; 3]> {
        self.crease_point_along(start, reach, None)
    }

    /// The point nearest `start` on the crease whose tie has unit normal
    /// close to `normal`, or on the nearest crease without one.
    ///
    /// Each step takes whichever of the two nearest ties continues the
    /// crease, so a correction cannot slip onto a neighboring crease, as
    /// across a fin thinner than a step.
    pub(super) fn crease_point_along(
        &self,
        start: [f64; 3],
        reach: f64,
        normal: Option<[f64; 3]>,
    ) -> Option<[f64; 3]> {
        let mut x = start;
        for _ in 0..16 {
            let crease = self.crease(x);
            let tie = following(crease, normal)?;
            let rows = [crease.field, tie.function];
            let step = minimum_norm_step(rows)?;
            x = add(x, step);
            if dot(sub(x, start), sub(x, start)) > reach * reach {
                return None;
            }
            if norm(step) < 1.0e-6 * reach {
                return Some(x);
            }
        }
        None
    }

    /// The corner nearest `start`, where the field and its two nearest ties
    /// are all zero, by Newton steps, if it converges within `reach`.
    pub(super) fn corner_point(&self, start: [f64; 3], reach: f64) -> Option<[f64; 3]> {
        let mut x = start;
        for _ in 0..16 {
            let crease = self.crease(x);
            let [Some(first), Some(second)] = crease.ties else {
                return None;
            };
            let step = newton_step([crease.field, first.function, second.function])?;
            x = add(x, step);
            if dot(sub(x, start), sub(x, start)) > reach * reach {
                return None;
            }
            if norm(step) < 1.0e-6 * reach {
                return Some(x);
            }
        }
        None
    }

    /// Whether `region` provably holds no surface, its values all of the
    /// sign `inside`.
    ///
    /// Value bounds that exclude zero prove it at once. Otherwise, along
    /// each axis where the gradient's bounds keep one sign, the extreme that
    /// could reach zero lies on one face, so the region narrows to that
    /// face; narrowed to a point, that point's value decides.
    pub(super) fn proves_empty(&self, region: Aabb, inside: bool) -> bool {
        let mut region = region;
        loop {
            let bounds = self.gradient_interval(&region);
            if (inside && bounds.value.high() < 0.0) || (!inside && bounds.value.low() > 0.0) {
                return true;
            }
            let mut narrowed = false;
            for axis in 0..3 {
                if region.min[axis] == region.max[axis] {
                    continue;
                }
                let slope = bounds.gradient[axis];
                let rising = if slope.low() > 0.0 {
                    true
                } else if slope.high() < 0.0 {
                    false
                } else {
                    continue;
                };
                // Inside, the greatest value could reach zero; outside, the
                // least.
                if rising == inside {
                    region.min[axis] = region.max[axis];
                } else {
                    region.max[axis] = region.min[axis];
                }
                narrowed = true;
            }
            if !narrowed {
                return false;
            }
            if region.min == region.max {
                let value = self.value(region.min);
                return if inside { value < 0.0 } else { value > 0.0 };
            }
        }
    }

    /// Looks for the surface inside `region`, whose corners all have the
    /// sign `inside`, down to `depth` halvings, and returns a point of the
    /// other sign with its value when it finds one.
    ///
    /// A probe is a sample, not a proof: a feature that passes between its
    /// samples stays unfound.
    pub(super) fn probe(&self, region: Aabb, inside: bool, depth: u32) -> Option<([f64; 3], f64)> {
        if self.proves_empty(region, inside) {
            return None;
        }
        let middle = region.center();
        let value = self.value(middle);
        if (value < 0.0) != inside {
            return Some((middle, value));
        }
        if depth == 0 {
            return None;
        }
        (0..8).find_map(|octant| {
            let child = Aabb {
                min: core::array::from_fn(|k| {
                    if octant >> k & 1 == 0 {
                        region.min[k]
                    } else {
                        middle[k]
                    }
                }),
                max: core::array::from_fn(|k| {
                    if octant >> k & 1 == 0 {
                        middle[k]
                    } else {
                        region.max[k]
                    }
                }),
            };
            self.probe(child, inside, depth - 1)
        })
    }
}

/// The nearest tie, or with `normal`, the nearer of the two nearest whose
/// unit normal lies within about 25 degrees of it, either way round.
pub(super) fn following(crease: Crease, normal: Option<[f64; 3]>) -> Option<Tie> {
    let Some(normal) = normal else {
        return crease.ties[0];
    };
    crease.ties.into_iter().flatten().find(|tie| {
        let gradient = tie.function.gradient;
        dot(gradient, normal).abs() > 0.9 * norm(gradient)
    })
}

/// The least step moving both rows' linear models to zero.
fn minimum_norm_step(rows: [Dual; 2]) -> Option<[f64; 3]> {
    let [f, t] = rows;
    let (a, b, d) = (
        dot(f.gradient, f.gradient),
        dot(f.gradient, t.gradient),
        dot(t.gradient, t.gradient),
    );
    let determinant = a * d - b * b;
    // Nearly parallel gradients: the pieces meet tangentially, or not at
    // all, and the crease is not determined.
    if determinant.is_nan() || determinant <= 1.0e-6 * a * d {
        return None;
    }
    let l0 = (d * f.value - b * t.value) / determinant;
    let l1 = (a * t.value - b * f.value) / determinant;
    Some(core::array::from_fn(|k| {
        -(l0 * f.gradient[k] + l1 * t.gradient[k])
    }))
}

/// The step moving three rows' linear models to zero.
fn newton_step(rows: [Dual; 3]) -> Option<[f64; 3]> {
    let [r0, r1, r2] = rows.map(|row| row.gradient);
    let determinant = dot(r0, cross(r1, r2));
    let scale = norm(r0) * norm(r1) * norm(r2);
    if determinant.is_nan() || determinant.abs() <= 1.0e-6 * scale {
        return None;
    }
    // Cramer's rule on J dx = -r, through the adjugate's columns.
    let columns = [cross(r1, r2), cross(r2, r0), cross(r0, r1)];
    Some(core::array::from_fn(|k| {
        -(rows[0].value * columns[0][k]
            + rows[1].value * columns[1][k]
            + rows[2].value * columns[2][k])
            / determinant
    }))
}

pub(super) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(super) fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    core::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

pub(super) fn norm(v: [f64; 3]) -> f64 {
    exedra_math::norm(v)
}

pub(super) fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    norm(sub(a, b))
}

pub(super) fn midpoint(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    lerp(a, b, 0.5)
}

/// `v` scaled to unit length, if it has one.
pub(super) fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = norm(v);
    (length > 0.0 && length.is_finite()).then(|| v.map(|c| c / length))
}

/// The point with equal power distance to four weighted points.
pub(super) fn orthocenter(p: [[f64; 3]; 4], w: [f64; 4]) -> [f64; 3] {
    let a = p[0];
    let [b, c, d] = [p[1], p[2], p[3]].map(|q| sub(q, a));
    let (bc, cd, db) = (cross(b, c), cross(c, d), cross(d, b));
    let determinant = dot(b, cd);
    let (rb, rc, rd) = (
        dot(b, b) - (w[1] - w[0]),
        dot(c, c) - (w[2] - w[0]),
        dot(d, d) - (w[3] - w[0]),
    );
    let x: [f64; 3] =
        core::array::from_fn(|k| (rb * cd[k] + rc * db[k] + rd * bc[k]) / (2.0 * determinant));
    add(a, x)
}

/// The center of the circle through a triangle's corners.
pub(super) fn circumcenter(p: [[f64; 3]; 3]) -> [f64; 3] {
    let a = p[0];
    let (b, c) = (sub(p[1], a), sub(p[2], a));
    let n = cross(b, c);
    let nn = dot(n, n);
    let (t, u) = (cross(n, b), cross(c, n));
    let x: [f64; 3] = core::array::from_fn(|k| (dot(c, c) * t[k] + dot(b, b) * u[k]) / (2.0 * nn));
    add(a, x)
}

#[cfg(test)]
mod tests {
    use exedra_implicit::{Aabb, Cuboid, Difference, Sphere};

    use super::Sampler;

    #[test]
    fn searches_land_on_the_surface() {
        let sphere = Sphere::new([0.0; 3], 1.0).unwrap();
        let sampler = Sampler::new(&sphere);
        let x = sampler.search([0.0; 3], -1.0, [2.0, 0.0, 0.0], 1.0, 1e-9);
        assert!((x[0] - 1.0).abs() < 1e-9);
        assert!(sampler.work().values < 12);
    }

    #[test]
    fn creases_and_corners_are_found_exactly() {
        let cuboid = Cuboid::new([0.0; 3], [1.0; 3]).unwrap();
        let sampler = Sampler::new(&cuboid);
        // An exact box distance is smooth outside, yet its edges are found
        // from either side.
        for start in [[0.97, 0.95, 0.2], [1.03, 1.06, 0.2]] {
            let edge = sampler.crease_point(start, 0.5).unwrap();
            assert!((edge[0] - 1.0).abs() < 1e-12 && (edge[1] - 1.0).abs() < 1e-12);
        }
        let corner = sampler.corner_point([0.96, 0.97, 0.98], 0.5).unwrap();
        for c in corner {
            assert!((c - 1.0).abs() < 1e-12, "{corner:?}");
        }
        // Where a ball is cut from the box, its rim is a crease too.
        let bitten = Difference::new(cuboid, Sphere::new([1.0, 1.0, 1.0], 0.5).unwrap());
        let sampler = Sampler::new(&bitten);
        let rim = sampler.crease_point([0.55, 1.0, 0.9], 0.5).unwrap();
        let r = ((rim[0] - 1.0).powi(2) + (rim[1] - 1.0).powi(2) + (rim[2] - 1.0).powi(2)).sqrt();
        assert!(
            (r - 0.5).abs() < 1e-12 && (rim[1] - 1.0).abs() < 1e-12,
            "{rim:?}"
        );
    }

    #[test]
    fn empty_regions_are_proven_and_thin_features_probed() {
        let sphere = Sphere::new([0.0; 3], 1.0).unwrap();
        let sampler = Sampler::new(&sphere);
        let far = Aabb::new([1.5, 1.5, 1.5], [1.7, 1.7, 1.7]).unwrap();
        assert!(sampler.proves_empty(far, false));
        let thin = Cuboid::new([0.0; 3], [0.01, 1.0, 1.0]).unwrap();
        let sampler = Sampler::new(&thin);
        let cell = Aabb::new([-0.1, 0.0, 0.0], [0.07, 0.1, 0.1]).unwrap();
        let (p, value) = sampler.probe(cell, false, 3).unwrap();
        assert!(value < 0.0 && p[0].abs() < 0.01);
        // Monotone toward a corner no sample has seen: that corner decides.
        let ball = Sphere::new([0.51, 0.51, -0.01], 0.47).unwrap();
        let sampler = Sampler::new(&ball);
        let cell = Aabb::new([0.0; 3], [1.0; 3]).unwrap();
        for corner in cell.corners() {
            assert!(sampler.value(corner) > 0.0);
        }
        assert!(!sampler.proves_empty(Aabb::new([0.0; 3], [0.5; 3]).unwrap(), false));
        assert!(sampler.probe(cell, false, 2).is_some());
    }
}
