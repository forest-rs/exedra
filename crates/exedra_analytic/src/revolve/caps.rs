// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spherical-cap authoring lowers to the same retained profile revolution.

use super::{Revolution, RevolutionError};
use alloc::vec;
use exedra_curves::profile::{Loop, LoopError, Profile, ProfileError, ProfilePolicy, Segment};
use exedra_math::Placement3;

/// A requested spherical cap could not preserve its authored dimensions.
#[derive(Clone, Debug, PartialEq)]
pub enum SphericalCapError {
    /// Radius must be finite and positive.
    Radius {
        /// Requested sphere radius in model units.
        radius: f64,
    },
    /// Height must be finite, positive and at most the sphere's diameter.
    Height {
        /// Requested sphere radius.
        radius: f64,
        /// Requested cap height.
        height: f64,
    },
    /// Subtracting the requested height loses a strictly partial cap's rim
    /// latitude. This refuses an epsilon ring or an accidental full sphere.
    HeightResolution {
        /// Requested sphere radius.
        radius: f64,
        /// Requested cap height.
        height: f64,
        /// Rounded base height relative to the sphere center.
        axis_offset: f64,
    },
    /// The required nonzero rim radius underflowed or overflowed.
    RimResolution {
        /// Requested sphere radius.
        radius: f64,
        /// Requested cap height.
        height: f64,
        /// Computed rim radius.
        rim_radius: f64,
    },
    /// Canonical source loop could not be represented.
    Loop(LoopError),
    /// Canonical source profile failed the caller's arrangement policy.
    Profile(ProfileError),
    /// Retained native construction was refused.
    Revolution(RevolutionError),
}
impl core::fmt::Display for SphericalCapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "spherical cap: {self:?}")
    }
}
impl core::error::Error for SphericalCapError {}

