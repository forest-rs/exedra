// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checked conversion into the shared exact profile representation.

use crate::profile::{Profile, ProfileError, ProfilePolicy};

/// An authored input that can become an exact line-and-arc [`Profile`].
///
/// Geometry heads accept this trait without depending on the input's authoring
/// crate. Conversion must preserve endpoints and feature tags, validate against
/// the supplied policy, and refuse unsupported curves rather than approximate
/// them. An owned input may transfer its storage; borrowed inputs may copy it.
pub trait IntoProfile {
    /// A conversion, unsupported-curve, or profile-validation failure.
    type Error;

    /// Converts and validates this input under `policy`.
    fn into_profile(self, policy: &ProfilePolicy) -> Result<Profile, Self::Error>;
}

impl IntoProfile for Profile {
    type Error = ProfileError;

    fn into_profile(self, policy: &ProfilePolicy) -> Result<Profile, Self::Error> {
        self.validate(policy)?;
        Ok(self)
    }
}

impl IntoProfile for &Profile {
    type Error = ProfileError;

    fn into_profile(self, policy: &ProfilePolicy) -> Result<Profile, Self::Error> {
        self.validate(policy)?;
        Ok(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Loop, Segment};
    use alloc::vec;

    #[test]
    fn owned_conversion_keeps_storage_but_honors_a_stronger_policy() {
        let policy = ProfilePolicy::new(1e-9);
        let profile = Profile::new(
            Loop::new(vec![
                Segment::line([2.0, 0.0]),
                Segment::line([2.0, 2.0]),
                Segment::line([0.0, 2.0]),
                Segment::line([0.0, 0.0]),
            ])
            .unwrap(),
            vec![],
            &policy,
        )
        .unwrap();
        let pointer = profile.outer().segments().as_ptr();
        let profile = profile.into_profile(&policy).unwrap();
        assert_eq!(pointer, profile.outer().segments().as_ptr());
        assert!(profile.into_profile(&ProfilePolicy::new(3.0)).is_err());
    }
}
