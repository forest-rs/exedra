// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact queries on an [`Extrusion`], independent of any tessellation.
//!
//! Results are in local coordinates unless named otherwise; the rigid
//! [`Extrusion::frame`] places them. Measures are exact for the native
//! geometry, not for a realized mesh.

use exedra_math::Placement3;
use exedra_profile::{Curve, LoopRef, SegmentRef};

use super::{EdgeCurve, EdgeId, End, Extrusion, FaceId, FaceRole, Surface};

/// A point on a face with its outward unit normal, in local coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SurfacePoint {
    /// The point.
    pub point: [f64; 3],
    /// The outward unit normal there.
    pub normal: [f64; 3],
}

impl Extrusion {
    /// Evaluates face `face` at `uv`.
    ///
    /// For a wall, `u` is the fraction along its profile segment, by angle for
    /// an arc, and `v` the height in `[0, length]`. For a cap, `uv` is a point
    /// of the profile plane; the cap's own boundary decides whether it lies on
    /// the face, which this does not check. Returns `None` for an unknown face
    /// or a wall coordinate outside `[0, 1] x [0, length]`.
    #[must_use]
    pub fn evaluate(&self, face: FaceId, uv: [f64; 2]) -> Option<SurfacePoint> {
        let face = self.faces.get(face.index())?;
        match face.role {
            FaceRole::Cap(end) => Some(SurfacePoint {
                point: [uv[0], uv[1], self.end_height(end)],
                normal: [0.0, 0.0, if end == End::Start { -1.0 } else { 1.0 }],
            }),
            FaceRole::Wall { segment, .. } => {
                let [u, v] = uv;
                if !(0.0..=1.0).contains(&u) || !(0.0..=self.length).contains(&v) {
                    return None;
                }
                let loop_ = self.loop_of(segment);
                let from = loop_.segments()
                    [(segment.segment + loop_.segments().len() - 1) % loop_.segments().len()]
                .to;
                let to = loop_.segments()[segment.segment].to;
                let (point, normal) = match face.surface {
                    Surface::Plane { normal, .. } => (
                        [
                            from[0] + u * (to[0] - from[0]),
                            from[1] + u * (to[1] - from[1]),
                        ],
                        [normal[0], normal[1]],
                    ),
                    Surface::Cylinder {
                        center, outward, ..
                    } => {
                        let arc = loop_.arc(segment.segment).expect("cylinder walls are arcs");
                        let point = arc.point_at(u);
                        let radial = [
                            (point[0] - center[0]) / arc.radius,
                            (point[1] - center[1]) / arc.radius,
                        ];
                        let sense = if outward { 1.0 } else { -1.0 };
                        (point, [sense * radial[0], sense * radial[1]])
                    }
                };
                Some(SurfacePoint {
                    point: [point[0], point[1], v],
                    normal: [normal[0], normal[1], 0.0],
                })
            }
        }
    }

    /// Exact length of edge `edge`.
    #[must_use]
    pub fn edge_length(&self, edge: EdgeId) -> Option<f64> {
        Some(match self.edges.get(edge.index())?.curve {
            EdgeCurve::Line { from, to } => exedra_math::norm(exedra_math::sub(to, from)),
            EdgeCurve::Arc { arc, .. } => arc.length(),
        })
    }

    /// Exact area of face `face`: the profile's area for a cap, the segment
    /// length times the extrusion length for a wall.
    #[must_use]
    pub fn face_area(&self, face: FaceId) -> Option<f64> {
        Some(match self.faces.get(face.index())?.role {
            FaceRole::Cap(_) => self.profile.area(),
            FaceRole::Wall { segment, .. } => {
                let loop_ = self.loop_of(segment);
                let length = match loop_.arc(segment.segment) {
                    Some(arc) => arc.length(),
                    None => {
                        let n = loop_.segments().len();
                        let from = loop_.segments()[(segment.segment + n - 1) % n].to;
                        let to = loop_.segments()[segment.segment].to;
                        libm::hypot(to[0] - from[0], to[1] - from[1])
                    }
                };
                length * self.length
            }
        })
    }

    /// Exact total boundary area.
    #[must_use]
    pub fn surface_area(&self) -> f64 {
        (0..self.faces.len())
            .filter_map(|face| self.face_area(FaceId(crate::usize_to_u32(face))))
            .sum()
    }

    /// Exact enclosed volume: the profile's area times the length. The solid
    /// is closed and its loops are oriented, so this is its signed volume.
    #[must_use]
    pub fn volume(&self) -> f64 {
        self.profile.area() * self.length
    }

    /// The smallest local axis-aligned box containing the solid, as
    /// `[min, max]`.
    #[must_use]
    pub fn local_bounds(&self) -> [[f64; 3]; 2] {
        let [min, max] = self.profile.bounds();
        [[min[0], min[1], 0.0], [max[0], max[1], self.length]]
    }

