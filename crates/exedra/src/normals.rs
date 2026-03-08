// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic corner-normal derivation and authored-normal source policy.

use alloc::vec;
use alloc::vec::Vec;

use crate::math::FloatExt;
use crate::{CornerId, FaceId, HalfEdgeId, Mesh};

/// Weighting mode used for derived corner normals.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum NormalWeightMode {
    /// Weight each contributing face by the corner angle only.
    Angle,
    /// Weight each contributing face by polygon area only.
    Area,
    /// Weight each contributing face by angle * area.
    #[default]
    AngleArea,
}

/// Parameters controlling deterministic derived corner normals.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct NormalParams {
    /// Optional automatic hard-edge threshold in degrees.
    ///
    /// When set, interior edges whose dihedral angle exceeds the threshold are
    /// treated as hard shading boundaries in addition to explicit sharp edges.
    pub auto_sharp_angle_degrees: Option<f32>,
    /// Face weighting mode used during corner-normal accumulation.
    pub weight_mode: NormalWeightMode,
}

impl Default for NormalParams {
    fn default() -> Self {
        Self {
            auto_sharp_angle_degrees: None,
            weight_mode: NormalWeightMode::AngleArea,
        }
    }
}

/// Source policy for render/extraction normals.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum NormalsSource {
    /// Ignore authored overrides and always use derived geometry normals.
    #[default]
    Derived,
    /// Use authored corner overrides where present, otherwise use derived normals.
    CustomOrDerived,
    /// Use authored corner overrides only; missing overrides become zero normals.
    CustomOnly,
}

/// Deterministic corner-normal buffer indexed by [`CornerId`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DerivedCornerNormals {
    values: Vec<Option<[f32; 3]>>,
}

impl DerivedCornerNormals {
    pub(crate) fn with_slot_count(slots: usize) -> Self {
        Self {
            values: vec![None; slots],
        }
    }

    pub(crate) fn set(&mut self, corner: CornerId, normal: [f32; 3]) {
        let index = corner.index() as usize;
        if index < self.values.len() {
            self.values[index] = Some(normal);
        }
    }

    /// Returns the derived normal for `corner`, when present.
    #[must_use]
    pub fn get(&self, corner: CornerId) -> Option<[f32; 3]> {
        self.values.get(corner.index() as usize).copied().flatten()
    }
}

#[derive(Copy, Clone, Debug)]
struct FaceNormalData {
    unit: [f32; 3],
    area_weight: f32,
}

impl Mesh {
    /// Computes derived corner normals for the current mesh state.
    ///
    /// The result is deterministic for identical mesh state and parameters.
    #[must_use]
    pub fn derive_corner_normals(&self, params: &NormalParams) -> DerivedCornerNormals {
        let face_normals = compute_face_normals(self);
        let mut by_vertex = vec![Vec::<CornerId>::new(); self.vertices.slot_count()];
        for face in self.faces() {
            for corner in self.face_loop(face) {
                let Some(vertex) = self.to_vertex(corner) else {
                    continue;
                };
                by_vertex[vertex.index() as usize].push(corner);
            }
        }

        let mut out = DerivedCornerNormals::with_slot_count(self.half_edges.slot_count());
        for corners in by_vertex {
            if corners.is_empty() {
                continue;
            }
            for &seed in &corners {
                if out.get(seed).is_some() {
                    continue;
                }
                let group = smooth_corner_group(self, &face_normals, seed, params);
                if group.is_empty() {
                    continue;
                }
                let mut sum = [0.0_f32, 0.0, 0.0];
                for corner in &group {
                    let Some(face) = self.face(*corner) else {
                        continue;
                    };
                    let Some(face_data) = face_normal_data(face, &face_normals) else {
                        continue;
                    };
                    let weight =
                        corner_weight(self, *corner, face_data.area_weight, params.weight_mode);
                    sum[0] += face_data.unit[0] * weight;
                    sum[1] += face_data.unit[1] * weight;
                    sum[2] += face_data.unit[2] * weight;
                }
                let Some(normal) = normalize3(sum) else {
                    continue;
                };
                for corner in group {
                    out.set(corner, normal);
                }
            }
        }
        out
    }
}

fn compute_face_normals(mesh: &Mesh) -> Vec<Option<FaceNormalData>> {
    let mut normals = vec![None; mesh.faces.slot_count()];
    for face in mesh.faces() {
        let Some((unit, area_weight)) = face_normal(mesh, face) else {
            continue;
        };
        normals[face.index() as usize] = Some(FaceNormalData { unit, area_weight });
    }
    normals
}

fn face_normal_data(face: FaceId, values: &[Option<FaceNormalData>]) -> Option<FaceNormalData> {
    if face == FaceId::OUTSIDE {
        return None;
    }
    values.get(face.index() as usize).copied().flatten()
}

