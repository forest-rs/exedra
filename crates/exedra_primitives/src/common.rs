// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared helpers for deterministic primitive assembly.

use alloc::vec::Vec;

use exedra::{ChangeSetBuilder, FaceId, HalfEdgeId, Mesh, op};

use crate::{EdgeSet, FaceRegionLayer, FaceSet, Primitive, RegionId, SelectionName, Selections};

pub(crate) fn primitive_from_parts(
    mesh: Mesh,
    face_region: FaceRegionLayer,
    face_sets: Vec<(SelectionName, Vec<FaceId>)>,
    edge_sets: Vec<(SelectionName, Vec<HalfEdgeId>)>,
) -> Primitive {
    Primitive {
        mesh,
        face_region,
        selections: Selections {
            face_sets: face_sets
                .into_iter()
                .map(|(name, set)| (name, FaceSet::from_vec(set)))
                .collect(),
            edge_sets: edge_sets
                .into_iter()
                .map(|(name, set)| (name, EdgeSet::from_vec(set)))
                .collect(),
        },
    }
}

pub(crate) fn mark_edges_sharp(mesh: &mut Mesh, edges: &[HalfEdgeId]) {
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    for &edge in edges {
        let _ = op::set_edge_sharpness(&mut edit, edge, 1.0);
    }
    let _ = edit.finish();
}

pub(crate) fn face_region_layer(
    face_ids: &[FaceId],
    default: RegionId,
    assignments: &[(FaceId, RegionId)],
) -> FaceRegionLayer {
    let len = face_ids
        .iter()
        .map(|id| id.index() as usize)
        .max()
        .map_or(0, |index| index.saturating_add(1));
    let mut layer = FaceRegionLayer::new(default, len);
    for (face, region) in assignments {
        let index = face.index() as usize;
        if index < layer.values.len() {
            layer.values[index] = *region;
        }
    }
    layer
}

pub(crate) fn sin_cos(theta: f32) -> (f32, f32) {
    #[cfg(feature = "libm")]
    {
        (libm::sinf(theta), libm::cosf(theta))
    }
    #[cfg(all(not(feature = "libm"), feature = "std"))]
    {
        theta.sin_cos()
    }
}

pub(crate) fn sqrt(value: f32) -> f32 {
    #[cfg(feature = "libm")]
    {
        libm::sqrtf(value)
    }
    #[cfg(all(not(feature = "libm"), feature = "std"))]
    {
        value.sqrt()
    }
}

pub(crate) fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("index overflowed u32")
}

#[cfg(test)]
mod tests {
    use super::sin_cos;

    const MAX_UNIT_SAMPLE_ABS_ERROR: f32 = 2.0e-6;
    const MAX_UNIT_LENGTH_ERROR: f32 = 2.0e-6;
    const SAMPLE_SEGMENTS: &[u32] = &[3, 4, 5, 6, 7, 8, 12, 16, 24, 32, 48, 64, 96, 127];

    #[expect(
        clippy::cast_possible_truncation,
        reason = "test reference intentionally rounds f64 results to the f32 output contract"
    )]
    fn reference_sin_cos(theta: f32) -> (f32, f32) {
        let theta = f64::from(theta);
        (theta.sin() as f32, theta.cos() as f32)
    }

    #[test]
    fn sin_cos_matches_sampled_angle_error_policy() {
        for &segments in SAMPLE_SEGMENTS {
            for index in 0..segments {
                let theta = (index as f32) * core::f32::consts::TAU / (segments as f32);
                let (sin_theta, cos_theta) = sin_cos(theta);
                let (ref_sin, ref_cos) = reference_sin_cos(theta);

                assert!(
                    (sin_theta - ref_sin).abs() <= MAX_UNIT_SAMPLE_ABS_ERROR,
                    "sin error exceeded policy for segment count {segments}, index {index}"
                );
                assert!(
                    (cos_theta - ref_cos).abs() <= MAX_UNIT_SAMPLE_ABS_ERROR,
                    "cos error exceeded policy for segment count {segments}, index {index}"
                );
            }
        }
    }

    #[test]
    fn sin_cos_samples_stay_on_unit_circle() {
        for &segments in SAMPLE_SEGMENTS {
            for index in 0..segments {
                let theta = (index as f32) * core::f32::consts::TAU / (segments as f32);
                let (sin_theta, cos_theta) = sin_cos(theta);
                let length_squared = sin_theta.mul_add(sin_theta, cos_theta * cos_theta);

                assert!(
                    (length_squared - 1.0).abs() <= MAX_UNIT_LENGTH_ERROR,
                    "unit-circle error exceeded policy for segment count {segments}, index {index}"
                );
            }
        }
    }

    #[cfg(all(feature = "std", feature = "libm"))]
    #[test]
    fn explicit_libm_feature_wins_when_cargo_unifies_both_backends() {
        // Workspace consumers can independently enable std and libm on this
        // crate. An explicit libm request must remain deterministic after
        // Cargo unifies those features instead of silently falling back to
        // platform math.
        let theta = 1.234_567_f32;
        let (sin_theta, cos_theta) = sin_cos(theta);
        assert_eq!(sin_theta.to_bits(), libm::sinf(theta).to_bits());
        assert_eq!(cos_theta.to_bits(), libm::cosf(theta).to_bits());
        assert_eq!(super::sqrt(7.25).to_bits(), libm::sqrtf(7.25).to_bits());
    }
}
