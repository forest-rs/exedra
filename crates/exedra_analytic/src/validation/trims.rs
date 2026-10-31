// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    BoundaryValidationError as Error, BoundaryValidationPolicy, TrimProof, TrimValidation,
    UnsupportedTrim,
};
use crate::body::{Body, Coedge, FaceId, LoopId, LoopTrims, TrimCurve};
use crate::geometry::{Curve, Surface};
use alloc::{borrow::Cow, vec, vec::Vec};
use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};

#[derive(Clone)]
enum Shape {
    Polygon(Vec<[f64; 2]>),
    Ellipse {
        center: [f64; 2],
        cosine: [f64; 2],
        sine: [f64; 2],
        reversed: bool,
    },
}
impl Shape {
    fn boundary_point(&self) -> [f64; 2] {
        match self {
            Self::Polygon(p) => p[0],
            Self::Ellipse { center, cosine, .. } => [center[0] + cosine[0], center[1] + cosine[1]],
        }
    }
    fn area(&self) -> f64 {
        match self {
            Self::Polygon(points) => {
                points
                    .iter()
                    .zip(points.iter().cycle().skip(1))
                    .take(points.len())
                    .map(|(a, b)| cross(sub(*a, points[0]), sub(*b, points[0])))
                    .sum::<f64>()
                    * 0.5
            }
            Self::Ellipse {
                cosine,
                sine,
                reversed,
                ..
            } => core::f64::consts::PI * cross(*cosine, *sine) * if *reversed { -1.0 } else { 1.0 },
        }
    }
    fn contains(&self, p: [f64; 2]) -> Option<bool> {
        match self {
            Self::Ellipse {
                center,
                cosine,
                sine,
                ..
            } => {
                let q = inverse(*cosine, *sine, sub(p, *center))?;
                Some(norm(q) < 1.0)
            }
            Self::Polygon(points) => {
                let query = polygon_loop(points, false).ok()?.query_point(p).ok()?;
                query.winding.map(|winding| winding != 0)
            }
        }
    }
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn norm(a: [f64; 2]) -> f64 {
    libm::hypot(a[0], a[1])
}
fn minimum_scale(a: [f64; 2], b: [f64; 2]) -> f64 {
    // det/sigma_max is the smaller singular value; the Frobenius norm
    // upper-bounds sigma_max even for accepted slightly nonorthogonal frames.
    cross(a, b).abs() / libm::hypot(norm(a), norm(b))
}
fn inverse(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> Option<[f64; 2]> {
    let determinant = cross(a, b);
    let scale = norm(a) * norm(b);
    if !determinant.is_finite() || determinant.abs() <= 256.0 * f64::EPSILON * scale {
        return None;
    }
    let q = [cross(p, b) / determinant, cross(a, p) / determinant];
    q.into_iter().all(f64::is_finite).then_some(q)
}
fn polygon_loop(
    points: &[[f64; 2]],
    reverse: bool,
) -> Result<Loop, exedra_curves::profile::LoopError> {
    let mut segments = points
        .iter()
        .copied()
        .map(Segment::line)
        .collect::<Vec<_>>();
    if reverse {
        segments.reverse();
    }
    Loop::new(segments)
}
fn profile_policy(policy: &BoundaryValidationPolicy) -> ProfilePolicy {
    ProfilePolicy {
        min_separation: policy.chart_separation,
        max_segments: u32::MAX,
    }
}
fn geometry_chart<F, E>(
    body: &Body<F, E>,
    face: FaceId,
    loop_id: LoopId,
) -> Option<Vec<TrimCurve>> {
    let surface = &body.faces()[face.index()].surface;
    let Surface::Plane(_) = surface else {
        return None;
    };
    body.loops()[loop_id.index()]
        .coedges
        .iter()
        .map(|co| {
            let edge = &body.edges()[co.edge.index()];
            let (curve, range) = edge.regular()?;
            match curve {
                Curve::Line(_) => Some(TrimCurve::Line {
                    start: surface.parameters(curve.evaluate(range.bounds()[0])?.point)?,
                    end: surface.parameters(curve.evaluate(range.bounds()[1])?.point)?,
                }),
                Curve::Circle(c) => {
                    let frame = c.frame();
                    let radius = c.radius();
                    harmonic_chart(
                        surface,
                        frame.rows.map(|r| r[3]),
                        frame.rows.map(|r| r[0] * radius),
                        frame.rows.map(|r| r[1] * radius),
                        range,
                    )
                }
                Curve::Ellipse(e) => {
                    let frame = e.frame();
                    harmonic_chart(
                        surface,
                        frame.rows.map(|r| r[3]),
                        frame.rows.map(|r| r[0] * e.semi_major()),
                        frame.rows.map(|r| r[1] * e.semi_minor()),
                        range,
                    )
                }
                Curve::Nurbs(n) => crate::body::planar_nurbs(surface, n, range).ok(),
            }
        })
        .collect()
}
fn harmonic_chart(
    surface: &Surface,
    center: [f64; 3],
    a: [f64; 3],
    b: [f64; 3],
    range: crate::geometry::ParameterRange,
) -> Option<TrimCurve> {
    let origin = surface.parameters(center)?;
    let vector = |v: [f64; 3]| {
        let frame = surface.frame();
        let zero = exedra_math::Placement3::from_axes(
            frame.rows.map(|r| r[0]),
            frame.rows.map(|r| r[1]),
            frame.rows.map(|r| r[2]),
            [0.0; 3],
        );
        let p = crate::geometry::frame_coordinates(zero, v)?;
        Some([p[0], p[1]])
    };
    Some(TrimCurve::Harmonic {
        origin,
        linear: [0.0; 2],
        cosine: vector(a)?,
        sine: vector(b)?,
        range,
    })
}
fn shape(curves: &[TrimCurve], coedges: &[Coedge], planar: bool) -> Option<Shape> {
    if curves
        .iter()
        .all(|curve| matches!(curve, TrimCurve::Line { .. }))
    {
        if curves.len() < 3 {
            return None;
        }
        return Some(Shape::Polygon(
            curves
                .iter()
                .zip(coedges)
                .map(|(curve, coedge)| {
                    curve
                        .evaluate(if coedge.reversed { 1.0 } else { 0.0 })
                        .expect("validated curve")
                })
                .collect(),
        ));
    }
    if !planar || curves.len() != 1 {
        return None;
    }
    let TrimCurve::Harmonic {
        origin,
        linear,
        cosine,
        sine,
        range,
    } = curves[0]
    else {
        return None;
    };
    if linear != [0.0; 2] || range.span() != core::f64::consts::TAU {
        return None;
    }
    inverse(cosine, sine, [1.0, 0.0])?;
    Some(Shape::Ellipse {
        center: origin,
        cosine,
        sine,
        reversed: coedges[0].reversed,
    })
}

pub(super) fn check<F, E>(
    body: &Body<F, E>,
    face: FaceId,
    policy: &BoundaryValidationPolicy,
    trim_index: &[Option<&LoopTrims>],
) -> Result<TrimValidation, Error> {
    let f = &body.faces()[face.index()];
    let repeated = match f.surface {
        Surface::Cylinder(_) | Surface::Cone(_) | Surface::Sphere(_) => {
            f.domain.u.span() > core::f64::consts::TAU
        }
        Surface::Torus(_) => {
            f.domain.u.span() > core::f64::consts::TAU || f.domain.v.span() > core::f64::consts::TAU
        }
        _ => false,
    };
    if repeated {
        return Ok(TrimValidation::Unsupported(UnsupportedTrim::CurveFamily));
    }
    let mut shapes = Vec::with_capacity(f.loops.len());
    let planar = matches!(f.surface, Surface::Plane(_));
    let mut charts: Vec<Cow<'_, [TrimCurve]>> = Vec::with_capacity(f.loops.len());
    for &loop_id in &f.loops {
        let curves = if let Some(record) = trim_index[loop_id.index()] {
            Cow::Borrowed(record.curves.as_slice())
        } else if let Some(curves) = geometry_chart(body, face, loop_id) {
            // Model-space endpoint slack cannot establish chart topology. In
            // particular, fitting a stored vertex polygon can hide an actual
            // line crossing immediately beside a shared corner. Infer from
            // complete native curves and require the same arithmetic-scale
            // closure as explicit chart records, independently of max_gap.
            let ring = &body.loops()[loop_id.index()];
            for (i, curve) in curves.iter().enumerate() {
                let next = (i + 1) % curves.len();
                let Some(end) = curve.evaluate(if ring.coedges[i].reversed { 0.0 } else { 1.0 })
                else {
                    return Ok(TrimValidation::Unsupported(UnsupportedTrim::NumericLimit));
                };
                let Some(start) = curves[next].evaluate(if ring.coedges[next].reversed {
                    1.0
                } else {
                    0.0
                }) else {
                    return Ok(TrimValidation::Unsupported(UnsupportedTrim::NumericLimit));
                };
                if (0..2).any(|axis| {
                    (end[axis] - start[axis]).abs()
                        > 256.0 * f64::EPSILON * end[axis].abs().max(start[axis].abs()).max(1.0)
                }) {
                    return Ok(TrimValidation::Unsupported(UnsupportedTrim::NumericLimit));
                }
            }
            for curve in &curves {
                let Some(bounds) = curve.bounds() else {
                    return Ok(TrimValidation::Unsupported(UnsupportedTrim::NumericLimit));
                };
                for (bound, domain) in bounds.into_iter().zip([f.domain.u, f.domain.v]) {
                    let [lo, hi] = domain.bounds();
                    let roundoff = 256.0 * f64::EPSILON * lo.abs().max(hi.abs()).max(1.0);
                    if bound[0] < lo - roundoff || bound[1] > hi + roundoff {
                        return Err(Error::ChartDomain { face, loop_id });
                    }
                }
            }
            Cow::Owned(curves)
        } else {
            return Ok(TrimValidation::Unsupported(UnsupportedTrim::MissingChart));
        };
        charts.push(curves);
    }
    let coefficient_scale = charts
        .iter()
        .flat_map(|curves| curves.iter())
        .map(|curve| match *curve {
            TrimCurve::Line { start, end } => start
                .into_iter()
                .chain(end)
                .map(f64::abs)
                .fold(1.0, f64::max),
            TrimCurve::Harmonic {
                origin,
                linear,
                cosine,
                sine,
                range,
            } => {
                let t = range.bounds().into_iter().map(f64::abs).fold(1.0, f64::max);
                (0..2)
                    .map(|i| {
                        origin[i].abs() + linear[i].abs() * t + cosine[i].abs() + sine[i].abs()
                    })
                    .fold(t, f64::max)
            }
            TrimCurve::Nurbs { ref curve, .. } => curve
                .points()
                .iter()
                .flatten()
                .map(|v| v.abs())
                .fold(1.0, f64::max),
        })
        .fold(1.0, f64::max);
    if !coefficient_scale.is_finite()
        || 4096.0 * f64::EPSILON * coefficient_scale > 0.25 * policy.chart_separation
    {
        return Ok(TrimValidation::Unsupported(UnsupportedTrim::NumericLimit));
    }
    if matches!(f.surface, Surface::Cylinder(_))
        && f.loops.len() == 1
        && let Some(status) = cylinder_graphs(body, face, &charts[0], policy)?
    {
        return Ok(status);
    }
    for (i, curves) in charts.iter().enumerate() {
        let ring = &body.loops()[f.loops[i].index()];
        let Some(s) = shape(curves, &ring.coedges, planar) else {
            return Ok(TrimValidation::Unsupported(UnsupportedTrim::CurveFamily));
        };
        let expected = f.sense.sign() * if i == 0 { 1.0 } else { -1.0 };
        if !s.area().is_finite() {
            return Ok(TrimValidation::Unsupported(UnsupportedTrim::NumericLimit));
        }
        if s.area() * expected <= 0.0 {
            return Err(Error::Winding {
                face,
                loop_id: f.loops[i],
            });
        }
        shapes.push(s);
    }
    if shapes.iter().all(|s| matches!(s, Shape::Polygon(_))) {
        let loops = shapes
            .iter()
            .map(|s| {
                let Shape::Polygon(p) = s else { unreachable!() };
                polygon_loop(p, f.sense.sign() < 0.0)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| Error::ChartLoop { face, error })?;
        let mut loops = loops.into_iter();
        Profile::new(
            loops.next().expect("nonempty face"),
            loops.collect(),
            &profile_policy(policy),
        )
        .map_err(|error| Error::PolygonArrangement { face, error })?;
        return Ok(TrimValidation::Certified(TrimProof::Polygon));
    }
    // Every polygon must be simple even when the other loops are elliptic.
    for s in &shapes {
        if let Shape::Polygon(p) = s {
            let ring = polygon_loop(p, s.area() < 0.0)
                .map_err(|error| Error::ChartLoop { face, error })?;
            Profile::new(ring, vec![], &profile_policy(policy))
                .map_err(|error| Error::PolygonArrangement { face, error })?;
        }
    }
    let mut unresolved = false;
    for i in 0..shapes.len() {
        for j in i + 1..shapes.len() {
            if !separated(&shapes[i], &shapes[j], policy.chart_separation) {
                unresolved = true;
            }
        }
    }
    // Classification is used only after complete boundary separation, never to
    // turn a handful of boundary evaluations into a continuous certificate.
    if unresolved {
        return Ok(TrimValidation::Unsupported(
            UnsupportedTrim::SeparationUnresolved,
        ));
    }
    for (i, hole) in shapes.iter().enumerate().skip(1) {
        if shapes[0].contains(hole.boundary_point()) != Some(true) {
            return Err(Error::HoleContainment {
                face,
                loop_id: f.loops[i],
            });
        }
        for (j, other) in shapes.iter().enumerate().skip(1) {
            if i != j && other.contains(hole.boundary_point()) != Some(false) {
                return Err(Error::HoleContainment {
                    face,
                    loop_id: f.loops[i],
                });
            }
        }
    }
    Ok(TrimValidation::Certified(TrimProof::PlanarEllipses))
}

fn separated(a: &Shape, b: &Shape, separation: f64) -> bool {
    match (a, b) {
        (Shape::Polygon(p), Shape::Polygon(q)) => {
            // Both orientations are normalized for this independent two-loop test.
            // An artificial surrounding rectangle would obscure containment, so
            // use complete segment distance directly.
            p.iter()
                .zip(p.iter().cycle().skip(1))
                .take(p.len())
                .all(|(a, b)| {
                    q.iter()
                        .zip(q.iter().cycle().skip(1))
                        .take(q.len())
                        .all(|(c, d)| segment_distance(*a, *b, *c, *d) > separation)
                })
        }
        (
            Shape::Polygon(p),
            ellipse @ Shape::Ellipse {
                center,
                cosine,
                sine,
                ..
            },
        )
        | (
            ellipse @ Shape::Ellipse {
                center,
                cosine,
                sine,
                ..
            },
            Shape::Polygon(p),
        ) => {
            let radius = minimum_scale(*cosine, *sine);
            let padding = separation / radius + 2048.0 * f64::EPSILON;
            if !padding.is_finite() {
                return false;
            }
            p.iter()
                .zip(p.iter().cycle().skip(1))
                .take(p.len())
                .all(|(a, b)| {
                    let (Some(a), Some(b)) = (
                        inverse(*cosine, *sine, sub(*a, *center)),
                        inverse(*cosine, *sine, sub(*b, *center)),
                    ) else {
                        return false;
                    };
                    (norm(a).max(norm(b)) < 1.0 - padding)
                        || (point_segment([0.0; 2], a, b) > 1.0 + padding)
                })
                && ellipse.area().is_finite()
        }
        (
            Shape::Ellipse {
                center: ca,
                cosine: ac,
                sine: as_,
                ..
            },
            Shape::Ellipse {
                center: cb,
                cosine: bc,
                sine: bs,
                ..
            },
        ) => {
            let delta = sub(*cb, *ca);
            let axes = [delta, [1.0, 0.0], [0.0, 1.0], *ac, *as_, *bc, *bs];
            if axes.into_iter().any(|axis| {
                let length = norm(axis);
                if length == 0.0 {
                    return false;
                }
                let distance = dot(delta, axis).abs();
                let radii = libm::hypot(dot(*ac, axis), dot(*as_, axis))
                    + libm::hypot(dot(*bc, axis), dot(*bs, axis));
                let error = 2048.0 * f64::EPSILON * (distance + radii + length * separation);
                distance - radii > length * separation + error
            }) {
                return true;
            }
            ellipse_inside(a, b, separation) || ellipse_inside(b, a, separation)
        }
    }
}
fn ellipse_inside(inner: &Shape, outer: &Shape, separation: f64) -> bool {
    let (
        Shape::Ellipse {
            center: ic,
            cosine: ia,
            sine: ib,
            ..
        },
        Shape::Ellipse {
            center: oc,
            cosine: oa,
            sine: ob,
            ..
        },
    ) = (inner, outer)
    else {
        return false;
    };
    let (Some(center), Some(a), Some(b)) = (
        inverse(*oa, *ob, sub(*ic, *oc)),
        inverse(*oa, *ob, *ia),
        inverse(*oa, *ob, *ib),
    ) else {
        return false;
    };
    let (aa, ab, bb) = (dot(a, a), dot(a, b), dot(b, b));
    let spectral = libm::sqrt(0.5 * (aa + bb + libm::hypot(aa - bb, 2.0 * ab)));
    let bound = norm(center) + spectral;
    let padding = separation / minimum_scale(*oa, *ob) + 4096.0 * f64::EPSILON * (1.0 + bound);
    bound.is_finite() && bound < 1.0 - padding
}
fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = sub(b, a);
    let length2 = dot(d, d);
    let t = if length2 == 0.0 {
        0.0
    } else {
        (dot(sub(p, a), d) / length2).clamp(0.0, 1.0)
    };
    norm(sub(p, [a[0] + t * d[0], a[1] + t * d[1]]))
}
fn segment_distance(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    let (ab, cd) = (sub(b, a), sub(d, c));
    let (x, y) = (cross(ab, sub(c, a)), cross(ab, sub(d, a)));
    let (u, v) = (cross(cd, sub(a, c)), cross(cd, sub(b, c)));
    let bounds_overlap =
        (0..2).all(|i| a[i].min(b[i]) <= c[i].max(d[i]) && c[i].min(d[i]) <= a[i].max(b[i]));
    if bounds_overlap && x * y <= 0.0 && u * v <= 0.0 {
        return 0.0;
    }
    point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b))
}