fn face_normal(mesh: &Mesh, face: FaceId) -> Option<([f32; 3], f32)> {
    let corners = mesh.face_loop(face).collect::<Vec<_>>();
    if corners.len() < 3 {
        return None;
    }
    let mut nx = 0.0_f32;
    let mut ny = 0.0_f32;
    let mut nz = 0.0_f32;
    for i in 0..corners.len() {
        let current = corner_position(mesh, corners[i])?;
        let next = corner_position(mesh, corners[(i + 1) % corners.len()])?;
        nx += (current[1] - next[1]) * (current[2] + next[2]);
        ny += (current[2] - next[2]) * (current[0] + next[0]);
        nz += (current[0] - next[0]) * (current[1] + next[1]);
    }
    let length_sq = nx * nx + ny * ny + nz * nz;
    if length_sq <= 1.0e-12 {
        return None;
    }
    let length = length_sq.sqrt_ext();
    let inv_len = 1.0 / length;
    Some(([nx * inv_len, ny * inv_len, nz * inv_len], length))
}

fn corner_position(mesh: &Mesh, corner: CornerId) -> Option<[f32; 3]> {
    let vertex = mesh.to_vertex(corner)?;
    mesh.vertex_position(vertex).copied()
}

fn smooth_corner_group(
    mesh: &Mesh,
    face_normals: &[Option<FaceNormalData>],
    seed: CornerId,
    params: &NormalParams,
) -> Vec<CornerId> {
    let seed_vertex = mesh.to_vertex(seed).expect("seed corner must have vertex");
    let mut stack = vec![seed];
    let mut visited = Vec::<CornerId>::new();
    while let Some(corner) = stack.pop() {
        if visited.contains(&corner) {
            continue;
        }
        visited.push(corner);
        for neighbor in corner_neighbors(mesh, corner).into_iter().flatten() {
            if mesh.to_vertex(neighbor) != Some(seed_vertex) {
                continue;
            }
            if edge_is_sharp_between(mesh, face_normals, corner, neighbor, params) {
                continue;
            }
            if !visited.contains(&neighbor) {
                stack.push(neighbor);
            }
        }
    }
    visited.sort_unstable();
    visited
}

fn corner_neighbors(mesh: &Mesh, corner: CornerId) -> [Option<CornerId>; 2] {
    let across_incoming = mesh
        .twin(corner)
        .and_then(|twin| {
            mesh.face(twin)
                .filter(|face| *face != FaceId::OUTSIDE)
                .map(|_| twin)
        })
        .and_then(|twin| mesh.prev(twin));
    let across_outgoing = mesh
        .next(corner)
        .and_then(|next| mesh.twin(next))
        .and_then(|twin| {
            mesh.face(twin)
                .filter(|face| *face != FaceId::OUTSIDE)
                .map(|_| twin)
        });
    [across_incoming, across_outgoing]
}

fn edge_is_sharp_between(
    mesh: &Mesh,
    face_normals: &[Option<FaceNormalData>],
    from_corner: CornerId,
    to_corner: CornerId,
    params: &NormalParams,
) -> bool {
    let Some(shared_edge) = shared_vertex_edge(mesh, from_corner, to_corner) else {
        return true;
    };
    if mesh.face(shared_edge) == Some(FaceId::OUTSIDE) {
        return true;
    }
    let Some(twin) = mesh.twin(shared_edge) else {
        return true;
    };
    if mesh.face(twin) == Some(FaceId::OUTSIDE) {
        return true;
    }
    if mesh
        .edge_sharpness(shared_edge)
        .is_some_and(|sharp| sharp > 0.0)
    {
        return true;
    }
    let Some(threshold_degrees) = params.auto_sharp_angle_degrees else {
        return false;
    };
    let Some(face_a) = mesh.face(shared_edge) else {
        return true;
    };
    let Some(face_b) = mesh.face(twin) else {
        return true;
    };
    let Some(normal_a) = face_normal_data(face_a, face_normals) else {
        return true;
    };
    let Some(normal_b) = face_normal_data(face_b, face_normals) else {
        return true;
    };
    let dot = dot3(normal_a.unit, normal_b.unit).clamp(-1.0, 1.0);
    let threshold_radians = threshold_degrees * core::f32::consts::PI / 180.0;
    dot <= threshold_radians.cos_ext()
}

fn shared_vertex_edge(
    mesh: &Mesh,
    from_corner: CornerId,
    to_corner: CornerId,
) -> Option<HalfEdgeId> {
    let incoming_neighbor = mesh
        .twin(from_corner)
        .and_then(|twin| mesh.prev(twin))
        .filter(|corner| *corner == to_corner)
        .map(|_| from_corner);
    let outgoing_neighbor = mesh
        .next(from_corner)
        .and_then(|next| mesh.twin(next))
        .filter(|corner| *corner == to_corner)
        .and_then(|_| mesh.next(from_corner));
    incoming_neighbor.or(outgoing_neighbor)
}

