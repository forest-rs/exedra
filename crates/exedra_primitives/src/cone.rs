// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic cone primitive.

use alloc::vec;
use alloc::vec::Vec;

use exedra::MeshBuilder;

use crate::{CapFill, Primitive, RegionId, SelectionName, common};

/// Region ID for side faces.
pub const REGION_SIDE: RegionId = RegionId(1);
/// Region ID for bottom cap faces.
pub const REGION_CAP_BOTTOM: RegionId = RegionId(2);

/// Parameters for [`cone`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ConeParams {
    /// Base radius.
    pub radius: f32,
    /// Height from base plane to apex.
    pub height: f32,
    /// Number of radial segments.
    pub segments: u32,
    /// Bottom cap fill policy.
    pub cap_fill: CapFill,
    /// When true, center along Y.
    pub centered: bool,
}

impl Default for ConeParams {
    fn default() -> Self {
        Self {
            radius: 1.0,
            height: 1.0,
            segments: 16,
            cap_fill: CapFill::Ngon,
            centered: true,
        }
    }
}

/// Builds a deterministic cone primitive.
///
/// Selection semantics:
/// - `faces.side`: side triangles.
/// - `faces.cap_bottom`: bottom cap face(s), empty when `cap_fill = None`.
/// - `edges.seam`: one deterministic apex side edge along the fixed seam.
/// - `edges.rim_bottom`: side/cap (or side/OUTSIDE) bottom rim edges.
///
/// # Panics
///
/// Panics when `params.segments < 3`.
#[must_use]
pub fn cone(params: &ConeParams) -> Primitive {
    assert!(params.segments >= 3, "cone requires at least 3 segments");

    let segments = params.segments as usize;
    let (bottom_y, top_y) = if params.centered {
        (-params.height * 0.5, params.height * 0.5)
    } else {
        (0.0, params.height)
    };

    let mut builder = MeshBuilder::new();
    for i in 0..segments {
        let theta = (i as f32) * core::f32::consts::TAU / (segments as f32);
        let (sin_theta, cos_theta) = common::sin_cos(theta);
        let x = params.radius * cos_theta;
        let z = params.radius * sin_theta;
        let _ = builder.push_vertex([x, bottom_y, z]);
    }
    let apex = builder.push_vertex([0.0, top_y, 0.0]);

    for i in 0..segments {
        let next = (i + 1) % segments;
        builder
            .add_face(&[common::usize_to_u32(i), apex, common::usize_to_u32(next)])
            .expect("side triangle should be valid");
    }

    match params.cap_fill {
        CapFill::None => {}
        CapFill::Ngon => {
            let mut bottom = Vec::with_capacity(segments);
            for i in 0..segments {
                bottom.push(common::usize_to_u32(i));
            }
            builder
                .add_face(&bottom)
                .expect("bottom cap ngon should be valid");
        }
        CapFill::TriangleFan => {
            let center = builder.push_vertex([0.0, bottom_y, 0.0]);
            for i in 0..segments {
                let next = (i + 1) % segments;
                builder
                    .add_face(&[center, common::usize_to_u32(i), common::usize_to_u32(next)])
                    .expect("bottom cap fan triangle should be valid");
            }
        }
    }

    let build = builder.build().expect("cone topology must build");
    let face_ids = &build.face_ids;
    let side_faces = face_ids[..segments].to_vec();
    let bottom_faces = match params.cap_fill {
        CapFill::None => Vec::new(),
        CapFill::Ngon => vec![face_ids[segments]],
        CapFill::TriangleFan => face_ids[segments..(segments * 2)].to_vec(),
    };

    let seam_edge = vec![build.face_edge_ids[segments - 1][1]];
    let mut rim_bottom = Vec::with_capacity(segments);
    for i in 0..segments {
        rim_bottom.push(build.face_edge_ids[i][2]);
    }

    let mut assignments = Vec::with_capacity(side_faces.len() + bottom_faces.len());
    for face in &side_faces {
        assignments.push((*face, REGION_SIDE));
    }
    for face in &bottom_faces {
        assignments.push((*face, REGION_CAP_BOTTOM));
    }
    let face_region = common::face_region_layer(face_ids, RegionId(0), &assignments);

    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![
            (SelectionName("faces.side"), side_faces),
            (SelectionName("faces.cap_bottom"), bottom_faces),
        ],
        vec![
            (SelectionName("edges.seam"), seam_edge),
            (SelectionName("edges.rim_bottom"), rim_bottom),
        ],
    )
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra::{ExtractParams, FaceId};

    use super::{CapFill, ConeParams, REGION_CAP_BOTTOM, REGION_SIDE, cone};

    #[test]
    fn cone_ngon_cap_builds_expected_regions() {
        let primitive = cone(&ConeParams {
            segments: 8,
            cap_fill: CapFill::Ngon,
            ..ConeParams::default()
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), 9);
        let faces = primitive.mesh.faces().collect::<Vec<_>>();
        for face in faces.iter().take(8) {
            assert_eq!(primitive.face_region.get(*face), REGION_SIDE);
        }
        assert_eq!(primitive.face_region.get(faces[8]), REGION_CAP_BOTTOM);
    }

    #[test]
    fn cone_triangle_fan_cap_face_count_matches_segments() {
        let segments = 10_u32;
        let primitive = cone(&ConeParams {
            segments,
            cap_fill: CapFill::TriangleFan,
            ..ConeParams::default()
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), (segments as usize) * 2);
        let bottom = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.cap_bottom")
            .expect("faces.cap_bottom should exist")
            .1
            .as_slice();
        assert_eq!(bottom.len(), segments as usize);
        for face in bottom {
            assert_eq!(primitive.face_region.get(*face), REGION_CAP_BOTTOM);
        }
    }

    #[test]
    fn uncapped_cone_has_rim_edges_against_outside() {
        let primitive = cone(&ConeParams {
            segments: 7,
            cap_fill: CapFill::None,
            ..ConeParams::default()
        });
        let rim = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.rim_bottom")
            .expect("edges.rim_bottom should exist")
            .1
            .as_slice();
        assert_eq!(rim.len(), 7);
        for &edge in rim {
            let face = primitive
                .mesh
                .face(edge)
                .expect("rim edge face should be live");
            let twin = primitive
                .mesh
                .twin(edge)
                .expect("rim edge twin should be live");
            let twin_face = primitive
                .mesh
                .face(twin)
                .expect("rim edge twin face should be live");
            let (side_face, outside_face) = if face == FaceId::OUTSIDE {
                (twin_face, face)
            } else {
                (face, twin_face)
            };
            assert_eq!(outside_face, FaceId::OUTSIDE);
            assert_eq!(primitive.face_region.get(side_face), REGION_SIDE);
        }
    }

    #[test]
    fn cone_is_deterministic() {
        let params = ConeParams {
            radius: 1.2,
            height: 2.3,
            segments: 12,
            cap_fill: CapFill::TriangleFan,
            centered: false,
        };
        let a = cone(&params);
        let b = cone(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }
}
