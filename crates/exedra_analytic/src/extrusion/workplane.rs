// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checked local sketch planes for independent native construction.

use super::{End, Extrusion, ExtrusionError, FRAME_TOLERANCE, FaceId, FaceRole};
use exedra_math::{FrameError, Placement3};
use exedra_profile::{Profile, SegTag};

/// A finite, right-handed orthonormal sketch plane, with local +Z outward.
///
/// Extruding on this plane creates an independent solid. It does not fuse
/// that solid with a host or extend the end-pocket model to arbitrary cuts.
///
/// ```
/// use exedra_analytic::extrusion::{End, Workplane};
/// use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
///
/// let outline = Loop::new(vec![
///     Segment::line([4.0, 0.0]), Segment::line([4.0, 2.0]),
///     Segment::line([0.0, 2.0]), Segment::line([0.0, 0.0]),
/// ])?;
/// let profile = Profile::new(outline, vec![], &ProfilePolicy::new(1e-9))?;
/// let plate = Workplane::XY.extrude(profile.clone(), 3.0)?;
/// let top = plate.workplane(plate.cap(End::End))?;
/// let second = top.offset(0.5)?.extrude(profile, 1.0)?;
/// assert_eq!(second.frame().transform_point([0.0; 3]), [0.0, 0.0, 3.5]);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Workplane {
    frame: Placement3,
}

impl Workplane {
    /// The world XY plane, with +Z outward.
    pub const XY: Self = Self {
        frame: Placement3::IDENTITY,
    };

    /// Checks a rigid frame for use as a sketch plane.
    ///
    /// # Errors
    ///
    /// Returns the frame's non-finite, reflection or orthonormality violation.
    pub fn new(frame: Placement3) -> Result<Self, FrameError> {
        let column = |axis: usize| frame.rows.map(|row| row[axis]);
        Placement3::try_from_orthonormal_axes(
            column(0),
            column(1),
            column(2),
            column(3),
            FRAME_TOLERANCE,
        )?;
        Ok(Self { frame })
    }

    /// The rigid frame placing this plane's local coordinates.
    #[must_use]
    pub const fn frame(self) -> Placement3 {
        self.frame
    }

    /// This plane translated in its own X/Y coordinates.
    ///
    /// # Errors
    ///
    /// Returns a frame violation if the translation is non-finite or overflows.
    pub fn translated(self, offset: [f64; 2]) -> Result<Self, FrameError> {
        Self::new(self.frame * Placement3::translate(offset[0], offset[1], 0.0))
    }

    /// This plane offset by `distance` along its local +Z normal.
    /// Negative distances move inward when the plane came from an outward face.
    ///
    /// # Errors
    ///
    /// Returns a frame violation if the distance is non-finite or overflows.
    pub fn offset(self, distance: f64) -> Result<Self, FrameError> {
        Self::new(self.frame * Placement3::translate(0.0, 0.0, distance))
    }

    /// Extrudes a local filled profile along this plane's +Z into a new solid.
    ///
    /// # Errors
    ///
    /// Returns [`ExtrusionError`] for an invalid length or topology size.
    pub fn extrude(self, profile: Profile, length: f64) -> Result<Extrusion, ExtrusionError> {
        Extrusion::new(profile, self.frame, length)
    }
}

/// Why a native face cannot provide a planar sketch frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WorkplaneError {
    /// The face ID is outside this solid's table.
    UnknownFace {
        /// The requested local face ID.
        face: FaceId,
    },
    /// A cylindrical wall has no single planar workplane.
    CurvedFace {
        /// The requested local face ID.
        face: FaceId,
    },
}

impl core::fmt::Display for WorkplaneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFace { face } => write!(f, "face {} is outside this solid", face.0),
            Self::CurvedFace { face } => {
                write!(f, "face {} is curved and has no planar workplane", face.0)
            }
        }
    }
}
impl core::error::Error for WorkplaneError {}

/// Why a label does not identify one pocket floor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FaceSelectionError {
    /// No pocket floor carries the label.
    MissingFloor {
        /// Requested caller label.
        tag: SegTag,
    },
    /// Several pocket floors carry the label; inspect `faces_tagged` to choose.
    AmbiguousFloor {
        /// Requested caller label.
        tag: SegTag,
        /// Number of matching floors.
        count: usize,
    },
}

impl core::fmt::Display for FaceSelectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingFloor { tag } => write!(f, "no pocket floor carries tag {}", tag.0),
            Self::AmbiguousFloor { tag, count } => write!(
                f,
                "{count} pocket floors carry tag {}; choose from faces_tagged",
                tag.0
            ),
        }
    }
}
impl core::error::Error for FaceSelectionError {}

impl Extrusion {
    /// The local face ID of the requested end cap.
    #[must_use]
    pub const fn cap(&self, end: End) -> FaceId {
        FaceId(match end {
            End::Start => 0,
            End::End => 1,
        })
    }

    /// The single pocket floor carrying `tag`, independent of insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`FaceSelectionError`] if the label names zero or several floors.
    pub fn floor_tagged(&self, tag: SegTag) -> Result<FaceId, FaceSelectionError> {
        let mut floors = self
            .faces_tagged(tag)
            .filter(|(_, face)| matches!(face.role, FaceRole::Floor { .. }));
        let first = floors
            .next()
            .ok_or(FaceSelectionError::MissingFloor { tag })?
            .0;
        let count = 1 + floors.count();
        if count > 1 {
            return Err(FaceSelectionError::AmbiguousFloor { tag, count });
        }
        Ok(first)
    }

