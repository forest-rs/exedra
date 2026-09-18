// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded polynomial root isolation using interval inclusion and subdivision.
//! Krawczyk inclusion: <https://ww2.ii.uj.edu.pl/~wilczak/papers/logmap/logmap.pdf>
//! Section 2.3, equation (10) and theorem 6. Arithmetic here is outward-rounded;
//! this isolates intersections of the fitted curves, not errors in the fitter.

use super::super::{OffsetBudget, OffsetContext};
use super::interval::I;
use crate::profile::ProfileError;
use alloc::vec::Vec;
use kurbo::{CubicBez, Point};

#[derive(Clone, Copy)]
pub(super) struct Curve {
    points: [Point; 4],
    count: usize,
}
impl Curve {
    pub(super) fn line(a: Point, b: Point) -> Self {
        Self {
            points: [a, b, Point::ZERO, Point::ZERO],
            count: 2,
        }
    }
    pub(super) fn cubic(c: CubicBez) -> Self {
        Self {
            points: [c.p0, c.p1, c.p2, c.p3],
            count: 4,
        }
    }
    fn control(self) -> [[I; 2]; 4] {
        self.points.map(|p| [I::p(p.x), I::p(p.y)])
    }
    fn eval(self, t: I) -> [I; 2] {
        bezier(&self.control()[..self.count], t)
    }
    fn deriv(self, t: I) -> [I; 2] {
        let p = self.control();
        let mut d = [[I::ZERO; 2]; 3];
        for i in 0..self.count - 1 {
            for k in 0..2 {
                d[i][k] = I::p((self.count - 1) as f64) * (p[i + 1][k] - p[i][k]);
            }
        }
        bezier(&d[..self.count - 1], t)
    }
    pub(super) fn point(self, t: f64) -> Point {
        let p = self.eval(I::p(t));
        Point::new(p[0].mid(), p[1].mid())
    }
}
fn bezier(control: &[[I; 2]], t: I) -> [I; 2] {
    let mut p = [[I::ZERO; 2]; 4];
    p[..control.len()].copy_from_slice(control);
    for n in (1..control.len()).rev() {
        for i in 0..n {
            p[i] = core::array::from_fn(|k| (I::ONE - t) * p[i][k] + t * p[i + 1][k]);
        }
    }
    p[0]
}
#[derive(Clone, Copy)]
pub(super) enum System {
    Curves(Curve, Curve),
    Circle(Curve, Point, f64),
}
impl System {
    fn eval(self, x: [I; 2]) -> ([I; 2], [[I; 2]; 2]) {
        match self {
            Self::Curves(a, b) => {
                let (p, q, dp, dq) = (a.eval(x[0]), b.eval(x[1]), a.deriv(x[0]), b.deriv(x[1]));
                (
                    [p[0] - q[0], p[1] - q[1]],
                    [[dp[0], -dq[0]], [dp[1], -dq[1]]],
                )
            }
            Self::Circle(c, center, radius) => {
                let p = c.eval(x[0]);
                let d = c.deriv(x[0]);
                let p = [p[0] - I::p(center.x), p[1] - I::p(center.y)];
                (
                    [
                        p[0].square() + p[1].square() - I::p(radius).square(),
                        x[1] - I::p(0.5),
                    ],
                    [
                        [I::p(2.0) * (p[0] * d[0] + p[1] * d[1]), I::ZERO],
                        [I::ZERO, I::ONE],
                    ],
                )
            }
        }
    }
    fn positions(self, x: [I; 2]) -> [I; 2] {
        match self {
            Self::Curves(a, b) => {
                let (p, q) = (a.eval(x[0]), b.eval(x[1]));
                [p[0].union(q[0]), p[1].union(q[1])]
            }
            Self::Circle(c, _, _) => c.eval(x[0]),
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct Root {
    pub parameters: [f64; 2],
    pub point: Point,
    pub bound: f64,
    domain: [I; 2],
    enclosure: [I; 2],
}
impl Root {
    pub(super) fn parameter_bounds(self) -> [[f64; 2]; 2] {
        self.enclosure.map(|v| [v.lo, v.hi])
    }
}
struct Node {
    x: [I; 2],
    depth: u32,
}
fn includes(a: [I; 2], b: [I; 2]) -> bool {
    (0..2).all(|i| a[i].contains(b[i]))
}

/// Returns every isolated interior root, or refuses an unresolved candidate.
pub(super) fn roots(
    system: System,
    tolerance: f64,
    context: &mut OffsetContext,
    hole: Option<usize>,
    seg: usize,
) -> Result<Vec<Root>, ProfileError> {
    let unresolved = || ProfileError::OffsetTrimUnresolved { hole, seg };
    let mut stack = alloc::vec![Node {
        x: [I::UNIT; 2],
        depth: 0
    }];
    let mut roots: Vec<Root> = Vec::new();
    'nodes: while let Some(Node { mut x, depth }) = stack.pop() {
        let mut domain = None;
        let mut expanded = false;
        for _ in 0..64 {
            context.charge(OffsetBudget::TrimSteps, 1)?;
            if roots.iter().any(|r| includes(r.domain, x)) {
                continue 'nodes;
            }
            let (f, j) = system.eval(x);
            if f.iter().chain(j.iter().flatten()).any(|v| !v.valid()) {
                return Err(unresolved());
            }
            if f.iter().any(|v| v.excludes_zero()) {
                continue 'nodes;
            }
            let mid = x.map(I::mid);
            let (f0, j0) = system.eval(mid.map(I::p));
            let a = j0.map(|r| r.map(I::mid));
            let determinant = a[0][0] * a[1][1] - a[0][1] * a[1][0];
            if determinant.is_finite() && determinant != 0.0 {
                let c = [
                    [a[1][1] / determinant, -a[0][1] / determinant],
                    [-a[1][0] / determinant, a[0][0] / determinant],
                ];
                let det_c = I::p(c[0][0]) * I::p(c[1][1]) - I::p(c[0][1]) * I::p(c[1][0]);
                if c.iter().flatten().all(|v| v.is_finite())
                    && det_c.valid()
                    && det_c.excludes_zero()
                {
                    let mut k = [I::ZERO; 2];
                    for i in 0..2 {
                        k[i] = I::p(mid[i]) - I::p(c[i][0]) * f0[0] - I::p(c[i][1]) * f0[1];
                        for n in 0..2 {
                            let identity = if i == n { I::ONE } else { I::ZERO };
                            let factor =
                                identity - I::p(c[i][0]) * j[0][n] - I::p(c[i][1]) * j[1][n];
                            k[i] = k[i] + factor * (x[n] - I::p(mid[n]));
                        }
                    }
                    if k.iter().any(|v| !v.valid()) {
                        return Err(unresolved());
                    }
                    let (Some(first), Some(second)) = (x[0].intersect(k[0]), x[1].intersect(k[1]))
                    else {
                        continue 'nodes;
                    };
                    if domain.is_none() && (0..2).all(|i| x[i].interior(k[i])) {
                        domain = Some(x);
                    }
                    let next = [first, second];
                    if let Some(domain) = domain {
                        let position = system.positions(next);
                        if position.iter().any(|v| !v.valid()) {
                            return Err(unresolved());
                        }
                        let point = Point::new(position[0].mid(), position[1].mid());
                        // Full box L1 diameter covers midpoint and evaluation rounding.
                        let bound = (position[0].width().next_up() + position[1].width().next_up())
                            .next_up();
                        if bound <= tolerance {
                            let root = Root {
                                parameters: next.map(I::mid),
                                point,
                                bound,
                                domain,
                                enclosure: next,
                            };
                            if !roots.iter().any(|old| {
                                includes(old.domain, root.enclosure)
                                    || includes(root.domain, old.enclosure)
                            }) {
                                roots.push(root);
                            }
                            continue 'nodes;
                        }
                    }
                    if next[0].width() + next[1].width() < 0.8 * (x[0].width() + x[1].width()) {
                        x = next;
                        continue;
                    }
                    if domain.is_some() {
                        return Err(unresolved());
                    }
                }
            }
            // Near a rounded root, a contracted box may have the root on its
            // boundary. Prove inclusion in a slightly larger finite-domain box;
            // this also covers every unresolved point of the original box.
            if !expanded && x.iter().all(|v| v.width() < 1e-6) {
                x = x.map(|v| {
                    let pad = (v.width() * 2.0).max(128.0 * f64::EPSILON);
                    I {
                        lo: (v.lo - pad).max(0.0),
                        hi: (v.hi + pad).min(1.0),
                    }
                });
                expanded = true;
                continue;
            }
            if depth >= 64 {
                return Err(unresolved());
            }
            let axis = usize::from(x[1].width() > x[0].width());
            let middle = x[axis].mid();
            let overlap = x[axis].width() * 0.0625;
            let mut left = x;
            let mut right = x;
            left[axis].hi = middle + overlap;
            right[axis].lo = middle - overlap;
            if left[axis].hi >= x[axis].hi || right[axis].lo <= x[axis].lo {
                return Err(unresolved());
            }
            stack.push(Node {
                x: right,
                depth: depth + 1,
            });
            stack.push(Node {
                x: left,
                depth: depth + 1,
            });
            continue 'nodes;
        }
        return Err(unresolved());
    }
    Ok(roots)
}
