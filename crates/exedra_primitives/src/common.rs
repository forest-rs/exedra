// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared helpers for deterministic primitive assembly.

use alloc::vec::Vec;

use exedra::{FaceId, HalfEdgeId, Mesh};

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
    #[cfg(feature = "std")]
    {
        theta.sin_cos()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        (libm::sinf(theta), libm::cosf(theta))
    }
}

pub(crate) fn sqrt(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.sqrt()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::sqrtf(value)
    }
}

pub(crate) fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("index overflowed u32")
}
