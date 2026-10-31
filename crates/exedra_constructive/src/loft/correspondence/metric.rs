// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Source-curve distances and exact subdivision.

use super::{BoundaryPoint, CorrespondenceError, LandmarkError, ProfileSegmentSpan};
use crate::profile::{Loop2, Seg2, SegKind};
use alloc::vec::Vec;
use exedra_curves::profile::{Arc, Loop, LoopRef, SegmentRef};

#[derive(Copy, Clone)]
pub(super) struct Anchor {
    pub(super) point: BoundaryPoint,
    pub(super) distance: f64,
}
pub(super) struct Metric<'a> {
    ring: &'a Loop,
    pub(super) arcs: Vec<Option<Arc>>,
    prefix: Vec<f64>,
    pub(super) anchors: Vec<Anchor>,
    first: f64,
    total: f64,
    pub(super) boundary: usize,
    pub(super) remaining: usize,
    pub(super) section: usize,
    loop_ref: LoopRef,
}
impl<'a> Metric<'a> {
    pub(super) fn new(
        ring: &'a Loop,
        landmarks: Option<&[BoundaryPoint]>,
        section: usize,
        loop_ref: LoopRef,
    ) -> Result<Self, CorrespondenceError> {
        let numeric = || CorrespondenceError::Numeric {
            section,
            loop_ref,
            points: None,
        };
        let n = ring.segments().len();
        let mut prefix = Vec::with_capacity(n + 1);
        let mut arcs = Vec::with_capacity(n);
        prefix.push(0.0);
        let mut total = 0.0;
        for (i, (from, s)) in ring.iter_with_starts().enumerate() {
            let arc = ring.arc(i);
            let length = arc.map_or_else(
                || libm::hypot(s.to[0] - from[0], s.to[1] - from[1]),
                |a| a.length(),
            );
            let next = total + length;
            if !next.is_finite() || next <= total {
                return Err(numeric());
            }
            total = next;
            prefix.push(total);
            arcs.push(arc);
        }
        let default = [BoundaryPoint::new(0, 0.0)];
        let landmarks = landmarks.unwrap_or(&default);
        let error = |point, reason| CorrespondenceError::Landmarks {
            section,
            loop_ref,
            point,
            reason,
        };
        if landmarks.is_empty() {
            return Err(error(None, LandmarkError::Point));
        }
        let canonical = |mut p: BoundaryPoint| {
            if p.segment >= n || !(0.0..=1.0).contains(&p.parameter) {
                return None;
            }
            if p.parameter == 1.0 {
                p = BoundaryPoint::new((p.segment + 1) % n, 0.0);
            } else if p.parameter == 0.0 {
                p.parameter = 0.0;
            }
            Some(p)
        };
        let first_point =
            canonical(landmarks[0]).ok_or_else(|| error(Some(0), LandmarkError::Point))?;
        let absolute = |p: BoundaryPoint| {
            prefix[p.segment] + p.parameter * (prefix[p.segment + 1] - prefix[p.segment])
        };
        let first = absolute(first_point);
        let mut anchors = Vec::with_capacity(landmarks.len());
        let mut previous = None;
        for (i, &point) in landmarks.iter().enumerate() {
            let point = canonical(point).ok_or_else(|| error(Some(i), LandmarkError::Point))?;
            let d = absolute(point);
            let distance = if d >= first {
                d - first
            } else {
                total - first + d
            };
            if !distance.is_finite() || distance >= total || previous.is_some_and(|p| distance <= p)
            {
                return Err(error(Some(i), LandmarkError::Order));
            }
            anchors.push(Anchor { point, distance });
            previous = Some(distance);
        }
        let boundary = if first_point.parameter == 0.0 {
            first_point.segment
        } else {
            (first_point.segment + 1) % n
        };
        Ok(Self {
            ring,
            arcs,
            prefix,
            anchors,
            first,
            total,
            boundary,
            remaining: n,
            section,
            loop_ref,
        })
    }
    pub(super) fn numeric(&self) -> CorrespondenceError {
        CorrespondenceError::Numeric {
            section: self.section,
            loop_ref: self.loop_ref,
            points: None,
        }
    }
    pub(super) fn interval_error(&self, a: BoundaryPoint, b: BoundaryPoint) -> CorrespondenceError {
        CorrespondenceError::Numeric {
            section: self.section,
            loop_ref: self.loop_ref,
            points: Some([a, b]),
        }
    }
    pub(super) fn anchor(&self, i: usize) -> Anchor {
        if i == self.anchors.len() {
            Anchor {
                point: self.anchors[0].point,
                distance: self.total,
            }
        } else {
            self.anchors[i]
        }
    }
    pub(super) fn boundary_distance(&self) -> f64 {
        let d = self.prefix[self.boundary];
        if d >= self.first {
            d - self.first
        } else {
            self.total - self.first + d
        }
    }
    pub(super) fn location(&self, distance: f64) -> Result<BoundaryPoint, CorrespondenceError> {
        let mut d = self.first + distance;
        if d >= self.total {
            d -= self.total;
        }
        if !(0.0..self.total).contains(&d) {
            return Err(self.numeric());
        }
        let segment = self.prefix.partition_point(|&p| p <= d).saturating_sub(1);
        let parameter =
            (d - self.prefix[segment]) / (self.prefix[segment + 1] - self.prefix[segment]);
        if !(0.0..1.0).contains(&parameter) {
            return Err(self.numeric());
        }
        Ok(BoundaryPoint::new(segment, parameter))
    }
    fn point(&self, p: BoundaryPoint) -> [f64; 2] {
        let n = self.ring.segments().len();
        let from = self.ring.segments()[(p.segment + n - 1) % n].to;
        if p.parameter == 0.0 {
            return from;
        }
        self.arcs[p.segment].map_or_else(
            || {
                core::array::from_fn(|k| {
                    from[k] + p.parameter * (self.ring.segments()[p.segment].to[k] - from[k])
                })
            },
            |a| a.point_at(p.parameter),
        )
    }
    pub(super) fn piece(
        &self,
        source: &Loop2,
        a: BoundaryPoint,
        b: BoundaryPoint,
    ) -> Result<(Seg2, ProfileSegmentSpan), CorrespondenceError> {
        let n = self.ring.segments().len();
        let end = if b.segment == a.segment && b.parameter > a.parameter {
            b.parameter
        } else if b.segment == (a.segment + 1) % n && b.parameter == 0.0 {
            1.0
        } else {
            return Err(self.interval_error(a, b));
        };
        if end <= a.parameter {
            return Err(self.interval_error(a, b));
        }
        let mut segment = source.segs()[a.segment].clone();
        let [x, y] = self.point(b);
        segment.to = (x, y).into();
        if let Some(arc) = self.arcs[a.segment]
            && (a.parameter != 0.0 || end != 1.0)
        {
            segment.kind = SegKind::Arc {
                bulge: libm::tan(arc.sweep * (end - a.parameter) * 0.25),
            };
        }
        Ok((
            segment,
            ProfileSegmentSpan {
                source: SegmentRef {
                    loop_ref: self.loop_ref,
                    segment: a.segment,
                },
                parameter: [a.parameter, end],
            },
        ))
    }
}
