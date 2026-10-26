// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction from independently authored, checked profile inputs.

use super::{Extrusion, ExtrusionError, Workplane};
use exedra_math::Placement3;
use exedra_profile::{IntoProfile, ProfilePolicy};

/// A profile could not be converted or its extrusion could not be constructed.
///
/// The original conversion error stays typed, including authoring-specific
/// curve and segment information. No approximation is attempted on failure.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileExtrusionError<E> {
    /// The input cannot provide an exact profile under the requested policy.
    Profile(E),
    /// The converted profile could not be extruded with these dimensions/frame.
    Extrusion(ExtrusionError),
}

impl<E: core::fmt::Display> core::fmt::Display for ProfileExtrusionError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Profile(error) => write!(f, "extrusion profile: {error}"),
            Self::Extrusion(error) => error.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ProfileExtrusionError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(match self {
            Self::Profile(error) => error,
            Self::Extrusion(error) => error,
        })
    }
}

impl Extrusion {
    /// Converts a profile input and extrudes it without caller-side adapters.
    ///
    /// Accepts owned or borrowed shared profiles, or other authoring types that
    /// implement [`IntoProfile`]. The input is checked against `policy` before
    /// construction; unsupported curves are refused by their authoring type.
    /// For a profile already validated under the desired policy, [`Self::new`]
    /// transfers it directly without repeating validation.
    ///
    /// # Errors
    ///
    /// Returns the original conversion error in [`ProfileExtrusionError::Profile`],
    /// or [`ExtrusionError`] in [`ProfileExtrusionError::Extrusion`].
    pub fn from_profile<P: IntoProfile>(
        profile: P,
        frame: Placement3,
        length: f64,
        policy: &ProfilePolicy,
    ) -> Result<Self, ProfileExtrusionError<P::Error>> {
        let profile = profile
            .into_profile(policy)
            .map_err(ProfileExtrusionError::Profile)?;
        Self::new(profile, frame, length).map_err(ProfileExtrusionError::Extrusion)
    }
}

impl Workplane {
    /// Extrudes an authored profile on this plane after checking it with `policy`.
    ///
    /// This accepts the same owned/borrowed inputs as [`Extrusion::from_profile`]
    /// and creates an independent solid along the plane's local +Z.
    ///
    /// # Errors
    ///
    /// Returns a typed profile-conversion or extrusion-construction failure.
    pub fn extrude_profile<P: IntoProfile>(
        self,
        profile: P,
        length: f64,
        policy: &ProfilePolicy,
    ) -> Result<Extrusion, ProfileExtrusionError<P::Error>> {
        Extrusion::from_profile(profile, self.frame(), length, policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_profile::{Loop, Profile, Segment};

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
            &ProfilePolicy::new(1e-9),
        )
        .unwrap()
    }

    #[test]
    fn borrowed_profile_reuses_the_plane_and_preserves_geometry() {
        let profile = square();
        let plane = Workplane::XY.offset(5.0).unwrap();
        let solid = plane
            .extrude_profile(&profile, 3.0, &ProfilePolicy::new(1e-9))
            .unwrap();
        assert_eq!(solid.profile(), &profile);
        assert_eq!(solid.frame(), plane.frame());
        assert_eq!(solid.volume(), 12.0);
    }

    #[test]
    fn conversion_and_construction_failures_remain_distinguishable() {
        struct Unsupported;
        impl IntoProfile for Unsupported {
            type Error = usize;
            fn into_profile(self, _: &ProfilePolicy) -> Result<Profile, usize> {
                Err(7)
            }
        }
        let policy = ProfilePolicy::new(1e-9);
        assert_eq!(
            Workplane::XY.extrude_profile(Unsupported, 2.0, &policy),
            Err(ProfileExtrusionError::Profile(7))
        );
        assert_eq!(
            Workplane::XY.extrude_profile(square(), -2.0, &policy),
            Err(ProfileExtrusionError::Extrusion(
                ExtrusionError::InvalidLength { length: -2.0 }
            ))
        );
    }
}