impl Revolution {
    /// Constructs an exact full-turn spherical cap, including hemispheres and
    /// the full sphere, through an authoritative center-defined profile arc.
    ///
    /// `radius` is the sphere radius and `height` is measured along local +Y
    /// from its planar base at Y=0. The supplied placement moves that frame.
    /// A partial cap retains the base as source segment 0 and spherical shell
    /// as segment 1; both remain wall roles, with no angular caps. At exactly
    /// the diameter the base disappears and the profile contains only the axis
    /// and spherical arc. The source profile remains available through
    /// [`Self::profile`]; this creates no separate native geometry family.
    ///
    /// ```
    /// use exedra_analytic::revolve::Revolution;
    /// use exedra_math::Placement3;
    /// use exedra_curves::profile::ProfilePolicy;
    /// let dome = Revolution::spherical_cap(20.0, 8.0,
    ///     Placement3::IDENTITY, &ProfilePolicy::new(1e-9))?;
    /// assert!(dome.body().is_closed());
    /// assert_eq!(dome.body().faces().len(), 2);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// Refuses invalid dimensions, lost height/rim resolution, invalid profile
    /// policy and native numeric or boundary failures. Separation validates the
    /// profile; it never heals or snaps dimensions to a sphere or an epsilon ring.
    pub fn spherical_cap(
        radius: f64,
        height: f64,
        frame: Placement3,
        policy: &ProfilePolicy,
    ) -> Result<Self, SphericalCapError> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err(SphericalCapError::Radius { radius });
        }
        if !height.is_finite() || height <= 0.0 || (height > radius && height - radius > radius) {
            return Err(SphericalCapError::Height { radius, height });
        }
        let axis_offset = radius - height;
        let center = [0.0, -axis_offset];
        let remaining = if height > radius {
            radius - (height - radius)
        } else {
            radius + (radius - height)
        };
        let full = remaining == 0.0;
        if !full && axis_offset.abs() >= radius {
            return Err(SphericalCapError::HeightResolution {
                radius,
                height,
                axis_offset,
            });
        }
        let segments = if full {
            vec![
                Segment::line([0.0, 0.0]),
                Segment::arc_centered([0.0, height], center, false),
            ]
        } else {
            // sqrt(h)*sqrt(2R-h) avoids cancellation in R²-(R-h)² and
            // premature under/overflow of the product h*(2R-h).
            let rim_radius = libm::sqrt(height) * libm::sqrt(remaining);
            if !rim_radius.is_finite() || rim_radius <= 0.0 {
                return Err(SphericalCapError::RimResolution {
                    radius,
                    height,
                    rim_radius,
                });
            }
            vec![
                Segment::line([rim_radius, 0.0]),
                Segment::arc_centered([0.0, height], center, false),
                Segment::line([0.0, 0.0]),
            ]
        };
        let outline = Loop::new(segments).map_err(SphericalCapError::Loop)?;
        let profile = Profile::new(outline, vec![], policy).map_err(SphericalCapError::Profile)?;
        Self::new(profile, frame, core::f64::consts::TAU).map_err(SphericalCapError::Revolution)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{geometry::Surface, realize::RealizationPolicy};
    use core::f64::consts::PI;
    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= expected.abs() * 2e-12,
            "{actual} vs {expected}"
        );
    }
    #[test]
    fn cap_dimensions_and_native_support_remain_inspectable() {
        for height in [0.25_f64, 1.0, 2.0, 3.5, 4.0] {
            let cap = Revolution::spherical_cap(
                2.0,
                height,
                Placement3::IDENTITY,
                &ProfilePolicy::new(1e-9),
            )
            .unwrap();
            assert!(cap.body().is_closed());
            assert_eq!(cap.profile().bounds()[0][1], 0.0);
            close(cap.profile().bounds()[1][1], height);
            close(cap.volume(), PI * height * height * (2.0 - height / 3.0));
            let base = if height == 4.0 {
                0.0
            } else {
                PI * height * (4.0 - height)
            };
            close(cap.surface_area(), 4.0 * PI * height + base);
            assert_eq!(
                cap.body()
                    .faces()
                    .iter()
                    .filter(|f| matches!(f.surface, Surface::Sphere(_)))
                    .count(),
                1
            );
            assert_eq!(
                cap.body()
                    .faces()
                    .iter()
                    .filter(|f| matches!(f.surface, Surface::Plane(_)))
                    .count(),
                usize::from(height != 4.0)
            );
            let mesh = cap
                .realize(&RealizationPolicy {
                    tolerance: 0.01,
                    ..Default::default()
                })
                .unwrap();
            assert!(mesh.report.within_tolerance());
            assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
        }
    }
    #[test]
    fn lost_height_is_refused_independently_of_manufacturing_separation() {
        let policy = ProfilePolicy::new(f64::from_bits(1));
        for (radius, height) in [(2.0, f64::from_bits(1)), (2.0, 1e-100), (1e16, 0.25)] {
            assert!(matches!(
                Revolution::spherical_cap(radius, height, Placement3::IDENTITY, &policy),
                Err(SphericalCapError::HeightResolution { .. })
            ));
        }
        for height in [0.0, -1.0, f64::NAN, f64::INFINITY, 4.0_f64.next_up()] {
            assert!(matches!(
                Revolution::spherical_cap(2.0, height, Placement3::IDENTITY, &policy),
                Err(SphericalCapError::Height { .. })
            ));
        }
        for radius in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                Revolution::spherical_cap(radius, 1.0, Placement3::IDENTITY, &policy),
                Err(SphericalCapError::Radius { .. })
            ));
        }
    }
    #[test]
    fn representable_near_full_cap_keeps_its_nonzero_base() {
        let height = 4.0_f64.next_down();
        let cap = Revolution::spherical_cap(
            2.0,
            height,
            Placement3::IDENTITY,
            &ProfilePolicy::new(1e-20),
        )
        .unwrap();
        assert_eq!(cap.body().faces().len(), 2);
        assert_eq!(
            cap.body()
                .edges()
                .iter()
                .filter(|edge| edge.is_collapsed())
                .count(),
            1
        );
        let rim = cap.profile().outer().segments()[0].to[0];
        assert!(
            rim > 0.0 && rim < 1e-6,
            "partial cap must not become a full sphere"
        );
    }
    #[test]
    fn accepted_shallow_caps_keep_stable_native_measures_at_small_heights() {
        // Source-profile circular-segment moments use their small-angle series;
        // do not replace them with a squared-radius subtraction or mesh estimate.
        for height in [
            1e-2,
            1e-4,
            1e-6,
            1e-8,
            1e-10,
            1e-12,
            1e-14,
            2.0_f64.next_up() - 2.0,
        ] {
            let cap = Revolution::spherical_cap(
                2.0,
                height,
                Placement3::IDENTITY,
                &ProfilePolicy::new(f64::from_bits(1)),
            )
            .unwrap();
            close(cap.volume(), PI * height * height * (2.0 - height / 3.0));
            close(
                cap.surface_area(),
                4.0 * PI * height + PI * height * (4.0 - height),
            );
            assert_eq!(cap.body().faces().len(), 2);
            assert!(cap.body().vertices().contains(&[0.0, height, 0.0]));
            assert!(cap.profile().outer().segments()[0].to[0] > 0.0);
        }
    }

    #[test]
    fn shallow_caps_cover_support_operand_roundoff_independently_of_separation() {
        for (radius, height) in [(25.0, 0.25), (25.0, 0.5), (60.0, 0.5)] {
            for scale in [1e-6, 1.0, 1e6] {
                let radius = radius * scale;
                let height = height * scale;
                let cap = Revolution::spherical_cap(
                    radius,
                    height,
                    Placement3::IDENTITY,
                    &ProfilePolicy::new(1e-9 * scale),
                )
                .unwrap();
                let coarser_separation = Revolution::spherical_cap(
                    radius,
                    height,
                    Placement3::IDENTITY,
                    &ProfilePolicy::new(1e-3 * scale),
                )
                .unwrap();
                assert_eq!(cap.body(), coarser_separation.body());
                let agreement = cap
                    .body()
                    .validate(&crate::body::ValidationPolicy {
                        max_gap: 1e-9 * scale,
                    })
                    .unwrap();
                assert!(agreement.max_gap > 0.0);
                assert!(matches!(
                    cap.body().validate(&crate::body::ValidationPolicy {
                        max_gap: 0.5 * agreement.max_gap,
                    }),
                    Err(crate::body::BodyError::SurfaceGap { bound, allowed, .. })
                        if bound > allowed
                ));
                assert!(cap.body().is_closed());
                assert!(cap.body().vertices().contains(&[0.0, height, 0.0]));
                close(cap.volume(), PI * height * height * (radius - height / 3.0));
                close(
                    cap.surface_area(),
                    2.0 * PI * radius * height + PI * height * (2.0 * radius - height),
                );
            }
        }
    }
}
