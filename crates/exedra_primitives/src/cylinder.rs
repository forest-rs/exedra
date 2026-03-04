// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic cylinder primitive.

use alloc::vec;
use alloc::vec::Vec;

use exedra::MeshBuilder;

use crate::{Primitive, RegionId, SelectionName, common};

/// Region ID for side faces.
pub const REGION_SIDE: RegionId = RegionId(1);
/// Region ID for top cap faces.
pub const REGION_CAP_TOP: RegionId = RegionId(2);
/// Region ID for bottom cap faces.
pub const REGION_CAP_BOTTOM: RegionId = RegionId(3);

/// Parameters for [`cylinder`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CylinderParams {
    /// Cylinder radius.
    pub radius: f32,
    /// Cylinder height.
    pub height: f32,
    /// Number of angular segments.
    pub segments: u32,
    /// When true, emit top and bottom cap ngons.
    pub capped: bool,
    /// When true, center the cylinder at the origin along Y.
    pub centered: bool,
}

impl Default for CylinderParams {
    fn default() -> Self {
        Self {
            radius: 1.0,
            height: 1.0,
            segments: 16,
            capped: true,
            centered: true,
        }
    }
}

/// Builds a deterministic cylinder primitive.
///
/// Vertices are emitted in increasing angular order; seam reference is fixed at
/// angle `0`.
///
/// Selection semantics:
/// - `edges.seam`: single side edge on the fixed longitude seam (`theta = 0`).
/// - `edges.rim_top`: one edge per segment on the top side/cap interface
///   (or side/OUTSIDE boundary when uncapped).
/// - `edges.rim_bottom`: one edge per segment on the bottom side/cap interface
///   (or side/OUTSIDE boundary when uncapped).
///
/// # Panics
///
/// Panics when `params.segments < 3`.
#[must_use]
pub fn cylinder(params: &CylinderParams) -> Primitive {
    assert!(
        params.segments >= 3,
        "cylinder requires at least 3 segments"
    );

    let segments = params.segments as usize;
    let (bottom_y, top_y) = if params.centered {
        (-params.height * 0.5, params.height * 0.5)
    } else {
        (0.0, params.height)
    };

    let mut builder = MeshBuilder::new();
    push_ring(&mut builder, params.radius, bottom_y, segments);
    push_ring(&mut builder, params.radius, top_y, segments);

    for i in 0..segments {
        let next = (i + 1) % segments;
        builder
            .add_face(&[
                common::usize_to_u32(i),
                common::usize_to_u32(next),
                common::usize_to_u32(segments + next),
                common::usize_to_u32(segments + i),
            ])
            .expect("side quad should be valid");
    }

    if params.capped {
        let mut top = Vec::with_capacity(segments);
        for i in 0..segments {
            top.push(common::usize_to_u32(segments + i));
        }
        builder.add_face(&top).expect("top cap should be valid");

        let mut bottom = Vec::with_capacity(segments);
        for i in (0..segments).rev() {
            bottom.push(common::usize_to_u32(i));
        }
        builder
            .add_face(&bottom)
            .expect("bottom cap should be valid");
    }

    let build = builder.build().expect("cylinder topology must build");
    let face_ids = &build.face_ids;
    // Face order is coupled to the emission loops above:
    // 0..segments are side faces, followed by optional top and bottom caps.
    let side_faces = face_ids[..segments].to_vec();
    let top_face = if params.capped {
        Some(face_ids[segments])
    } else {
        None
    };
    let bottom_face = if params.capped {
        Some(face_ids[segments + 1])
    } else {
        None
    };

    let seam_edges = vec![build.face_edge_ids[segments - 1][1]];

    let mut rim_top = Vec::with_capacity(segments);
    let mut rim_bottom = Vec::with_capacity(segments);
    // edge[0] is bottom rim and edge[2] is top rim for each side quad emitted
    // as [bottom_i, bottom_next, top_next, top_i].
    for i in 0..segments {
        rim_bottom.push(build.face_edge_ids[i][0]);
        rim_top.push(build.face_edge_ids[i][2]);
    }

    let mut assignments = Vec::with_capacity(segments + 2);
    for face in &side_faces {
        assignments.push((*face, REGION_SIDE));
    }
    if let Some(face) = top_face {
        assignments.push((face, REGION_CAP_TOP));
    }
    if let Some(face) = bottom_face {
        assignments.push((face, REGION_CAP_BOTTOM));
    }
    let face_region = common::face_region_layer(face_ids, RegionId(0), &assignments);

    let face_sets = vec![
        (SelectionName("faces.side"), side_faces),
        (
            SelectionName("faces.cap_top"),
            top_face.into_iter().collect(),
        ),
        (
            SelectionName("faces.cap_bottom"),
            bottom_face.into_iter().collect(),
        ),
    ];

    common::primitive_from_parts(
        build.mesh,
        face_region,
        face_sets,
        vec![
            (SelectionName("edges.seam"), seam_edges),
            (SelectionName("edges.rim_top"), rim_top),
            (SelectionName("edges.rim_bottom"), rim_bottom),
        ],
    )
}

