// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic UV sphere primitive.

use alloc::vec;
use alloc::vec::Vec;

use exedra::MeshBuilder;

use crate::{Primitive, RegionId, SelectionName, common};

/// Region ID for non-pole faces.
pub const REGION_BODY: RegionId = RegionId(1);
/// Region ID for top pole cap faces.
pub const REGION_POLE_TOP: RegionId = RegionId(2);
/// Region ID for bottom pole cap faces.
pub const REGION_POLE_BOTTOM: RegionId = RegionId(3);

/// Parameters for [`uv_sphere`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvSphereParams {
    /// Sphere radius.
    pub radius: f32,
    /// Number of latitude rings between poles.
    pub lat_segments: u32,
    /// Number of longitude segments around each ring.
    pub lon_segments: u32,
    /// When true, center sphere at origin; otherwise place bottom at `y = 0`.
    pub centered: bool,
}

impl Default for UvSphereParams {
    fn default() -> Self {
        Self {
            radius: 1.0,
            lat_segments: 8,
            lon_segments: 16,
            centered: true,
        }
    }
}

/// Builds a deterministic UV sphere primitive.
///
/// Poles are triangle fans; intermediate bands are quads.
///
/// Selection semantics:
/// - `edges.seam`: one edge per latitude step along fixed longitude `0`,
///   ordered from top pole to bottom pole.
///
/// # Panics
///
/// Panics when `lat_segments < 1` or `lon_segments < 3`.
#[must_use]
pub fn uv_sphere(params: &UvSphereParams) -> Primitive {
    assert!(
        params.lat_segments >= 1,
        "uv_sphere requires at least one latitude ring"
    );
    assert!(
        params.lon_segments >= 3,
        "uv_sphere requires at least three longitude segments"
    );

    let lat_segments = params.lat_segments as usize;
    let lon_segments = params.lon_segments as usize;
    let mut builder = MeshBuilder::new();
    let y_offset = if params.centered { 0.0 } else { params.radius };

    let _ = builder.push_vertex([0.0, params.radius + y_offset, 0.0]);
    for lat in 1..=lat_segments {
        let phi = (lat as f32) * core::f32::consts::PI / ((lat_segments + 1) as f32);
        let (sin_phi, cos_phi) = common::sin_cos(phi);
        let ring_radius = params.radius * sin_phi;
        let y = params.radius * cos_phi + y_offset;
        for lon in 0..lon_segments {
            let theta = (lon as f32) * core::f32::consts::TAU / (lon_segments as f32);
            let (sin_theta, cos_theta) = common::sin_cos(theta);
            let x = ring_radius * cos_theta;
            let z = ring_radius * sin_theta;
            let _ = builder.push_vertex([x, y, z]);
        }
    }
    let bottom_index = 1 + lat_segments * lon_segments;
    let _ = builder.push_vertex([0.0, -params.radius + y_offset, 0.0]);

    let ring_start = |ring: usize| 1 + ring * lon_segments;
    for lon in 0..lon_segments {
        let current = ring_start(0) + lon;
        let next = ring_start(0) + ((lon + 1) % lon_segments);
        builder
            .add_face(&[0, common::usize_to_u32(next), common::usize_to_u32(current)])
            .expect("top cap triangle should be valid");
    }

    for band in 0..lat_segments.saturating_sub(1) {
        let upper = ring_start(band);
        let lower = ring_start(band + 1);
        for lon in 0..lon_segments {
            let next = (lon + 1) % lon_segments;
            builder
                .add_face(&[
                    common::usize_to_u32(upper + lon),
                    common::usize_to_u32(upper + next),
                    common::usize_to_u32(lower + next),
                    common::usize_to_u32(lower + lon),
                ])
                .expect("band quad should be valid");
        }
    }

    let last_ring = ring_start(lat_segments - 1);
    for lon in 0..lon_segments {
        let current = last_ring + lon;
        let next = last_ring + ((lon + 1) % lon_segments);
        builder
            .add_face(&[
                common::usize_to_u32(bottom_index),
                common::usize_to_u32(current),
                common::usize_to_u32(next),
            ])
            .expect("bottom cap triangle should be valid");
    }

    let build = builder.build().expect("uv sphere topology must build");
    let top_face_count = lon_segments;
    let band_face_count = lon_segments * lat_segments.saturating_sub(1);
    let bottom_face_start = top_face_count + band_face_count;
    // Face order follows emission order:
    // top cap triangles, then band quads (if any), then bottom cap triangles.

    let mut seam = Vec::with_capacity(lat_segments + 1);
    // Seam edges are taken from the last longitude cell in each emitted block,
    // matching the fixed seam at longitude 0.
    seam.push(build.face_edge_ids[lon_segments - 1][0]);
    for band in 0..lat_segments.saturating_sub(1) {
        let start = top_face_count + band * lon_segments;
        seam.push(build.face_edge_ids[start + (lon_segments - 1)][1]);
    }
    seam.push(build.face_edge_ids[bottom_face_start + (lon_segments - 1)][2]);

    let top_pole_faces = build.face_ids[..top_face_count].to_vec();
    let band_faces = build.face_ids[top_face_count..bottom_face_start].to_vec();
    let bottom_pole_faces = build.face_ids[bottom_face_start..].to_vec();

    let mut region_assignments = Vec::with_capacity(build.face_ids.len());
    for face in &top_pole_faces {
        region_assignments.push((*face, REGION_POLE_TOP));
    }
    for face in &band_faces {
        region_assignments.push((*face, REGION_BODY));
    }
    for face in &bottom_pole_faces {
        region_assignments.push((*face, REGION_POLE_BOTTOM));
    }
    let face_region = common::face_region_layer(&build.face_ids, RegionId(0), &region_assignments);

    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![
            (SelectionName("faces.all"), build.face_ids.clone()),
            (SelectionName("faces.body"), band_faces.clone()),
            (SelectionName("faces.pole_top"), top_pole_faces),
            (SelectionName("faces.pole_bottom"), bottom_pole_faces),
        ],
        vec![(SelectionName("edges.seam"), seam)],
    )
}

