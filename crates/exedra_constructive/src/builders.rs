// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Generic profile builders.
//!
//! Deliberately spec-agnostic conveniences: rectangles, rounded rectangles,
//! L-shaped corner profiles, circles, and rings. External frontends compose
//! spec-driven shapes from these (and from raw segments) without any spec
//! vocabulary living here.
//!
//! All builders produce validated counter-clockwise [`Profile2`] values in
//! the XY plane with the origin at the profile's minimum corner (or center,
//! for circular shapes). Each segment is tagged with its index so downstream
//! source maps can name features without the caller doing anything; callers
//! that want their own tags rebuild the loops with [`Seg2`] directly.

use alloc::vec;
use alloc::vec::Vec;

use kurbo::Point;

use crate::profile::{Loop2, Profile2, ProfileError, Seg2, SegTag};

/// Half-circle bulge: `tan(pi / 8)`, the bulge of a 90-degree arc.
///
/// Computed once from libm so every rounded builder shares identical bits.
fn quarter_arc_bulge() -> f64 {
    libm::tan(core::f64::consts::FRAC_PI_8)
}

fn positive_finite(v: f64) -> bool {
    v.is_finite() && v > 0.0
}

/// An axis-aligned rectangle: origin corner, counter-clockwise.
///
/// Segments (and their tags) in order: bottom, right, top, left.
///
/// # Errors
///
/// Both dimensions must be positive and finite.
pub fn rect(width: f64, height: f64) -> Result<Profile2, ProfileError> {
    if !positive_finite(width) || !positive_finite(height) {
        return Err(ProfileError::InvalidDimension);
    }
    let outer = Loop2::new(vec![
        Seg2::line((width, 0.0)).tagged(SegTag(0)),
        Seg2::line((width, height)).tagged(SegTag(1)),
        Seg2::line((0.0, height)).tagged(SegTag(2)),
        Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
    ])?;
    Profile2::simple(outer)
}

/// An axis-aligned rectangle with four rounded corners of equal radius.
///
/// Alternates straight edges and 90-degree arcs, eight segments total,
/// tagged 0..8 starting from the bottom edge.
///
/// # Errors
///
/// Dimensions must be positive and finite, and the radius must be positive
/// and strictly less than half of each dimension.
pub fn rounded_rect(width: f64, height: f64, radius: f64) -> Result<Profile2, ProfileError> {
    if !positive_finite(width) || !positive_finite(height) || !positive_finite(radius) {
        return Err(ProfileError::InvalidDimension);
    }
    if radius >= 0.5 * width || radius >= 0.5 * height {
        return Err(ProfileError::InvalidDimension);
    }
    let b = quarter_arc_bulge();
    let outer = Loop2::new(vec![
        Seg2::line((width - radius, 0.0)).tagged(SegTag(0)),
        Seg2::arc((width, radius), b).tagged(SegTag(1)),
        Seg2::line((width, height - radius)).tagged(SegTag(2)),
        Seg2::arc((width - radius, height), b).tagged(SegTag(3)),
        Seg2::line((radius, height)).tagged(SegTag(4)),
        Seg2::arc((0.0, height - radius), b).tagged(SegTag(5)),
        Seg2::line((0.0, radius)).tagged(SegTag(6)),
        Seg2::arc((radius, 0.0), b).tagged(SegTag(7)),
    ])?;
    Profile2::simple(outer)
}

/// An L-shaped corner profile: a `width x height` rectangle with a
/// `notch_width x notch_height` rectangle removed from its top-right corner.
///
/// Six segments, counter-clockwise from the origin, tagged 0..6.
///
/// # Errors
///
/// All dimensions must be positive and finite, and the notch must be
/// strictly smaller than the rectangle in both axes.
pub fn l_profile(
    width: f64,
    height: f64,
    notch_width: f64,
    notch_height: f64,
) -> Result<Profile2, ProfileError> {
    if !positive_finite(width)
        || !positive_finite(height)
        || !positive_finite(notch_width)
        || !positive_finite(notch_height)
        || notch_width >= width
        || notch_height >= height
    {
        return Err(ProfileError::InvalidDimension);
    }
    let outer = Loop2::new(vec![
        Seg2::line((width, 0.0)).tagged(SegTag(0)),
        Seg2::line((width, height - notch_height)).tagged(SegTag(1)),
        Seg2::line((width - notch_width, height - notch_height)).tagged(SegTag(2)),
        Seg2::line((width - notch_width, height)).tagged(SegTag(3)),
        Seg2::line((0.0, height)).tagged(SegTag(4)),
        Seg2::line((0.0, 0.0)).tagged(SegTag(5)),
    ])?;
    Profile2::simple(outer)
}

