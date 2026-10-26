// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reusable cutout placement with occurrence-specific failures.

use super::{Profile2, ProfileError};

/// Why a reusable cutout pattern could not be added to a profile.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CutoutPatternError {
    /// The template cannot be used as a filled cutout.
    Template(ProfileError),
    /// A particular translation could not preserve valid segment geometry.
    Placement {
        /// Zero-based offset in the supplied iterator.
        placement: usize,
        /// Original coordinate/loop error.
        error: ProfileError,
    },
    /// The combined profile violates containment or hole separation.
    Profile(ProfileError),
}

impl core::fmt::Display for CutoutPatternError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Template(error) => write!(f, "cutout template: {error}"),
            Self::Placement { placement, error } => {
                write!(f, "cutout placement {placement}: {error}")
            }
            Self::Profile(error) => write!(f, "cutout pattern: {error}"),
        }
    }
}
impl core::error::Error for CutoutPatternError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(match self {
            Self::Template(error) | Self::Profile(error) | Self::Placement { error, .. } => error,
        })
    }
}

impl Profile2 {
    /// Places copies of one reusable cutout at the supplied 2D offsets.
    ///
    /// Tags and curve kinds survive placement. The cutout must have no holes;
    /// all copies are appended in iterator order and the combined profile is
    /// checked once. An empty iterator leaves the profile's geometry unchanged.
    ///
    /// ```
    /// use exedra_constructive::{builders, profile::SegTag};
    /// let bore = builders::circle(2.0)?.tagged(SegTag(100));
    /// let plate = builders::rect_centered(40.0, 20.0)?
    ///     .with_cutouts_at(&bore, [(-10.0, 0.0), (10.0, 0.0)])?;
    /// assert_eq!(plate.holes().len(), 2);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::CutoutHasHoles`] for a nested cutout,
    /// [`CutoutPatternError::Placement`] with the failing placement index,
    /// or the combined profile's containment/contact error.
    pub fn with_cutouts_at<I, P>(
        self,
        cutout: &Self,
        offsets: I,
    ) -> Result<Self, CutoutPatternError>
    where
        I: IntoIterator<Item = P>,
        P: Into<kurbo::Vec2>,
    {
        if !cutout.holes.is_empty() {
            return Err(CutoutPatternError::Template(ProfileError::CutoutHasHoles {
                cutout: 0,
            }));
        }
        let Self { outer, mut holes } = self;
        let offsets = offsets.into_iter();
        holes.reserve(offsets.size_hint().0);
        for (placement, offset) in offsets.enumerate() {
            let placed = cutout
                .outer
                .translated(offset)
                .map_err(|error| CutoutPatternError::Placement { placement, error })?;
            holes.push(placed.reversed());
        }
        Self::new(outer, holes).map_err(CutoutPatternError::Profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{builders, profile::SegTag};

    #[test]
    fn patterns_preserve_the_template_and_match_individually_placed_cutouts() {
        let bore = builders::circle(2.0).unwrap().tagged(SegTag(700));
        let offsets = [(-10.0, 0.0), (10.0, 0.0)];
        let plate = builders::rect_centered(40.0, 20.0).unwrap();
        let old = plate
            .clone()
            .with_cutouts(offsets.map(|at| bore.translated(at).unwrap()))
            .unwrap();
        let patterned = plate.clone().with_cutouts_at(&bore, offsets).unwrap();
        assert_eq!(patterned, old);
        assert_eq!(bore.outer().segs()[0].tag, Some(SegTag(700)));
        assert_eq!(
            plate
                .clone()
                .with_cutouts_at(&bore, core::iter::empty::<(f64, f64)>())
                .unwrap(),
            plate
        );
    }

    #[test]
    fn failures_distinguish_a_placement_from_the_combined_arrangement() {
        let bore = builders::circle(2.0).unwrap();
        let plate = || builders::rect_centered(40.0, 20.0).unwrap();
        let error = plate()
            .with_cutouts_at(&bore, [(-10.0, 0.0), (f64::NAN, 0.0)])
            .unwrap_err();
        assert!(matches!(
            error,
            CutoutPatternError::Placement {
                placement: 1,
                error: ProfileError::NonFinite { .. }
            }
        ));
        assert!(matches!(
            plate().with_cutouts_at(&bore, [(0.0, 0.0), (0.0, 0.0)]),
            Err(CutoutPatternError::Profile(
                ProfileError::OverlappingHoles {
                    first: 0,
                    second: 1
                }
            ))
        ));
        assert!(matches!(
            plate().with_cutouts_at(&builders::ring(3.0, 1.0).unwrap(), [(0.0, 0.0)]),
            Err(CutoutPatternError::Template(ProfileError::CutoutHasHoles {
                cutout: 0
            }))
        ));
    }
}
