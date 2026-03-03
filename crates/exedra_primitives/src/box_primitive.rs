// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic box primitive.

use alloc::vec;
use alloc::vec::Vec;

use exedra::MeshBuilder;

use crate::{Primitive, RegionId, SelectionName, common};

/// Region ID for +X side faces.
pub const REGION_SIDE_X_POS: RegionId = RegionId(1);
/// Region ID for -X side faces.
pub const REGION_SIDE_X_NEG: RegionId = RegionId(2);
/// Region ID for +Y side faces.
pub const REGION_SIDE_Y_POS: RegionId = RegionId(3);
/// Region ID for -Y side faces.
pub const REGION_SIDE_Y_NEG: RegionId = RegionId(4);
/// Region ID for +Z side faces.
pub const REGION_SIDE_Z_POS: RegionId = RegionId(5);
/// Region ID for -Z side faces.
pub const REGION_SIDE_Z_NEG: RegionId = RegionId(6);

/// Parameters for [`box_primitive`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxParams {
    /// Box size along X, Y, and Z.
    pub size: [f32; 3],
    /// When true, center the box at the origin.
    pub centered: bool,
    /// Subdivision counts along X, Y, Z.
    ///
    /// v0.1 supports only `[1, 1, 1]`.
    pub segments: [u32; 3],
}

impl Default for BoxParams {
    fn default() -> Self {
        Self {
            size: [1.0, 1.0, 1.0],
            centered: true,
            segments: [1, 1, 1],
        }
    }
}

/// Builds a deterministic six-face box primitive.
///
/// Face emission order is fixed: `+X, -X, +Y, -Y, +Z, -Z`.
///
/// # Panics
///
/// Panics when `params.segments != [1, 1, 1]` (v0.1 limitation).
#[must_use]
pub fn box_primitive(params: &BoxParams) -> Primitive {
    assert_eq!(
        params.segments,
        [1, 1, 1],
        "box_primitive currently supports only segments = [1, 1, 1]"
    );

    let [sx, sy, sz] = params.size;
    let (min_x, max_x) = extent(sx, params.centered);
    let (min_y, max_y) = extent(sy, params.centered);
    let (min_z, max_z) = extent(sz, params.centered);

    let mut builder = MeshBuilder::new();
    // 0..=7 fixed vertex numbering.
    let _ = builder.push_vertex([min_x, min_y, min_z]);
    let _ = builder.push_vertex([max_x, min_y, min_z]);
    let _ = builder.push_vertex([max_x, max_y, min_z]);
    let _ = builder.push_vertex([min_x, max_y, min_z]);
    let _ = builder.push_vertex([min_x, min_y, max_z]);
    let _ = builder.push_vertex([max_x, min_y, max_z]);
    let _ = builder.push_vertex([max_x, max_y, max_z]);
    let _ = builder.push_vertex([min_x, max_y, max_z]);

    // Face order: +X, -X, +Y, -Y, +Z, -Z.
    builder.add_face(&[1, 5, 6, 2]).expect("+X");
    builder.add_face(&[4, 0, 3, 7]).expect("-X");
    builder.add_face(&[3, 2, 6, 7]).expect("+Y");
    builder.add_face(&[0, 4, 5, 1]).expect("-Y");
    builder.add_face(&[4, 7, 6, 5]).expect("+Z");
    builder.add_face(&[0, 1, 2, 3]).expect("-Z");

    let build = builder.build().expect("box topology must build");
    let face_ids = &build.face_ids;
    let face_region = common::face_region_layer(
        face_ids,
        RegionId(0),
        &[
            (face_ids[0], REGION_SIDE_X_POS),
            (face_ids[1], REGION_SIDE_X_NEG),
            (face_ids[2], REGION_SIDE_Y_POS),
            (face_ids[3], REGION_SIDE_Y_NEG),
            (face_ids[4], REGION_SIDE_Z_POS),
            (face_ids[5], REGION_SIDE_Z_NEG),
        ],
    );

    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![
            (SelectionName("faces.all"), face_ids.clone()),
            (SelectionName("faces.side_x_pos"), vec![face_ids[0]]),
            (SelectionName("faces.side_x_neg"), vec![face_ids[1]]),
            (SelectionName("faces.side_y_pos"), vec![face_ids[2]]),
            (SelectionName("faces.side_y_neg"), vec![face_ids[3]]),
            (SelectionName("faces.side_z_pos"), vec![face_ids[4]]),
            (SelectionName("faces.side_z_neg"), vec![face_ids[5]]),
        ],
        Vec::new(),
    )
}

fn extent(size: f32, centered: bool) -> (f32, f32) {
    if centered {
        (-size * 0.5, size * 0.5)
    } else {
        (0.0, size)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra::ExtractParams;

    use super::{
        BoxParams, REGION_SIDE_X_NEG, REGION_SIDE_X_POS, REGION_SIDE_Y_NEG, REGION_SIDE_Y_POS,
        REGION_SIDE_Z_NEG, REGION_SIDE_Z_POS, box_primitive,
    };

    #[test]
    fn box_primitive_builds_six_face_mesh() {
        let primitive = box_primitive(&BoxParams::default());
        assert_eq!(primitive.mesh.faces().count(), 6);
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());

        let faces = primitive.mesh.faces().collect::<Vec<_>>();
        assert_eq!(primitive.face_region.get(faces[0]), REGION_SIDE_X_POS);
        assert_eq!(primitive.face_region.get(faces[1]), REGION_SIDE_X_NEG);
        assert_eq!(primitive.face_region.get(faces[2]), REGION_SIDE_Y_POS);
        assert_eq!(primitive.face_region.get(faces[3]), REGION_SIDE_Y_NEG);
        assert_eq!(primitive.face_region.get(faces[4]), REGION_SIDE_Z_POS);
        assert_eq!(primitive.face_region.get(faces[5]), REGION_SIDE_Z_NEG);
    }

    #[test]
    fn box_primitive_is_deterministic_across_runs() {
        let params = BoxParams {
            size: [2.0, 3.0, 4.0],
            centered: false,
            segments: [1, 1, 1],
        };
        let a = box_primitive(&params);
        let b = box_primitive(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }
}
