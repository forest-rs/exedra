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
/// Panics when any `params.segments` component is `0`.
#[must_use]
pub fn box_primitive(params: &BoxParams) -> Primitive {
    assert!(
        params.segments[0] >= 1 && params.segments[1] >= 1 && params.segments[2] >= 1,
        "box_primitive requires segments >= 1 on all axes"
    );

    let [sx, sy, sz] = params.size;
    let [seg_x, seg_y, seg_z] = params.segments;
    let seg_x = seg_x as usize;
    let seg_y = seg_y as usize;
    let seg_z = seg_z as usize;
    let (min_x, max_x) = extent(sx, params.centered);
    let (min_y, max_y) = extent(sy, params.centered);
    let (min_z, max_z) = extent(sz, params.centered);

    let mut builder = MeshBuilder::new();
    let x_count = seg_x + 1;
    let y_count = seg_y + 1;
    let z_count = seg_z + 1;
    let slot_count = x_count * y_count * z_count;
    let mut slots = vec![None; slot_count];
    let idx = |x: usize, y: usize, z: usize| -> usize { (z * y_count + y) * x_count + x };
    let lerp = |min: f32, max: f32, i: usize, n: usize| -> f32 {
        if n == 0 {
            min
        } else {
            min + (max - min) * (i as f32) / (n as f32)
        }
    };
    let mut vertex = |x: usize, y: usize, z: usize, builder: &mut MeshBuilder| -> u32 {
        let slot = idx(x, y, z);
        if let Some(index) = slots[slot] {
            return index;
        }
        let vx = lerp(min_x, max_x, x, seg_x);
        let vy = lerp(min_y, max_y, y, seg_y);
        let vz = lerp(min_z, max_z, z, seg_z);
        let index = builder.push_vertex([vx, vy, vz]);
        slots[slot] = Some(index);
        index
    };

    // Face order: +X, -X, +Y, -Y, +Z, -Z.
    for y in 0..seg_y {
        for z in 0..seg_z {
            let face = [
                vertex(seg_x, y, z, &mut builder),
                vertex(seg_x, y, z + 1, &mut builder),
                vertex(seg_x, y + 1, z + 1, &mut builder),
                vertex(seg_x, y + 1, z, &mut builder),
            ];
            builder.add_face(&face).expect("+X");
        }
    }
    for y in 0..seg_y {
        for z in 0..seg_z {
            let face = [
                vertex(0, y, z + 1, &mut builder),
                vertex(0, y, z, &mut builder),
                vertex(0, y + 1, z, &mut builder),
                vertex(0, y + 1, z + 1, &mut builder),
            ];
            builder.add_face(&face).expect("-X");
        }
    }
    for x in 0..seg_x {
        for z in 0..seg_z {
            let face = [
                vertex(x, seg_y, z, &mut builder),
                vertex(x + 1, seg_y, z, &mut builder),
                vertex(x + 1, seg_y, z + 1, &mut builder),
                vertex(x, seg_y, z + 1, &mut builder),
            ];
            builder.add_face(&face).expect("+Y");
        }
    }
    for x in 0..seg_x {
        for z in 0..seg_z {
            let face = [
                vertex(x, 0, z, &mut builder),
                vertex(x, 0, z + 1, &mut builder),
                vertex(x + 1, 0, z + 1, &mut builder),
                vertex(x + 1, 0, z, &mut builder),
            ];
            builder.add_face(&face).expect("-Y");
        }
    }
    for x in 0..seg_x {
        for y in 0..seg_y {
            let face = [
                vertex(x, y, seg_z, &mut builder),
                vertex(x, y + 1, seg_z, &mut builder),
                vertex(x + 1, y + 1, seg_z, &mut builder),
                vertex(x + 1, y, seg_z, &mut builder),
            ];
            builder.add_face(&face).expect("+Z");
        }
    }
    for x in 0..seg_x {
        for y in 0..seg_y {
            let face = [
                vertex(x, y, 0, &mut builder),
                vertex(x + 1, y, 0, &mut builder),
                vertex(x + 1, y + 1, 0, &mut builder),
                vertex(x, y + 1, 0, &mut builder),
            ];
            builder.add_face(&face).expect("-Z");
        }
    }

    let build = builder.build().expect("box topology must build");
    let face_ids = &build.face_ids;
    let x_faces = seg_y * seg_z;
    let y_faces = seg_x * seg_z;
    let z_faces = seg_x * seg_y;
    let side_x_pos = face_ids[0..x_faces].to_vec();
    let side_x_neg = face_ids[x_faces..x_faces * 2].to_vec();
    let side_y_pos = face_ids[x_faces * 2..x_faces * 2 + y_faces].to_vec();
    let side_y_neg = face_ids[x_faces * 2 + y_faces..x_faces * 2 + y_faces * 2].to_vec();
    let side_z_pos =
        face_ids[x_faces * 2 + y_faces * 2..x_faces * 2 + y_faces * 2 + z_faces].to_vec();
    let side_z_neg = face_ids[x_faces * 2 + y_faces * 2 + z_faces..].to_vec();
    let mut assignments = Vec::with_capacity(face_ids.len());
    for face in &side_x_pos {
        assignments.push((*face, REGION_SIDE_X_POS));
    }
    for face in &side_x_neg {
        assignments.push((*face, REGION_SIDE_X_NEG));
    }
    for face in &side_y_pos {
        assignments.push((*face, REGION_SIDE_Y_POS));
    }
    for face in &side_y_neg {
        assignments.push((*face, REGION_SIDE_Y_NEG));
    }
    for face in &side_z_pos {
        assignments.push((*face, REGION_SIDE_Z_POS));
    }
    for face in &side_z_neg {
        assignments.push((*face, REGION_SIDE_Z_NEG));
    }
    let face_region = common::face_region_layer(face_ids, RegionId(0), &assignments);

    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![
            (SelectionName("faces.all"), face_ids.clone()),
            (SelectionName("faces.side_x_pos"), side_x_pos),
            (SelectionName("faces.side_x_neg"), side_x_neg),
            (SelectionName("faces.side_y_pos"), side_y_pos),
            (SelectionName("faces.side_y_neg"), side_y_neg),
            (SelectionName("faces.side_z_pos"), side_z_pos),
            (SelectionName("faces.side_z_neg"), side_z_neg),
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

    #[test]
    fn box_primitive_supports_asymmetric_segments() {
        let primitive = box_primitive(&BoxParams {
            size: [2.0, 3.0, 4.0],
            centered: true,
            segments: [2, 1, 3],
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        assert_eq!(primitive.mesh.faces().count(), 22);

        let side_x_pos = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.side_x_pos")
            .expect("faces.side_x_pos should exist");
        let side_x_neg = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.side_x_neg")
            .expect("faces.side_x_neg should exist");
        let side_y_pos = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.side_y_pos")
            .expect("faces.side_y_pos should exist");
        let side_y_neg = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.side_y_neg")
            .expect("faces.side_y_neg should exist");
        let side_z_pos = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.side_z_pos")
            .expect("faces.side_z_pos should exist");
        let side_z_neg = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.side_z_neg")
            .expect("faces.side_z_neg should exist");

        assert_eq!(side_x_pos.1.as_slice().len(), 3);
        assert_eq!(side_x_neg.1.as_slice().len(), 3);
        assert_eq!(side_y_pos.1.as_slice().len(), 6);
        assert_eq!(side_y_neg.1.as_slice().len(), 6);
        assert_eq!(side_z_pos.1.as_slice().len(), 2);
        assert_eq!(side_z_neg.1.as_slice().len(), 2);

        for face in side_y_pos.1.as_slice() {
            assert_eq!(primitive.face_region.get(*face), REGION_SIDE_Y_POS);
        }
        for face in side_z_neg.1.as_slice() {
            assert_eq!(primitive.face_region.get(*face), REGION_SIDE_Z_NEG);
        }
    }

    #[test]
    #[should_panic(expected = "box_primitive requires segments >= 1 on all axes")]
    fn box_primitive_rejects_zero_segments() {
        let _ = box_primitive(&BoxParams {
            size: [1.0, 1.0, 1.0],
            centered: true,
            segments: [0, 1, 1],
        });
    }
}