/// A circle of the given radius centered at the origin.
///
/// Two half-circle arcs (bulge 1) meeting at `(-radius, 0)` and
/// `(radius, 0)`, tagged 0 (lower half) and 1 (upper half).
///
/// # Errors
///
/// The radius must be positive and finite.
pub fn circle(radius: f64) -> Result<Profile2, ProfileError> {
    Profile2::simple(circle_loop(radius, false)?)
}

/// An annulus: outer circle with a concentric circular hole.
///
/// # Errors
///
/// Radii must be positive and finite with `inner_radius < outer_radius`.
pub fn ring(outer_radius: f64, inner_radius: f64) -> Result<Profile2, ProfileError> {
    if !positive_finite(inner_radius) || inner_radius >= outer_radius {
        return Err(ProfileError::InvalidDimension);
    }
    let outer = circle_loop(outer_radius, false)?;
    let hole = circle_loop(inner_radius, true)?;
    Profile2::new(outer, vec![hole])
}

fn circle_loop(radius: f64, clockwise: bool) -> Result<Loop2, ProfileError> {
    if !positive_finite(radius) {
        return Err(ProfileError::InvalidDimension);
    }
    let ccw = Loop2::new(vec![
        Seg2::arc((radius, 0.0), 1.0).tagged(SegTag(0)),
        Seg2::arc((-radius, 0.0), 1.0).tagged(SegTag(1)),
    ])?;
    Ok(if clockwise { ccw.reversed() } else { ccw })
}

/// Signed area of a profile: outer area minus hole areas (holes wind
/// clockwise, so their signed areas are negative and simply add).
///
/// Computed via the kurbo path rendering — exact for polylines and cubics,
/// close for arcs.
#[must_use]
pub fn profile_area(profile: &Profile2) -> f64 {
    let mut area = profile.outer().signed_area();
    for hole in profile.holes() {
        area += hole.signed_area();
    }
    area
}

/// Area centroid of a profile (holes subtracted), via the kurbo rendering.
///
/// Returns `None` when the net area is zero.
#[must_use]
pub fn profile_centroid(profile: &Profile2) -> Option<Point> {
    use kurbo::Shape;
    let mut area = 0.0;
    let mut moment = kurbo::Vec2::ZERO;
    let mut acc = |loop_: &Loop2| {
        let path = loop_.to_bez_path();
        let a = path.area();
        // First moment via the bounding centroid of flattened segments is
        // not exact; kurbo offers per-path moments through integration on
        // the path's segments.
        let m = path_moment(&path);
        area += a;
        moment += m;
    };
    acc(profile.outer());
    for hole in profile.holes() {
        acc(hole);
    }
    (area != 0.0).then(|| (moment / area).to_point())
}

/// First moment of area of a closed path (Green's theorem over segments).
fn path_moment(path: &kurbo::BezPath) -> kurbo::Vec2 {
    use kurbo::ParamCurve;
    let mut mx = 0.0;
    let mut my = 0.0;
    for seg in path.segments() {
        // Numerical integration with a fixed 16-point midpoint rule per
        // segment: deterministic, and accurate enough for centroids used in
        // diagnostics and builder tests (not in geometry output).
        const N: usize = 16;
        let mut prev = seg.eval(0.0);
        for i in 1..=N {
            let t = i as f64 / N as f64;
            let p = seg.eval(t);
            let cross = prev.x * p.y - p.x * prev.y;
            mx += (prev.x + p.x) * cross;
            my += (prev.y + p.y) * cross;
            prev = p;
        }
    }
    kurbo::Vec2::new(mx / 6.0, my / 6.0)
}