    /// A placed right-handed frame on planar face `face`, with +Z along its
    /// outward normal and its origin on the face's boundary. Cylindrical walls
    /// have no planar frame and return `None`.
    ///
    /// A cap's frame keeps the profile's X axis; its origin is the local
    /// origin projected onto the cap. A planar wall's frame runs +X along its
    /// segment and +Y along the extrusion from the segment's start at the
    /// start cap.
    #[must_use]
    pub fn attachment_frame(&self, face: FaceId) -> Option<Placement3> {
        let face = self.faces.get(face.index())?;
        let local = match (face.role, face.surface) {
            (FaceRole::Cap(End::End), _) => Placement3::translate(0.0, 0.0, self.length),
            (FaceRole::Cap(End::Start), _) => Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
                [0.0, 0.0, -1.0],
                [0.0; 3],
            ),
            (FaceRole::Wall { segment, .. }, Surface::Plane { normal, .. }) => {
                let loop_ = self.loop_of(segment);
                let n = loop_.segments().len();
                let from = loop_.segments()[(segment.segment + n - 1) % n].to;
                // The outward normal is the travel direction turned right, so
                // travel, +Z and the normal are right-handed.
                Placement3::from_axes(
                    [-normal[1], normal[0], 0.0],
                    [0.0, 0.0, 1.0],
                    normal,
                    [from[0], from[1], 0.0],
                )
            }
            (FaceRole::Wall { .. }, Surface::Cylinder { .. }) => return None,
        };
        Some(self.frame * local)
    }

    /// Whether `segment` is an arc, whose wall is a cylinder.
    #[must_use]
    pub fn is_cylindrical(&self, segment: SegmentRef) -> bool {
        matches!(
            self.loop_of(segment)
                .segments()
                .get(segment.segment)
                .map(|s| s.curve),
            Some(Curve::Arc { .. })
        )
    }

    fn end_height(&self, end: End) -> f64 {
        if end == End::Start { 0.0 } else { self.length }
    }

    fn loop_of(&self, segment: SegmentRef) -> &exedra_profile::Loop {
        match segment.loop_ref {
            LoopRef::Outer => self.profile.outer(),
            LoopRef::Hole(hole) => &self.profile.holes()[hole],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::f64::consts::PI;
    use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

    /// A 4 x 2 plate with a hole of radius 0.5, extruded by 3.
    fn plate(frame: Placement3) -> Extrusion {
        let outer = Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::line([4.0, 2.0]),
            Segment::line([0.0, 2.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        let hole = Loop::new(vec![
            Segment::arc([1.5, 1.0], -1.0),
            Segment::arc([2.5, 1.0], -1.0),
        ])
        .unwrap();
        let profile = Profile::new(outer, vec![hole], &ProfilePolicy::new(1e-9)).unwrap();
        Extrusion::new(profile, frame, 3.0).unwrap()
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() <= 1e-12 * b.abs().max(1.0), "{a} != {b}");
    }

    #[test]
    fn measures_follow_the_native_geometry() {
        let solid = plate(Placement3::IDENTITY);
        let area = 8.0 - PI / 4.0;
        close(solid.volume(), 3.0 * area);
        close(solid.face_area(FaceId(0)).unwrap(), area);
        // Caps, four planar walls (perimeter 12) and the hole's walls
        // (circumference pi).
        close(solid.surface_area(), 2.0 * area + 3.0 * (12.0 + PI));
        let seam = EdgeId(crate::usize_to_u32(solid.edges().len() - 1));
        close(solid.edge_length(seam).unwrap(), 3.0);
        close(solid.edge_length(EdgeId(4)).unwrap(), PI / 2.0);
        assert_eq!(solid.local_bounds(), [[0.0, 0.0, 0.0], [4.0, 2.0, 3.0]]);
        assert_eq!(solid.face_area(FaceId(99)), None);
    }

    #[test]
    fn evaluation_lies_on_the_support_with_outward_normals() {
        let solid = plate(Placement3::IDENTITY);
        for (index, face) in solid.faces().iter().enumerate().skip(2) {
            for uv in [[0.0, 0.0], [0.25, 1.0], [1.0, 3.0]] {
                let sample = solid
                    .evaluate(FaceId(crate::usize_to_u32(index)), uv)
                    .unwrap();
                close(exedra_math::norm(sample.normal), 1.0);
                match face.surface {
                    Surface::Plane { normal, offset } => {
                        assert_eq!(sample.normal, normal);
                        close(exedra_math::dot(normal, sample.point), offset);
                    }
                    Surface::Cylinder {
                        center,
                        radius,
                        outward,
                    } => {
                        let radial = [sample.point[0] - center[0], sample.point[1] - center[1]];
                        close(libm::hypot(radial[0], radial[1]), radius);
                        // The hole's walls face the axis.
                        assert!(!outward);
                        assert!(exedra_math::dot(sample.normal, [radial[0], radial[1], 0.0]) < 0.0);
                    }
                }
            }
        }
        assert!(solid.evaluate(FaceId(2), [1.5, 0.0]).is_none());
        assert!(solid.evaluate(FaceId(2), [0.5, 4.0]).is_none());
        assert_eq!(
            solid.evaluate(FaceId(1), [1.0, 1.0]).unwrap().point,
            [1.0, 1.0, 3.0]
        );
    }

    #[test]
    fn attachment_frames_sit_on_planar_faces_facing_out() {
        let frame = Placement3::rotate_z_then_translate(0.4, 10.0, -2.0, 1.0);
        let solid = plate(frame);
        for (index, face) in solid.faces().iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(index));
            let Some(attached) = solid.attachment_frame(id) else {
                assert!(matches!(face.surface, Surface::Cylinder { .. }));
                continue;
            };
            close(attached.linear_determinant(), 1.0);
            // +Z of the attachment is the placed outward normal.
            let z = attached.rows.map(|row| row[2]);
            let normal = match face.surface {
                Surface::Plane { normal, .. } => normal,
                Surface::Cylinder { .. } => unreachable!(),
            };
            let placed_normal = frame
                .rows
                .map(|row| row[0] * normal[0] + row[1] * normal[1] + row[2] * normal[2]);
            for (a, b) in z.iter().zip(placed_normal) {
                close(*a, b);
            }
        }
        assert!(solid.is_cylindrical(SegmentRef {
            loop_ref: LoopRef::Hole(0),
            segment: 1
        }));
    }
}
