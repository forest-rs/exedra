// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic quad/plane primitive.

use alloc::vec;

use exedra::{FaceBuildAttrs, MeshBuilder};

use crate::{Primitive, RegionId, SelectionName, common};

/// Region ID used by [`quad`].
pub const REGION_FACE: RegionId = RegionId(1);

/// Parameters for [`quad`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct QuadParams {
    /// Plane size along X and Y.
    pub size: [f32; 2],
    /// When true, center the quad at the origin.
    pub centered: bool,
}

impl Default for QuadParams {
    fn default() -> Self {
        Self {
            size: [1.0, 1.0],
            centered: true,
        }
    }
}

/// Builds a single-face quad primitive.
///
/// The output mesh contains one ngon face (degree 4) with deterministic vertex
/// and edge ordering.
#[must_use]
pub fn quad(params: &QuadParams) -> Primitive {
    let mut builder = MeshBuilder::new();
    let [sx, sy] = params.size;
    let (min_x, max_x) = if params.centered {
        (-sx * 0.5, sx * 0.5)
    } else {
        (0.0, sx)
    };
    let (min_y, max_y) = if params.centered {
        (-sy * 0.5, sy * 0.5)
    } else {
        (0.0, sy)
    };

    let _ = builder.push_vertex([min_x, min_y, 0.0]);
    let _ = builder.push_vertex([max_x, min_y, 0.0]);
    let _ = builder.push_vertex([max_x, max_y, 0.0]);
    let _ = builder.push_vertex([min_x, max_y, 0.0]);
    builder
        .add_face_with_attrs(
            &[0, 1, 2, 3],
            &FaceBuildAttrs {
                region: Some(REGION_FACE.0),
                ..FaceBuildAttrs::default()
            },
        )
        .expect("quad loop must be valid");
    let build = builder.build().expect("quad topology must build");
    let face = build.face_ids[0];
    let boundary = build.face_edge_ids[0].clone();
    let face_region =
        common::face_region_layer(&build.face_ids, RegionId(0), &[(face, REGION_FACE)]);
    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![(SelectionName("faces.all"), vec![face])],
        vec![(SelectionName("edges.boundary"), boundary)],
    )
}

#[cfg(test)]
mod tests {
    use exedra::{ExtractParams, attr};

    use super::{QuadParams, REGION_FACE, quad};

    #[test]
    fn quad_returns_single_face_primitive() {
        let primitive = quad(&QuadParams::default());
        assert_eq!(primitive.mesh.faces().count(), 1);
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        let face = primitive.mesh.faces().next().expect("face should exist");
        assert_eq!(primitive.mesh.face_loop(face).count(), 4);
        assert_eq!(primitive.face_region.get(face), REGION_FACE);
        assert_eq!(
            primitive
                .mesh
                .attrs()
                .dense(attr::FACE_REGION)
                .expect("FACE_REGION should exist")
                .get(face.as_id()),
            Some(&REGION_FACE.0)
        );
        let faces_all = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.all")
            .expect("faces.all selection should exist");
        assert_eq!(faces_all.1.as_slice(), &[face]);
        let boundary = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.boundary")
            .expect("edges.boundary selection should exist");
        assert_eq!(boundary.1.as_slice().len(), 4);
    }

    #[test]
    fn quad_is_deterministic_across_runs() {
        let params = QuadParams {
            size: [2.0, 3.0],
            centered: false,
        };
        let a = quad(&params);
        let b = quad(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }
}
