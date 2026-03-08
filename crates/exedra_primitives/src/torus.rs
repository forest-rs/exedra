// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic torus primitive.

use alloc::vec;
use alloc::vec::Vec;

use exedra::MeshBuilder;

use crate::{Primitive, RegionId, SelectionName, common};

/// Region ID for torus body faces.
pub const REGION_BODY: RegionId = RegionId(1);

/// Parameters for [`torus`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TorusParams {
    /// Major radius from origin to ring center.
    pub major_radius: f32,
    /// Minor radius of the tube.
    pub minor_radius: f32,
    /// Segment count around major ring.
    pub major_segments: u32,
    /// Segment count around minor ring.
    pub minor_segments: u32,
}

impl Default for TorusParams {
    fn default() -> Self {
        Self {
            major_radius: 1.0,
            minor_radius: 0.3,
            major_segments: 24,
            minor_segments: 12,
        }
    }
}

/// Builds a deterministic torus primitive.
///
/// Selection semantics:
/// - `faces.all`: all torus quads.
/// - `edges.seam_major`: one loop crossing the major-ring wrap seam.
/// - `edges.seam_minor`: one loop crossing the minor-ring wrap seam.
///
/// # Panics
///
/// Panics when either segment count is below 3.
#[must_use]
pub fn torus(params: &TorusParams) -> Primitive {
    assert!(
        params.major_segments >= 3 && params.minor_segments >= 3,
        "torus requires major_segments >= 3 and minor_segments >= 3"
    );

    let major = params.major_segments as usize;
    let minor = params.minor_segments as usize;

    let mut builder = MeshBuilder::new();
    for i in 0..major {
        let u = (i as f32) * core::f32::consts::TAU / (major as f32);
        let (sin_u, cos_u) = common::sin_cos(u);
        for j in 0..minor {
            let v = (j as f32) * core::f32::consts::TAU / (minor as f32);
            let (sin_v, cos_v) = common::sin_cos(v);
            let ring_radius = params.major_radius + params.minor_radius * cos_v;
            let x = ring_radius * cos_u;
            let y = params.minor_radius * sin_v;
            let z = ring_radius * sin_u;
            let _ = builder.push_vertex([x, y, z]);
        }
    }

    let mut seam_major = Vec::with_capacity(minor);
    let mut seam_minor = Vec::with_capacity(major);

    for i in 0..major {
        let next_i = (i + 1) % major;
        for j in 0..minor {
            let next_j = (j + 1) % minor;
            let a = common::usize_to_u32(i * minor + j);
            let b = common::usize_to_u32(next_i * minor + j);
            let c = common::usize_to_u32(next_i * minor + next_j);
            let d = common::usize_to_u32(i * minor + next_j);
            builder
                .add_face(&[a, d, c, b])
                .expect("torus quad should be valid");
        }
    }

    let build = builder.build().expect("torus topology must build");
    let face_ids = build.face_ids.clone();
    for i in 0..major {
        for j in 0..minor {
            let face_index = i * minor + j;
            let edges = &build.face_edge_ids[face_index];
            if i + 1 == major {
                seam_major.push(edges[0]);
            }
            if j + 1 == minor {
                seam_minor.push(edges[1]);
            }
        }
    }

    let assignments = face_ids
        .iter()
        .map(|face| (*face, REGION_BODY))
        .collect::<Vec<_>>();
    let face_region = common::face_region_layer(&face_ids, RegionId(0), &assignments);

    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![(SelectionName("faces.all"), face_ids)],
        vec![
            (SelectionName("edges.seam_major"), seam_major),
            (SelectionName("edges.seam_minor"), seam_minor),
        ],
    )
}

#[cfg(test)]
mod tests {
    use exedra::ExtractParams;

    use super::{REGION_BODY, TorusParams, torus};

    #[test]
    fn torus_builds_expected_counts_and_regions() {
        let primitive = torus(&TorusParams {
            major_segments: 12,
            minor_segments: 8,
            ..TorusParams::default()
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), 96);
        assert_eq!(primitive.mesh.vertices().count(), 96);

        let faces_all = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.all")
            .expect("faces.all should exist")
            .1
            .as_slice();
        assert_eq!(faces_all.len(), 96);
        for face in faces_all {
            assert_eq!(primitive.face_region.get(*face), REGION_BODY);
        }
    }

    #[test]
    fn torus_seam_selection_sizes_match_segments() {
        let major = 10_u32;
        let minor = 6_u32;
        let primitive = torus(&TorusParams {
            major_segments: major,
            minor_segments: minor,
            ..TorusParams::default()
        });
        let seam_major = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.seam_major")
            .expect("edges.seam_major should exist")
            .1
            .as_slice();
        let seam_minor = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.seam_minor")
            .expect("edges.seam_minor should exist")
            .1
            .as_slice();
        assert_eq!(seam_major.len(), minor as usize);
        assert_eq!(seam_minor.len(), major as usize);
    }

    #[test]
    fn torus_is_deterministic() {
        let params = TorusParams {
            major_radius: 2.0,
            minor_radius: 0.7,
            major_segments: 18,
            minor_segments: 9,
        };
        let a = torus(&params);
        let b = torus(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }

    #[test]
    fn torus_default_extracted_normals_point_outward() {
        let primitive = torus(&TorusParams {
            major_radius: 1.0,
            minor_radius: 0.25,
            major_segments: 12,
            minor_segments: 8,
        });
        let (tri, _) = primitive.mesh.to_trimesh(&ExtractParams::default());
        let pos = tri.positions[0];
        let normal = tri.normals[0];
        let major_len = (pos[0] * pos[0] + pos[2] * pos[2]).sqrt();
        let center = [pos[0] / major_len, 0.0, pos[2] / major_len];
        let expected = [pos[0] - center[0], pos[1] - center[1], pos[2] - center[2]];
        let expected_len =
            (expected[0] * expected[0] + expected[1] * expected[1] + expected[2] * expected[2])
                .sqrt();
        let expected = [
            expected[0] / expected_len,
            expected[1] / expected_len,
            expected[2] / expected_len,
        ];
        let dot = normal[0] * expected[0] + normal[1] * expected[1] + normal[2] * expected[2];
        assert!(
            dot > 0.95,
            "torus normal should point outward: normal {:?}, expected {:?}, dot {}",
            normal,
            expected,
            dot
        );
    }

    #[test]
    #[should_panic(expected = "torus requires major_segments >= 3 and minor_segments >= 3")]
    fn torus_rejects_small_segments() {
        let _ = torus(&TorusParams {
            major_segments: 2,
            minor_segments: 3,
            ..TorusParams::default()
        });
    }
}