/// Classification of a profile's convexity, from its discretized outline.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProfileShapeClass {
    /// Every outer vertex turns the same way and there are no holes.
    Convex,
    /// The outer loop has reflex vertices, or holes exist.
    Concave,
}

/// Classifies a profile by discretizing its outer loop coarsely and
/// checking turn directions. Any hole, or any inability to realize the
/// coarse default-policy discretization within its accuracy and numeric
/// contract, is conservatively classified as concave.
#[must_use]
pub fn classify_profile(profile: &Profile2) -> ProfileShapeClass {
    if !profile.holes().is_empty() {
        return ProfileShapeClass::Concave;
    }
    let policy = crate::discretize::DiscretizePolicy::default();
    let Ok(d) = crate::discretize::discretize_loop(profile.outer(), &policy) else {
        return ProfileShapeClass::Concave;
    };
    let n = d.points.len();
    for i in 0..n {
        let a = d.points[i];
        let b = d.points[(i + 1) % n];
        let c = d.points[(i + 2) % n];
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross < 0.0 {
            return ProfileShapeClass::Concave;
        }
    }
    ProfileShapeClass::Convex
}

/// Collects the distinct tags used by a profile, in first-use order.
#[must_use]
pub fn profile_tags(profile: &Profile2) -> Vec<SegTag> {
    let mut tags = Vec::new();
    let mut push = |seg: &Seg2| {
        if let Some(tag) = seg.tag
            && !tags.contains(&tag)
        {
            tags.push(tag);
        }
    };
    for seg in profile.outer().segs() {
        push(seg);
    }
    for hole in profile.holes() {
        for seg in hole.segs() {
            push(seg);
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_builder() {
        let r = rect(3.0, 2.0).expect("valid rect");
        assert!((profile_area(&r) - 6.0).abs() < 1e-12);
        assert_eq!(classify_profile(&r), ProfileShapeClass::Convex);
        let c = profile_centroid(&r).expect("has area");
        assert!((c.x - 1.5).abs() < 1e-9 && (c.y - 1.0).abs() < 1e-9);
        assert!(rect(0.0, 1.0).is_err());
        assert!(rect(1.0, f64::INFINITY).is_err());
    }

    #[test]
    fn rounded_rect_builder() {
        let r = rounded_rect(4.0, 2.0, 0.5).expect("valid rounded rect");
        // Area = full rect minus corner squares plus quarter circles.
        let expected = 4.0 * 2.0 - (4.0 - core::f64::consts::PI) * 0.25;
        assert!((profile_area(&r) - expected).abs() < 1e-6);
        assert_eq!(classify_profile(&r), ProfileShapeClass::Convex);
        assert!(
            rounded_rect(4.0, 2.0, 1.0).is_err(),
            "radius = h/2 rejected"
        );
    }

    #[test]
    fn l_profile_builder() {
        let l = l_profile(1.0, 1.0, 0.5, 0.5).expect("valid L");
        assert!((profile_area(&l) - 0.75).abs() < 1e-12);
        assert_eq!(classify_profile(&l), ProfileShapeClass::Concave);
        assert!(l_profile(1.0, 1.0, 1.0, 0.5).is_err());
    }

    #[test]
    fn circle_and_ring_builders() {
        let c = circle(2.0).expect("valid circle");
        assert!((profile_area(&c) - core::f64::consts::PI * 4.0).abs() < 1e-4);
        assert_eq!(classify_profile(&c), ProfileShapeClass::Convex);

        let r = ring(2.0, 1.0).expect("valid ring");
        assert!((profile_area(&r) - core::f64::consts::PI * 3.0).abs() < 1e-4);
        assert_eq!(classify_profile(&r), ProfileShapeClass::Concave);
        assert!(ring(1.0, 1.0).is_err());
    }

    #[test]
    fn builder_tags_enumerate() {
        let r = rounded_rect(4.0, 2.0, 0.5).expect("valid");
        let tags = profile_tags(&r);
        assert_eq!(tags.len(), 8);
        assert_eq!(tags[0], SegTag(0));
    }
}
