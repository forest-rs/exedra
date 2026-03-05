// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic icosphere primitive.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use exedra::MeshBuilder;

use crate::{Primitive, RegionId, SelectionName, common};

/// Region ID for icosphere body faces.
pub const REGION_BODY: RegionId = RegionId(1);

/// Parameters for [`icosphere`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct IcosphereParams {
    /// Sphere radius.
    pub radius: f32,
    /// Number of subdivision rounds.
    pub subdivisions: u32,
}

impl Default for IcosphereParams {
    fn default() -> Self {
        Self {
            radius: 1.0,
            subdivisions: 1,
        }
    }
}

/// Builds a deterministic icosphere primitive.
///
/// Starts from an icosahedron and applies midpoint subdivision rounds.
///
/// Selection semantics:
/// - `faces.all`: all triangular faces.
#[must_use]
pub fn icosphere(params: &IcosphereParams) -> Primitive {
    let phi = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let mut positions = vec![
        [-1.0, phi, 0.0],
        [1.0, phi, 0.0],
        [-1.0, -phi, 0.0],
        [1.0, -phi, 0.0],
        [0.0, -1.0, phi],
        [0.0, 1.0, phi],
        [0.0, -1.0, -phi],
        [0.0, 1.0, -phi],
        [phi, 0.0, -1.0],
        [phi, 0.0, 1.0],
        [-phi, 0.0, -1.0],
        [-phi, 0.0, 1.0],
    ];
    for p in &mut positions {
        *p = normalize_scaled(*p, params.radius);
    }

    let mut faces = vec![
        [0_u32, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];

    for _ in 0..params.subdivisions {
        let mut midpoint_cache = BTreeMap::<(u32, u32), u32>::new();
        let mut next_faces = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let ab = midpoint_index(&mut positions, &mut midpoint_cache, a, b, params.radius);
            let bc = midpoint_index(&mut positions, &mut midpoint_cache, b, c, params.radius);
            let ca = midpoint_index(&mut positions, &mut midpoint_cache, c, a, params.radius);

            next_faces.push([a, ab, ca]);
            next_faces.push([b, bc, ab]);
            next_faces.push([c, ca, bc]);
            next_faces.push([ab, bc, ca]);
        }
        faces = next_faces;
    }

    let mut builder = MeshBuilder::new();
    for p in &positions {
        let _ = builder.push_vertex(*p);
    }
    for [a, b, c] in &faces {
        builder
            .add_face(&[*a, *b, *c])
            .expect("triangle should be valid");
    }

    let build = builder.build().expect("icosphere topology must build");
    let face_ids = build.face_ids.clone();
    let assignments = face_ids
        .iter()
        .map(|face| (*face, REGION_BODY))
        .collect::<Vec<_>>();
    let face_region = common::face_region_layer(&face_ids, RegionId(0), &assignments);

    common::primitive_from_parts(
        build.mesh,
        face_region,
        vec![(SelectionName("faces.all"), face_ids)],
        Vec::new(),
    )
}

fn midpoint_index(
    positions: &mut Vec<[f32; 3]>,
    cache: &mut BTreeMap<(u32, u32), u32>,
    a: u32,
    b: u32,
    radius: f32,
) -> u32 {
    let key = if a < b { (a, b) } else { (b, a) };
    if let Some(&index) = cache.get(&key) {
        return index;
    }
    let pa = positions[a as usize];
    let pb = positions[b as usize];
    let p = normalize_scaled(
        [
            (pa[0] + pb[0]) * 0.5,
            (pa[1] + pb[1]) * 0.5,
            (pa[2] + pb[2]) * 0.5,
        ],
        radius,
    );
    let index = common::usize_to_u32(positions.len());
    positions.push(p);
    cache.insert(key, index);
    index
}

fn normalize_scaled(v: [f32; 3], radius: f32) -> [f32; 3] {
    let len = common::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    if len == 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        let inv = radius / len;
        [v[0] * inv, v[1] * inv, v[2] * inv]
    }
}

#[cfg(test)]
mod tests {
    use exedra::ExtractParams;

    use super::{IcosphereParams, REGION_BODY, icosphere};

    #[test]
    fn icosphere_face_count_scales_by_subdivision() {
        let s0 = icosphere(&IcosphereParams {
            subdivisions: 0,
            ..IcosphereParams::default()
        });
        let s1 = icosphere(&IcosphereParams {
            subdivisions: 1,
            ..IcosphereParams::default()
        });
        let s2 = icosphere(&IcosphereParams {
            subdivisions: 2,
            ..IcosphereParams::default()
        });
        assert_eq!(s0.mesh.faces().count(), 20);
        assert_eq!(s1.mesh.faces().count(), 80);
        assert_eq!(s2.mesh.faces().count(), 320);
    }

    #[test]
    fn icosphere_builds_valid_mesh_and_regions() {
        let primitive = icosphere(&IcosphereParams {
            subdivisions: 1,
            ..IcosphereParams::default()
        });
        assert!(primitive.mesh.validate_fast().is_empty());
        assert!(primitive.mesh.validate_deep().is_empty());
        let faces_all = primitive
            .selections
            .face_sets
            .iter()
            .find(|(name, _)| name.0 == "faces.all")
            .expect("faces.all should exist")
            .1
            .as_slice();
        for face in faces_all {
            assert_eq!(primitive.face_region.get(*face), REGION_BODY);
        }
    }

    #[test]
    fn icosphere_is_deterministic() {
        let params = IcosphereParams {
            radius: 2.0,
            subdivisions: 2,
        };
        let a = icosphere(&params);
        let b = icosphere(&params);
        let (tri_a, stats_a) = a.mesh.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = b.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.positions, tri_b.positions);
        assert_eq!(a.face_region, b.face_region);
        assert_eq!(a.selections, b.selections);
    }
}
