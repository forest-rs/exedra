// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra::VertexId;

use crate::math::FloatExt;

pub(crate) fn centroid(mesh: &exedra::Mesh, vertices: &[VertexId]) -> Option<[f32; 3]> {
    if vertices.is_empty() {
        return None;
    }
    let mut sum = [0.0_f32, 0.0, 0.0];
    for &vertex in vertices {
        let position = mesh.vertex_position(vertex)?;
        sum[0] += position[0];
        sum[1] += position[1];
        sum[2] += position[2];
    }
    let inv = 1.0 / (vertices.len() as f32);
    Some([sum[0] * inv, sum[1] * inv, sum[2] * inv])
}

pub(crate) fn normalized_face_normal(
    mesh: &exedra::Mesh,
    vertices: &[VertexId],
) -> Option<[f32; 3]> {
    let mut nx = 0.0_f32;
    let mut ny = 0.0_f32;
    let mut nz = 0.0_f32;
    for i in 0..vertices.len() {
        let current = mesh.vertex_position(vertices[i])?;
        let next = mesh.vertex_position(vertices[(i + 1) % vertices.len()])?;
        nx += (current[1] - next[1]) * (current[2] + next[2]);
        ny += (current[2] - next[2]) * (current[0] + next[0]);
        nz += (current[0] - next[0]) * (current[1] + next[1]);
    }
    let length_sq = nx * nx + ny * ny + nz * nz;
    if length_sq <= 1e-12 {
        return None;
    }
    let inv_len = 1.0 / length_sq.sqrt_ext();
    Some([nx * inv_len, ny * inv_len, nz * inv_len])
}
