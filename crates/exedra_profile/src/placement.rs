// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checked placement of shared profile geometry without curve sampling.

use crate::{Loop, LoopError, LoopRef, Profile, ProfileError, ProfilePolicy};
use alloc::vec::Vec;

/// Why translating a profile cannot preserve its validated geometry.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ProfileTranslationError {
    /// Rounded coordinates no longer form a representable closed boundary.
    Loop {
        /// Boundary being translated.
        loop_ref: LoopRef,
        /// Structural or numeric refusal.
        error: LoopError,
    },
    /// The moved boundaries fail the requested arrangement policy.
    Profile(ProfileError),
}
impl core::fmt::Display for ProfileTranslationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Loop { loop_ref, error } => write!(f, "translated {loop_ref:?}: {error}"),
            Self::Profile(error) => write!(f, "translated profile: {error}"),
        }
    }
}
impl core::error::Error for ProfileTranslationError {}

impl Loop {
    /// Moves endpoints and explicit arc centers by `offset`, preserving direction, bulges, tags and order.
    ///
    /// # Errors
    ///
    /// Returns a structural or numeric refusal if floating-point translation
    /// makes coordinates non-finite, collapses an edge or invalidates an arc.
    pub fn translated(&self, offset: [f64; 2]) -> Result<Self, LoopError> {
        Self::new(
            self.segments()
                .iter()
                .map(|segment| {
                    let mut moved = *segment;
                    moved.to = [segment.to[0] + offset[0], segment.to[1] + offset[1]];
                    if let crate::Curve::ArcCenter { center, clockwise } = segment.curve {
                        moved.curve = crate::Curve::ArcCenter {
                            center: core::array::from_fn(|i| {
                                if offset[i] == 0.0 {
                                    center[i]
                                } else {
                                    center[i] + offset[i]
                                }
                            }),
                            clockwise,
                        };
                    }
                    moved
                })
                .collect(),
        )
    }
}
impl Profile {
    /// Moves all boundaries without sampling curves or changing segment tags.
    /// Floating-point output is validated under `policy`; no points are healed.
    ///
    /// # Errors
    ///
    /// Returns the boundary and numeric failure, or the moved arrangement's
    /// policy violation. Translation can fail at far origins even when its
    /// mathematical counterpart is rigid.
    pub fn translated(
        &self,
        offset: [f64; 2],
        policy: &ProfilePolicy,
    ) -> Result<Self, ProfileTranslationError> {
        let outer =
            self.outer()
                .translated(offset)
                .map_err(|error| ProfileTranslationError::Loop {
                    loop_ref: LoopRef::Outer,
                    error,
                })?;
        let holes = self
            .holes()
            .iter()
            .enumerate()
            .map(|(index, boundary)| {
                boundary
                    .translated(offset)
                    .map_err(|error| ProfileTranslationError::Loop {
                        loop_ref: LoopRef::Hole(index),
                        error,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(outer, holes, policy).map_err(ProfileTranslationError::Profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CornerTreatment, Curve, SegTag, Segment};
    use alloc::vec;
    const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);
    fn square() -> Profile {
        Profile::new(
            Loop::new(vec![
                Segment::line([2.0, 0.0]),
                Segment::line([2.0, 2.0]),
                Segment::line([0.0, 2.0]),
                Segment::line([0.0, 0.0]),
            ])
            .unwrap(),
            vec![],
            &POLICY,
        )
        .unwrap()
    }
    #[test]
    fn placement_and_whole_tags_preserve_treated_curves_and_boundary_order() {
        let source = square().tagged(SegTag(5));
        let selected = source.corner_at([2.0, 2.0], 0.0).unwrap();
        let rounded = source
            .with_corners(
                [selected.segment],
                CornerTreatment::fillet(0.5).tagged(SegTag(9)),
                &POLICY,
            )
            .unwrap()
            .profile;
        let moved = rounded.translated([10.0, 20.0], &POLICY).unwrap();
        for (a, b) in rounded
            .outer()
            .segments()
            .iter()
            .zip(moved.outer().segments())
        {
            assert_eq!(b.to, [a.to[0] + 10.0, a.to[1] + 20.0]);
            assert_eq!(a.curve, b.curve);
            assert_eq!(a.tag, b.tag);
        }
        assert!(
            moved
                .outer()
                .segments()
                .iter()
                .any(|s| matches!(s.curve, Curve::Arc { .. }) && s.tag == Some(SegTag(9)))
        );
        assert!((moved.area() - rounded.area()).abs() < 1e-12);
        let all = moved.tagged(SegTag(11));
        assert!(
            all.outer()
                .segments()
                .iter()
                .all(|s| s.tag == Some(SegTag(11)))
        );
    }
    #[test]
    fn translation_refuses_coordinate_collapse_and_stronger_arrangement_policy() {
        assert!(matches!(
            square().translated([1e20, 0.0], &POLICY),
            Err(ProfileTranslationError::Loop {
                error: LoopError::ZeroLength { .. },
                ..
            })
        ));
        assert!(matches!(
            square().translated([f64::NAN, 0.0], &POLICY),
            Err(ProfileTranslationError::Loop {
                error: LoopError::NonFinite { .. },
                ..
            })
        ));
        assert!(matches!(
            square().translated([0.0, 0.0], &ProfilePolicy::new(3.0)),
            Err(ProfileTranslationError::Profile(_))
        ));
    }
}

#[cfg(test)]
mod centered_tests {
    use super::*;
    use crate::{CornerTreatment, Curve, SegTag, Segment};
    use alloc::vec;
    #[test]
    fn unchanged_center_components_keep_authored_signed_zero() {
        let boundary = Loop::new(vec![
            Segment::arc_centered([-1.0, 0.0], [-0.0, 0.0], false),
            Segment::arc_centered([1.0, 0.0], [-0.0, 0.0], false),
        ])
        .unwrap();
        for offset in [[0.0; 2], [0.0, 10.0]] {
            let moved = boundary.translated(offset).unwrap();
            let Curve::ArcCenter { center, .. } = moved.segments()[0].curve else {
                panic!()
            };
            assert_eq!(center[0].to_bits(), (-0.0_f64).to_bits());
        }
    }
    #[test]
    fn center_survives_reversal_tagging_translation_and_unselected_corner_treatment() {
        let policy = ProfilePolicy::new(1e-9);
        let boundary = Loop::new(vec![
            Segment::line([0.0, 0.0]),
            Segment::line([1.0, 0.0]),
            Segment::arc_centered([0.0, 1.0], [0.0; 2], false),
        ])
        .unwrap();
        assert!((boundary.signed_area() - core::f64::consts::PI / 4.0).abs() < 1e-15);
        let reversed = boundary.reversed();
        assert!(matches!(
            reversed.segments()[0].curve,
            Curve::ArcCenter {
                center: [0.0, 0.0],
                clockwise: true
            }
        ));
        assert_eq!(reversed.reversed(), boundary);
        let source = Profile::new(boundary, vec![], &policy)
            .unwrap()
            .tagged(SegTag(4));
        let moved = source.translated([10.0, 20.0], &policy).unwrap();
        assert!(moved.outer().segments().iter().any(|s| s.curve
            == Curve::ArcCenter {
                center: [10.0, 20.0],
                clockwise: false
            }));
        assert!((moved.area() - source.area()).abs() < 1e-13);
        let corner = source.corner_at([0.0, 0.0], 0.0).unwrap();
        let treated = source
            .with_corners(
                [corner.segment],
                CornerTreatment::fillet(0.1).tagged(SegTag(9)),
                &policy,
            )
            .unwrap();
        assert!(treated.profile.outer().segments().iter().any(|s| s.curve
            == Curve::ArcCenter {
                center: [0.0; 2],
                clockwise: false
            }
            && s.tag == Some(SegTag(4))));
        assert!(
            source.translated([1e16, 0.0], &policy).is_err(),
            "rounded placement cannot heal an unrepresentable circle"
        );
    }
}