#[cfg(test)]
mod tests {
    use exedra::{ExtractParams, HalfEdgeId};

    use super::{REGION_BODY, REGION_POLE_BOTTOM, REGION_POLE_TOP, UvSphereParams, uv_sphere};

    fn incident_regions(
        primitive: &crate::Primitive,
        edge: HalfEdgeId,
    ) -> (crate::RegionId, crate::RegionId) {
        let face = primitive
            .mesh
            .face(edge)
            .expect("selection edge should be live");
        let twin = primitive
            .mesh
            .twin(edge)
            .expect("selection edge should have twin");
        let twin_face = primitive.mesh.face(twin).expect("twin edge should be live");
        (
            primitive.face_region.get(face),
            primitive.face_region.get(twin_face),
        )
    }

    #[test]
    fn uv_sphere_builds_expected_topology() {
        let primitive = uv_sphere(&UvSphereParams {
            radius: 1.0,
            lat_segments: 2,
            lon_segments: 8,
            centered: true,
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), 24);
        let top = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.pole_top")
            .expect("faces.pole_top should exist");
        let bottom = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.pole_bottom")
            .expect("faces.pole_bottom should exist");
        let seam = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.seam")
            .expect("edges.seam should exist");
        let body = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.body")
            .expect("faces.body should exist");
        assert_eq!(top.1.as_slice().len(), 8);
        assert_eq!(bottom.1.as_slice().len(), 8);
        assert_eq!(body.1.as_slice().len(), 8);
        assert_eq!(seam.1.as_slice().len(), 3);
    }

    #[test]
    fn uv_sphere_is_deterministic_across_runs() {
        let params = UvSphereParams {
            radius: 2.0,
            lat_segments: 3,
            lon_segments: 12,
            centered: false,
        };
        let a = uv_sphere(&params);
        let b = uv_sphere(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }

    #[test]
    fn uv_sphere_seam_selection_matches_documented_contract() {
        let primitive = uv_sphere(&UvSphereParams {
            lat_segments: 3,
            lon_segments: 8,
            ..UvSphereParams::default()
        });
        let seam = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.seam")
            .expect("edges.seam should exist");
        let seam_edges = seam.1.as_slice();
        assert_eq!(seam_edges.len(), 4);

        let (first_a, first_b) = incident_regions(&primitive, seam_edges[0]);
        assert!(first_a == REGION_POLE_TOP || first_b == REGION_POLE_TOP);

        for edge in &seam_edges[1..seam_edges.len() - 1] {
            let (a, b) = incident_regions(&primitive, *edge);
            assert_eq!(a, REGION_BODY);
            assert_eq!(b, REGION_BODY);
        }

        let (last_a, last_b) = incident_regions(&primitive, seam_edges[seam_edges.len() - 1]);
        assert!(last_a == REGION_POLE_BOTTOM || last_b == REGION_POLE_BOTTOM);
    }
}