// v(u)=constant+cosine*cos(u)+sine*sin(u); input chart correspondence
// supplies the angular phase exactly, including a reversed canonical rim.
#[derive(Copy, Clone)]
struct Graph {
    start: f64,
    end: f64,
    constant: f64,
    cosine: f64,
    sine: f64,
    direction: f64,
}
fn graph(curve: &TrimCurve, reversed: bool) -> Option<Graph> {
    if let TrimCurve::Line { start, end } = *curve {
        if start[1] != end[1] || start[0] == end[0] {
            return None;
        }
        let (start, end) = if reversed { (end, start) } else { (start, end) };
        return Some(Graph {
            start: start[0],
            end: end[0],
            constant: start[1],
            cosine: 0.0,
            sine: 0.0,
            direction: (end[0] - start[0]).signum(),
        });
    }
    let TrimCurve::Harmonic {
        origin,
        linear,
        cosine,
        sine,
        range,
    } = *curve
    else {
        return None;
    };
    if cosine[0] != 0.0 || sine[0] != 0.0 || linear[1] != 0.0 || linear[0].abs() != 1.0 {
        return None;
    }
    let phase = origin[0];
    let (s, c) = (libm::sin(phase), libm::cos(phase));
    let a = cosine[1] * c - linear[0] * sine[1] * s;
    let b = cosine[1] * s + linear[0] * sine[1] * c;
    let [lo, hi] = range.bounds();
    let start = origin[0] + linear[0] * if reversed { hi } else { lo };
    let end = origin[0] + linear[0] * if reversed { lo } else { hi };
    Some(Graph {
        start,
        end,
        constant: origin[1],
        cosine: a,
        sine: b,
        direction: (end - start).signum(),
    })
}
fn cylinder_graphs<F, E>(
    body: &Body<F, E>,
    face: FaceId,
    curves: &[TrimCurve],
    policy: &BoundaryValidationPolicy,
) -> Result<Option<TrimValidation>, Error> {
    let f = &body.faces()[face.index()];
    let ring = &body.loops()[f.loops[0].index()];
    if curves.len() != 4 {
        return Ok(None);
    }
    let mut rims = Vec::with_capacity(2);
    let mut seams = 0;
    for (curve, co) in curves.iter().zip(&ring.coedges) {
        if let Some(g) = graph(curve, co.reversed) {
            rims.push(g);
        } else if let TrimCurve::Line { start, end } = *curve {
            if start[0] != end[0] {
                return Ok(None);
            }
            seams += 1;
        } else {
            return Ok(None);
        }
    }
    if rims.len() != 2 || seams != 2 {
        return Ok(None);
    }
    if rims[0].direction == rims[1].direction {
        return Err(Error::Winding {
            face,
            loop_id: f.loops[0],
        });
    }
    let extent = |g: Graph| [g.start.min(g.end), g.start.max(g.end)];
    let (a, b) = (extent(rims[0]), extent(rims[1]));
    let magnitude = curves
        .iter()
        .filter_map(TrimCurve::bounds)
        .flatten()
        .flatten()
        .map(f64::abs)
        .fold(1.0, f64::max);
    let error = 4096.0 * f64::EPSILON * magnitude;
    if (a[0] - b[0]).abs() > error
        || (a[1] - b[1]).abs() > error
        || (a[1] - a[0] - core::f64::consts::TAU).abs() > error
    {
        return Ok(None);
    }
    // A positive-sense chart follows the lower graph toward increasing U.
    let (lower, upper) = if rims[0].direction * f.sense.sign() > 0.0 {
        (rims[0], rims[1])
    } else {
        (rims[1], rims[0])
    };
    let minimum = upper.constant
        - lower.constant
        - libm::hypot(upper.cosine - lower.cosine, upper.sine - lower.sine)
        - error;
    if minimum <= policy.chart_separation {
        if minimum + 2.0 * error < 0.0 {
            return Err(Error::CylinderWidth { face, minimum });
        }
        return Ok(Some(TrimValidation::Unsupported(
            UnsupportedTrim::SeparationUnresolved,
        )));
    }
    Ok(Some(TrimValidation::Certified(
        TrimProof::CylindricalGraphs,
    )))
}
