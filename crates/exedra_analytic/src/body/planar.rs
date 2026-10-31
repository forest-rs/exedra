// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Derived checked profile views of retained planar line/circle loops.
use super::{Body, FaceId, LoopId};
use crate::geometry::{Curve, CurveSpan, ParameterRange, Surface};
use alloc::vec::Vec;
use exedra_math::{Placement3, dot, sub};

impl<F, E> Body<F, E> {
    /// Derives a line/arc profile loop and a full-curve conversion gap. The native
    /// curves and shared vertices stay authoritative; an unsupported or unstable
    /// projection is refused rather than replacing native curves with a fit.
    pub(crate) fn planar_profile_loop(
        &self,
        face: FaceId,
        loop_id: LoopId,
    ) -> Option<(exedra_profile::Loop, f64)> {
        let surface = &self.face(face)?.surface;
        let Surface::Plane(plane) = surface else {
            return None;
        };
        let frame = plane.frame();
        let normal = frame.rows.map(|r| r[2]);
        let mut segments = Vec::new();
        let mut spans = Vec::new();
        for co in &self.boundary_loop(loop_id)?.coedges {
            let edge = self.edge(co.edge)?;
            let (curve, range) = edge.regular()?;
            if matches!(curve, Curve::Circle(_))
                && range.span() > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON)
            {
                return None;
            }
            let count =
                if matches!(curve, Curve::Circle(_)) && range.span() >= core::f64::consts::PI {
                    2
                } else {
                    1
                };
            for i in 0..count {
                let fraction0 = f64::from(i) / f64::from(count);
                let fraction1 = f64::from(i + 1) / f64::from(count);
                let at = |fraction: f64| {
                    if co.reversed {
                        1.0 - fraction
                    } else {
                        fraction
                    }
                };
                let from = surface.parameters(self.edge_point(co.edge, at(fraction0))?)?;
                let to = surface.parameters(self.edge_point(co.edge, at(fraction1))?)?;
                let p0 = range.at(at(fraction0))?;
                let p1 = range.at(at(fraction1))?;
                let source = CurveSpan::new(
                    curve.clone(),
                    ParameterRange::new(p0.min(p1), p0.max(p1)).ok()?,
                    co.reversed,
                )
                .ok()?;
                let segment = match curve {
                    Curve::Line(_) => exedra_profile::Segment::line(to),
                    Curve::Circle(c) => {
                        let clockwise = dot(c.frame().rows.map(|r| r[2]), normal)
                            * (if co.reversed { -1.0 } else { 1.0 })
                            < 0.0;
                        exedra_profile::Segment::arc_centered(
                            to,
                            surface.parameters(c.frame().rows.map(|r| r[3]))?,
                            clockwise,
                        )
                    }
                    Curve::Ellipse(_) | Curve::Nurbs(_) => return None,
                };
                // Resolving now establishes that both authoritative endpoints can
                // participate in this exact circular representation.
                exedra_profile::Arc::resolve(from, to, segment.curve).ok()?;
                segments.push(segment);
                spans.push(source);
            }
        }
        let loop_ = exedra_profile::Loop::new(segments).ok()?;
        let mut gap = 0.0_f64;
        for (index, ((from, segment), source)) in loop_.iter_with_starts().zip(&spans).enumerate() {
            let lifted = if let Some(arc) = loop_.arc(index) {
                let center = surface.evaluate(arc.center)?.point;
                let circle = Curve::circle(
                    Placement3::from_axes(
                        frame.rows.map(|r| r[0]),
                        frame.rows.map(|r| r[1]),
                        normal,
                        center,
                    ),
                    arc.radius,
                )
                .ok()?;
                let end = arc.start_angle + arc.sweep;
                CurveSpan::new(
                    circle,
                    ParameterRange::new(arc.start_angle.min(end), arc.start_angle.max(end)).ok()?,
                    arc.sweep < 0.0,
                )
                .ok()?
            } else {
                let a = surface.evaluate(from)?.point;
                let b = surface.evaluate(segment.to)?.point;
                CurveSpan::new(Curve::line(a, sub(b, a)).ok()?, ParameterRange::UNIT, false).ok()?
            };
            gap = gap.max(source.alignment(&lifted));
        }
        let magnitude = self
            .boundary_loop(loop_id)?
            .coedges
            .iter()
            .flat_map(|co| {
                let edge = &self.edges()[co.edge.index()];
                [
                    self.vertices()[edge.start.index()],
                    self.vertices()[edge.end.index()],
                ]
            })
            .flatten()
            .map(f64::abs)
            .fold(0.0, f64::max);
        gap += 512.0 * f64::EPSILON * magnitude;
        gap.is_finite().then_some((loop_, gap))
    }
}