fn push_ring(builder: &mut MeshBuilder, radius: f32, y: f32, segments: usize) {
    for i in 0..segments {
        let theta = (i as f32) * core::f32::consts::TAU / (segments as f32);
        let (sin_theta, cos_theta) = common::sin_cos(theta);
        let x = radius * cos_theta;
        let z = radius * sin_theta;
        let _ = builder.push_vertex([x, y, z]);
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra::{ExtractParams, FaceId, HalfEdgeId};

    use super::{CylinderParams, REGION_CAP_BOTTOM, REGION_CAP_TOP, REGION_SIDE, cylinder};

    fn incident_faces(primitive: &crate::Primitive, edge: HalfEdgeId) -> (FaceId, FaceId) {
        let face = primitive
            .mesh
            .face(edge)
            .expect("selection edge should be live");
        let twin = primitive
            .mesh
            .twin(edge)
            .expect("selection edge should have twin");
        let twin_face = primitive.mesh.face(twin).expect("twin edge should be live");
        (face, twin_face)
    }

    #[test]
    fn capped_cylinder_builds_expected_regions() {
        let primitive = cylinder(&CylinderParams {
            radius: 1.0,
            height: 2.0,
            segments: 8,
            capped: true,
            centered: true,
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), 10);
        let faces = primitive.mesh.faces().collect::<Vec<_>>();
        for face in faces.iter().take(8) {
            assert_eq!(primitive.face_region.get(*face), REGION_SIDE);
        }
        assert_eq!(primitive.face_region.get(faces[8]), REGION_CAP_TOP);
        assert_eq!(primitive.face_region.get(faces[9]), REGION_CAP_BOTTOM);
    }

    #[test]
    fn uncapped_cylinder_has_no_cap_faces() {
        let primitive = cylinder(&CylinderParams {
            capped: false,
            segments: 6,
            ..CylinderParams::default()
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), 6);
        let top = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.cap_top")
            .expect("faces.cap_top should exist");
        let bottom = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.cap_bottom")
            .expect("faces.cap_bottom should exist");
        assert!(top.1.as_slice().is_empty());
        assert!(bottom.1.as_slice().is_empty());
    }

    #[test]
    fn cylinder_is_deterministic() {
        let params = CylinderParams {
            radius: 1.5,
            height: 3.0,
            segments: 12,
            capped: true,
            centered: false,
        };
        let a = cylinder(&params);
        let b = cylinder(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }

    #[test]
    fn capped_cylinder_rim_and_seam_sets_match_contract() {
        let primitive = cylinder(&CylinderParams {
            segments: 8,
            capped: true,
            ..CylinderParams::default()
        });
        let seam = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.seam")
            .expect("edges.seam should exist");
        let rim_top = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.rim_top")
            .expect("edges.rim_top should exist");
        let rim_bottom = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.rim_bottom")
            .expect("edges.rim_bottom should exist");

        assert_eq!(seam.1.as_slice().len(), 1);
        assert_eq!(rim_top.1.as_slice().len(), 8);
        assert_eq!(rim_bottom.1.as_slice().len(), 8);

        let (seam_face, seam_twin_face) = incident_faces(&primitive, seam.1.as_slice()[0]);
        assert_eq!(primitive.face_region.get(seam_face), REGION_SIDE);
        assert_eq!(primitive.face_region.get(seam_twin_face), REGION_SIDE);

        for edge in rim_top.1.as_slice() {
            let (face, twin_face) = incident_faces(&primitive, *edge);
            let a = primitive.face_region.get(face);
            let b = primitive.face_region.get(twin_face);
            assert!(
                (a == REGION_SIDE && b == REGION_CAP_TOP)
                    || (a == REGION_CAP_TOP && b == REGION_SIDE)
            );
        }
        for edge in rim_bottom.1.as_slice() {
            let (face, twin_face) = incident_faces(&primitive, *edge);
            let a = primitive.face_region.get(face);
            let b = primitive.face_region.get(twin_face);
            assert!(
                (a == REGION_SIDE && b == REGION_CAP_BOTTOM)
                    || (a == REGION_CAP_BOTTOM && b == REGION_SIDE)
            );
        }
    }

    #[test]
    fn uncapped_cylinder_rim_sets_are_side_to_outside_boundaries() {
        let primitive = cylinder(&CylinderParams {
            segments: 6,
            capped: false,
            ..CylinderParams::default()
        });
        let rim_top = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.rim_top")
            .expect("edges.rim_top should exist");
        let rim_bottom = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.rim_bottom")
            .expect("edges.rim_bottom should exist");

        for edge in rim_top
            .1
            .as_slice()
            .iter()
            .chain(rim_bottom.1.as_slice().iter())
        {
            let (face, twin_face) = incident_faces(&primitive, *edge);
            let side = if face == FaceId::OUTSIDE {
                twin_face
            } else {
                face
            };
            let outside = if face == FaceId::OUTSIDE {
                face
            } else {
                twin_face
            };
            assert_eq!(outside, FaceId::OUTSIDE);
            assert_eq!(primitive.face_region.get(side), REGION_SIDE);
        }
    }
}
