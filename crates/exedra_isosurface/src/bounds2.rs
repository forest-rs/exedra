// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simple 2D axis-aligned bounds for profile fields.

/// Two-dimensional axis-aligned bounding box.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb2 {
    /// Inclusive minimum corner.
    pub min: [f32; 2],
    /// Inclusive maximum corner.
    pub max: [f32; 2],
}

impl Aabb2 {
    /// Creates a valid 2D bounding box.
    #[must_use]
    pub fn new(min: [f32; 2], max: [f32; 2]) -> Option<Self> {
        if !min
            .iter()
            .chain(max.iter())
            .all(|component| component.is_finite())
        {
            return None;
        }
        if min[0] > max[0] || min[1] > max[1] {
            return None;
        }
        Some(Self { min, max })
    }

    /// Returns the center of the box.
    #[must_use]
    pub fn center(&self) -> [f32; 2] {
        [
            self.min[0].midpoint(self.max[0]),
            self.min[1].midpoint(self.max[1]),
        ]
    }

    /// Returns the four corners in deterministic order.
    #[must_use]
    pub fn corners(&self) -> [[f32; 2]; 4] {
        [
            [self.min[0], self.min[1]],
            [self.max[0], self.min[1]],
            [self.min[0], self.max[1]],
            [self.max[0], self.max[1]],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::Aabb2;

    #[test]
    fn rejects_invalid_boxes() {
        assert!(Aabb2::new([1.0, 0.0], [0.0, 1.0]).is_none());
        assert!(Aabb2::new([0.0, 0.0], [f32::INFINITY, 1.0]).is_none());
    }

    #[test]
    fn returns_center_and_corners() {
        let bounds = Aabb2::new([-2.0, -1.0], [4.0, 3.0]).expect("valid bounds");

        assert_eq!(bounds.center(), [1.0, 1.0]);
        assert_eq!(
            bounds.corners(),
            [[-2.0, -1.0], [4.0, -1.0], [-2.0, 3.0], [4.0, 3.0]]
        );

        // `f32::midpoint` must not overflow for finite same-sign extremes.
        let wide = Aabb2::new([f32::MAX / 2.0; 2], [f32::MAX; 2]).expect("finite ordered bounds");
        assert!(wide.center().iter().all(|component| component.is_finite()));
    }
}