    /// A checked plane on a planar face, with the same origin and axes as
    /// [`Self::attachment_frame`]. The origin lies on the face's supporting
    /// plane and is not guaranteed to be inside its trimmed boundary.
    ///
    /// # Errors
    ///
    /// Distinguishes an unknown local face ID from a cylindrical wall.
    pub fn workplane(&self, face: FaceId) -> Result<Workplane, WorkplaneError> {
        if face.index() >= self.body.faces().len() {
            return Err(WorkplaneError::UnknownFace { face });
        }
        self.attachment_frame(face)
            .map(|frame| Workplane { frame })
            .ok_or(WorkplaneError::CurvedFace { face })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_profile::{Loop, ProfilePolicy, Segment};

    const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);

    fn rectangle() -> Profile {
        let outline = Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::line([4.0, 2.0]),
            Segment::line([0.0, 2.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        Profile::new(outline, vec![], &POLICY).unwrap()
    }

    #[test]
    fn face_planes_construct_independent_solids_in_the_hosts_local_frame() {
        let frame =
            Placement3::rotate_y_then_translate(core::f64::consts::FRAC_PI_2, 10.0, 20.0, 30.0);
        let host = Workplane::new(frame)
            .unwrap()
            .extrude(rectangle(), 3.0)
            .unwrap();
        let top = host.workplane(host.cap(End::End)).unwrap();
        let positioned = top.translated([1.0, 2.0]).unwrap().offset(0.5).unwrap();
        let child = positioned.extrude(rectangle(), 2.0).unwrap();
        let expected = frame.transform_point([1.0, 2.0, 3.5]);
        let actual = child.frame().transform_point([0.0; 3]);
        for axis in 0..3 {
            assert!((actual[axis] - expected[axis]).abs() < 1e-12);
        }
        assert_eq!(child.volume(), 16.0);
        assert_eq!(
            host.volume(),
            24.0,
            "building on a plane leaves the host unchanged"
        );
        let bottom = host.workplane(host.cap(End::Start)).unwrap();
        let direction = bottom.frame().rows.map(|row| row[2]);
        let expected = frame.rows.map(|row| -row[2]);
        for axis in 0..3 {
            assert!((direction[axis] - expected[axis]).abs() < 1e-12);
        }
    }

    #[test]
    fn plane_refusals_distinguish_unknown_ids_curved_faces_and_invalid_frames() {
        let circle = Loop::new(vec![
            Segment::arc([-1.0, 0.0], 1.0),
            Segment::arc([1.0, 0.0], 1.0),
        ])
        .unwrap();
        let disk = Workplane::XY
            .extrude(Profile::new(circle, vec![], &POLICY).unwrap(), 2.0)
            .unwrap();
        assert_eq!(
            disk.workplane(FaceId(2)),
            Err(WorkplaneError::CurvedFace { face: FaceId(2) })
        );
        assert_eq!(
            disk.workplane(FaceId(100)),
            Err(WorkplaneError::UnknownFace { face: FaceId(100) })
        );
        assert!(Workplane::XY.offset(f64::INFINITY).is_err());
        assert!(Workplane::XY.translated([f64::NAN, 0.0]).is_err());
        assert!(
            Workplane::new(Placement3::from_axes(
                [2.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0; 3]
            ))
            .is_err()
        );
    }

    #[test]
    fn unique_floor_selection_refuses_missing_and_repeated_labels() {
        const LABEL: SegTag = SegTag(77);
        let host = Workplane::XY.extrude(rectangle(), 3.0).unwrap();
        assert_eq!(
            host.floor_tagged(LABEL),
            Err(FaceSelectionError::MissingFloor { tag: LABEL })
        );
        let region = |x: f64| {
            Loop::new(vec![
                Segment::line([x + 0.5, 0.5]),
                Segment::line([x + 0.5, 1.0]),
                Segment::line([x, 1.0]),
                Segment::line([x, 0.5]),
            ])
            .unwrap()
        };
        let first = super::super::Pocket::from_region(&region(0.5), End::End, 1.0).tagged(LABEL);
        let host = host.with_pocket(first, &POLICY).unwrap();
        let floor = host.floor_tagged(LABEL).unwrap();
        assert_eq!(host.face_area(floor), Some(0.25));
        let second = super::super::Pocket::from_region(&region(2.0), End::End, 1.0).tagged(LABEL);
        let host = host.with_pocket(second, &POLICY).unwrap();
        assert_eq!(
            host.floor_tagged(LABEL),
            Err(FaceSelectionError::AmbiguousFloor {
                tag: LABEL,
                count: 2
            })
        );
    }

    #[test]
    fn filled_pocket_input_refuses_islands_instead_of_discarding_them() {
        let outer = rectangle().outer().clone();
        let hole = Loop::new(vec![
            Segment::arc([1.25, 1.0], -1.0),
            Segment::arc([1.75, 1.0], -1.0),
        ])
        .unwrap();
        let profile = Profile::new(outer, vec![hole], &POLICY).unwrap();
        assert_eq!(
            super::super::Pocket::from_profile(&profile, End::End, 1.0),
            Err(super::super::PocketError::RegionHasHoles { count: 1 })
        );
    }
}
