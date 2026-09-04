// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic segmented grid primitive.

use alloc::vec;
use alloc::vec::Vec;

use exedra_mesh::{FaceBuildAttrs, MeshBuilder};

use crate::{Primitive, RegionId, SelectionName, common, sort_and_dedup_edges};

/// Region ID used by [`grid`].
pub const REGION_GRID: RegionId = RegionId(1);

/// Parameters for [`grid`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GridParams {
    /// Plane size along X and Y.
    pub size: [f32; 2],
    /// Segment counts along X and Y.
    pub segments: [u32; 2],
    /// When true, center the grid at the origin.
    pub centered: bool,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            size: [1.0, 1.0],
            segments: [1, 1],
            centered: true,
        }
    }
}

/// Builds a deterministic segmented planar grid.
///
/// The output contains only quad faces (`segments.x * segments.y` total).
///
/// Selection semantics:
/// - `faces.all`: all grid quads.
/// - `edges.boundary`: canonical outer boundary edge ring.
///
/// # Panics
///
/// Panics when either segment count is zero.
#[must_use]
pub fn grid(params: &GridParams) -> Primitive {
    let [seg_x_u32, seg_y_u32] = params.segments;
    assert!(
        seg_x_u32 >= 1 && seg_y_u32 >= 1,
        "grid requires segments >= 1 on both axes"
    );

    let seg_x = seg_x_u32 as usize;
    let seg_y = seg_y_u32 as usize;
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

    let mut builder = MeshBuilder::new();
    for y in 0..=seg_y {
        let ty = y as f32 / seg_y as f32;
        let py = min_y + (max_y - min_y) * ty;
        for x in 0..=seg_x {
            let tx = x as f32 / seg_x as f32;
            let px = min_x + (max_x - min_x) * tx;
            let _ = builder.push_vertex([px, py, 0.0]);
        }
    }

    let mut boundary_edges = Vec::new();
    for y in 0..seg_y {
        for x in 0..seg_x {
            let i00 = common::usize_to_u32(y * (seg_x + 1) + x);
            let i10 = common::usize_to_u32(y * (seg_x + 1) + x + 1);
            let i11 = common::usize_to_u32((y + 1) * (seg_x + 1) + x + 1);
            let i01 = common::usize_to_u32((y + 1) * (seg_x + 1) + x);
            builder
                .add_face_with_attrs(
                    &[i00, i10, i11, i01],
                    &FaceBuildAttrs {
                        region: Some(REGION_GRID.0),
                        ..FaceBuildAttrs::default()
                    },
                )
                .expect("grid quad should be valid");
        }
    }

    let build = builder.build().expect("grid topology must build");
    let face_ids = build.face_ids.clone();
    let mut edge_cursor = 0_usize;
    for y in 0..seg_y {
        for x in 0..seg_x {
            let edges = &build.face_edge_ids[edge_cursor];
            edge_cursor += 1;
            if y == 0 {
                boundary_edges.push(edges[0]);
            }
            if x + 1 == seg_x {
                boundary_edges.push(edges[1]);
            }
            if y + 1 == seg_y {
                boundary_edges.push(edges[2]);
            }
            if x == 0 {
                boundary_edges.push(edges[3]);
            }
        }
    }
    let _ = sort_and_dedup_edges(&mut boundary_edges);

    let assignments = face_ids
        .iter()
        .map(|face| (*face, REGION_GRID))
        .collect::<Vec<_>>();
    let face_region = common::face_region_layer(&face_ids, RegionId(0), &assignments);

    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![(SelectionName("faces.all"), face_ids)],
        vec![(SelectionName("edges.boundary"), boundary_edges)],
    )
}

#[cfg(test)]
mod tests {
    use exedra_mesh::ExtractParams;

    use super::{GridParams, REGION_GRID, grid};

    #[test]
    fn grid_builds_expected_counts_and_boundary() {
        let primitive = grid(&GridParams {
            size: [2.0, 3.0],
            segments: [3, 2],
            centered: true,
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), 6);
        assert_eq!(primitive.mesh.vertices().count(), 12);

        let faces_all = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.all")
            .expect("faces.all selection should exist")
            .1
            .as_slice();
        assert_eq!(faces_all.len(), 6);
        for face in faces_all {
            assert_eq!(primitive.face_region.get(*face), REGION_GRID);
        }

        let boundary = primitive
            .selections
            .edge_sets
            .iter()
            .find(|(name, _)| name.0 == "edges.boundary")
            .expect("edges.boundary selection should exist")
            .1
            .as_slice();
        assert_eq!(boundary.len(), 10);
    }

    #[test]
    fn grid_is_deterministic_across_runs() {
        let params = GridParams {
            size: [3.0, 2.0],
            segments: [4, 3],
            centered: false,
        };
        let a = grid(&params);
        let b = grid(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }

    #[test]
    #[should_panic(expected = "grid requires segments >= 1 on both axes")]
    fn grid_rejects_zero_segments() {
        let _ = grid(&GridParams {
            segments: [0, 1],
            ..GridParams::default()
        });
    }
}
