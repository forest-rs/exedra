// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact axis-aligned reflection for profile authoring.

use super::SegKind;
use kurbo::Point;

/// A horizontal or vertical reflection line in profile coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MirrorLine {
    /// The line `x = coordinate`.
    Vertical(f64),
    /// The line `y = coordinate`.
    Horizontal(f64),
}

impl MirrorLine {
    pub(super) fn coordinate(self) -> f64 {
        match self {
            Self::Vertical(at) | Self::Horizontal(at) => at,
        }
    }

    pub(super) fn contains(self, p: Point) -> bool {
        match self {
            Self::Vertical(at) => p.x == at,
            Self::Horizontal(at) => p.y == at,
        }
    }

    pub(super) fn reflect(self, p: Point) -> Point {
        // Avoid overflowing 2*at when the final reflected coordinate is finite.
        match self {
            Self::Vertical(at) => Point::new(at + (at - p.x), p.y),
            Self::Horizontal(at) => Point::new(p.x, at + (at - p.y)),
        }
    }

    pub(super) fn reflect_kind(self, kind: &SegKind) -> SegKind {
        match kind {
            SegKind::Line => SegKind::Line,
            SegKind::Arc { bulge } => SegKind::Arc { bulge: -*bulge },
            SegKind::Cubic { c1, c2 } => SegKind::Cubic {
                c1: self.reflect(*c1),
                c2: self.reflect(*c2),
            },
            SegKind::PolicyTo { policy, realized } => SegKind::PolicyTo {
                policy: *policy,
                realized: alloc::boxed::Box::new(self.reflect_kind(realized)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{LoopCursor, Profile2, ProfileError, Seg2, SegTag};

    #[test]
    fn mirrored_arc_closes_a_circle_with_tags_and_exact_endpoints() {
        let circle = LoopCursor::new((0.0, -1.0))
            .arc_through((1.0, 0.0), (0.0, 1.0))
            .tagged(SegTag(10))
            .close_mirrored(MirrorLine::Vertical(0.0))
            .unwrap();
        assert_eq!(circle.segs().len(), 2);
        assert_eq!(circle.segs()[0].to, Point::new(0.0, 1.0));
        assert_eq!(circle.segs()[1].to, Point::new(0.0, -1.0));
        assert!(circle.segs().iter().all(|s| s.tag == Some(SegTag(10))));
        let profile = Profile2::simple(circle)
            .unwrap()
            .to_profile(&exedra_curves::profile::ProfilePolicy::new(1e-9))
            .unwrap();
        assert!((profile.area() - core::f64::consts::PI).abs() < 1e-14);
    }

    #[test]
    fn mirror_reverses_cubic_handles_and_supports_offset_horizontal_seams() {
        let outline = LoopCursor::new((1.0, 3.0))
            .segment(Seg2::cubic((3.0, 3.0), (1.0, 1.0), (3.0, 1.0)))
            .tagged(SegTag(11))
            .close_mirrored(MirrorLine::Horizontal(3.0))
            .unwrap();
        assert_eq!(
            outline.segs()[1],
            Seg2::cubic((1.0, 3.0), (3.0, 5.0), (1.0, 5.0)).tagged(SegTag(11))
        );
        assert!(outline.signed_area() > 0.0);
    }

    #[test]
    fn mirror_refuses_seam_gaps_and_preserves_first_drawing_failure() {
        let half = || {
            LoopCursor::new((0.0, 0.0))
                .line_to((2.0, 0.0))
                .line_to((1e-12, 2.0))
        };
        assert!(matches!(
            half().close_mirrored(MirrorLine::Vertical(0.0)),
            Err(ProfileError::MirrorEndpointOffAxis { endpoint: 1, .. })
        ));
        assert!(matches!(
            half().close_mirrored(MirrorLine::Vertical(f64::NAN)),
            Err(ProfileError::InvalidMirrorLine { .. })
        ));
        assert_eq!(
            LoopCursor::new((0.0, 0.0))
                .arc_through((1.0, 0.0), (2.0, 0.0))
                .close_mirrored(MirrorLine::Vertical(0.0)),
            Err(ProfileError::DegenerateArc { seg: 0 })
        );
    }
}