fn corner_weight(mesh: &Mesh, corner: CornerId, area_weight: f32, mode: NormalWeightMode) -> f32 {
    let angle = corner_angle(mesh, corner).unwrap_or(1.0);
    match mode {
        NormalWeightMode::Angle => angle,
        NormalWeightMode::Area => area_weight,
        NormalWeightMode::AngleArea => angle * area_weight,
    }
}

fn corner_angle(mesh: &Mesh, corner: CornerId) -> Option<f32> {
    let vertex = mesh.to_vertex(corner)?;
    let prev_vertex = mesh.from_vertex(corner)?;
    let next_vertex = mesh.next(corner).and_then(|next| mesh.to_vertex(next))?;
    let center = mesh.vertex_position(vertex)?;
    let prev = mesh.vertex_position(prev_vertex)?;
    let next = mesh.vertex_position(next_vertex)?;
    let a = sub3(*prev, *center);
    let b = sub3(*next, *center);
    let a_unit = normalize3(a)?;
    let b_unit = normalize3(b)?;
    let dot = dot3(a_unit, b_unit).clamp(-1.0, 1.0);
    Some(dot.acos_ext())
}

fn normalize3(value: [f32; 3]) -> Option<[f32; 3]> {
    let len_sq = dot3(value, value);
    if len_sq <= 1.0e-12 {
        return None;
    }
    let inv = 1.0 / len_sq.sqrt_ext();
    Some([value[0] * inv, value[1] * inv, value[2] * inv])
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use crate::{BuildParams, ChangeSetBuilder, Mesh, MeshBuilder, op};

    use super::{NormalParams, NormalWeightMode};

    #[test]
    fn derived_normals_on_triangle_match_face_normal() {
        let mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle mesh should build");
        let normals = mesh.derive_corner_normals(&NormalParams::default());
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                assert_eq!(normals.get(corner), Some([0.0, 0.0, 1.0]));
            }
        }
    }

    #[test]
    fn derived_normals_respect_explicit_sharp_edges() {
        let mut builder = MeshBuilder::new();
        let v0 = builder.push_vertex([-1.0, -1.0, -1.0]);
        let v1 = builder.push_vertex([1.0, -1.0, -1.0]);
        let v2 = builder.push_vertex([1.0, 1.0, -1.0]);
        let v3 = builder.push_vertex([-1.0, 1.0, -1.0]);
        let v4 = builder.push_vertex([-1.0, -1.0, 1.0]);
        let v5 = builder.push_vertex([1.0, -1.0, 1.0]);
        let v6 = builder.push_vertex([1.0, 1.0, 1.0]);
        let v7 = builder.push_vertex([-1.0, 1.0, 1.0]);
        builder.add_face(&[v0, v3, v2, v1]).expect("face");
        builder.add_face(&[v4, v5, v6, v7]).expect("face");
        builder.add_face(&[v0, v1, v5, v4]).expect("face");
        builder.add_face(&[v1, v2, v6, v5]).expect("face");
        builder.add_face(&[v2, v3, v7, v6]).expect("face");
        builder.add_face(&[v3, v0, v4, v7]).expect("face");
        let built = builder.build().expect("cube mesh should build");
        let mut mesh = built.mesh;
        let edges = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .filter(|&edge| {
                mesh.twin(edge)
                    .is_some_and(|twin| core::cmp::min(edge, twin) == edge)
            })
            .collect::<Vec<_>>();

        let mut edit = mesh.edit_with(ChangeSetBuilder::new());
        for edge in edges {
            op::set_edge_sharpness(&mut edit, edge, 1.0).expect("cube edge should be live");
        }
        let _ = edit.finish();

        let normals = mesh.derive_corner_normals(&NormalParams::default());
        let top_face = mesh
            .faces()
            .find(|&face| {
                mesh.face_loop(face).all(|corner| {
                    mesh.to_vertex(corner)
                        .and_then(|v| mesh.vertex_position(v))
                        .is_some_and(|p| p[2] > 0.0)
                })
            })
            .expect("top face exists");
        for corner in mesh.face_loop(top_face) {
            assert_eq!(normals.get(corner), Some([0.0, 0.0, 1.0]));
        }
    }

    #[test]
    fn normal_params_default_is_stable() {
        let params = NormalParams::default();
        assert_eq!(params.auto_sharp_angle_degrees, None);
        assert_eq!(params.weight_mode, NormalWeightMode::AngleArea);
    }
}
