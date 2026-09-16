// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Corner UV source policy for render extraction and the shared box-projection
//! math behind it.
//!
//! Authored UVs live in the sparse `attr::CORNER_UV` layer. [`UvSource`]
//! decides what extraction emits for a face corner that has no authored value:
//! the historical zero fallback, or a deterministic box projection of the
//! corner's destination vertex. The projection is the same function the
//! `uv.box` operator writes into the mesh, so extracting with
//! [`UvSource::CustomOrBoxProjected`] and extracting after `uv.box` produce
//! identical render buffers for identical parameters.

use alloc::vec::Vec;

use exedra_math::{add, cross, sub};

use crate::{CornerId, FaceId, Mesh};

/// Default dominant-axis tie-break epsilon for [`dominant_box_plane`].
///
/// Extraction under [`UvSource::CustomOrBoxProjected`] uses this value; the
/// `uv.box` operator exposes it as its default `normal_epsilon`.
pub const DEFAULT_BOX_NORMAL_EPSILON: f32 = 1.0e-6;

/// Source policy for render/extraction UVs.
///
/// Authored corner UVs are never overwritten: every variant emits them where
/// present. The variants differ only in what a corner without an authored UV
/// receives.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum UvSource {
    /// Authored corner UVs only; missing corners emit `[0.0, 0.0]`.
    #[default]
    CustomOnly,
    /// Authored corner UVs where present; otherwise the corner position
    /// projected on the face's dominant-axis plane and multiplied by `scale`.
    ///
    /// The plane is selected once per face by [`dominant_box_plane`] with
    /// [`DEFAULT_BOX_NORMAL_EPSILON`], so a face's projected corners share
    /// one plane and adjacent faces on different planes split their shared
    /// render vertices exactly as authored UV seams do. Exedra is
    /// unit-agnostic; callers whose positions are in meters get one UV unit
    /// per meter of surface with `scale: 1.0`.
    CustomOrBoxProjected {
        /// Uniform multiplier applied to the projected coordinates.
        scale: f32,
    },
}

/// Axis-aligned plane a face projects onto under box UV projection.
///
/// The sign records which way the face normal points along the dominant axis;
/// it flips the horizontal UV coordinate so textures read the same way from
/// outside on opposite faces of a box.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum BoxPlane {
    /// Normal dominated by +X; UV = `[-z, y]`.
    PosX,
    /// Normal dominated by -X; UV = `[z, y]`.
    NegX,
    /// Normal dominated by +Y; UV = `[x, -z]`.
    PosY,
    /// Normal dominated by -Y; UV = `[x, z]`.
    NegY,
    /// Normal dominated by +Z; UV = `[x, y]`.
    PosZ,
    /// Normal dominated by -Z; UV = `[-x, y]`.
    NegZ,
}

/// Projects `position` onto `plane`, then applies `scale` and `offset`.
#[must_use]
pub fn project_box_position(
    position: [f32; 3],
    plane: BoxPlane,
    scale: f32,
    offset: [f32; 2],
) -> [f32; 2] {
    let p = position;
    let base = match plane {
        BoxPlane::PosX => [-p[2], p[1]],
        BoxPlane::NegX => [p[2], p[1]],
        BoxPlane::PosY => [p[0], -p[2]],
        BoxPlane::NegY => [p[0], p[2]],
        BoxPlane::PosZ => [p[0], p[1]],
        BoxPlane::NegZ => [-p[0], p[1]],
    };
    [base[0] * scale + offset[0], base[1] * scale + offset[1]]
}

/// Projects the destination vertex of `corner` onto `plane`.
///
/// Uses the corner's destination vertex, matching render extraction. Returns
/// `None` when the corner is stale or its vertex has no position.
#[must_use]
pub fn project_corner_box(
    mesh: &Mesh,
    corner: CornerId,
    plane: BoxPlane,
    scale: f32,
    offset: [f32; 2],
) -> Option<[f32; 2]> {
    let position = corner_position(mesh, corner)?;
    Some(project_box_position(position, plane, scale, offset))
}

/// Selects the box projection plane for `face` from its dominant normal axis.
///
/// The face normal is the signed fan sum over the face loop, so concave
/// polygons resolve correctly. Ties within `epsilon` prefer X, then Y, then Z.
/// A degenerate normal (fewer than three corners, or every component below
/// `epsilon`) falls back to [`BoxPlane::PosZ`]; the second value reports
/// that fallback.
#[must_use]
pub fn dominant_box_plane(mesh: &Mesh, face: FaceId, epsilon: f32) -> (BoxPlane, bool) {
    let Some([nx, ny, nz]) = face_normal_sum(mesh, face) else {
        return (BoxPlane::PosZ, true);
    };
    let ax = nx.abs();
    let ay = ny.abs();
    let az = nz.abs();
    let max_axis = ax.max(ay).max(az);
    if max_axis < epsilon {
        return (BoxPlane::PosZ, true);
    }
    if ax + epsilon >= max_axis {
        return (
            if nx >= 0.0 {
                BoxPlane::PosX
            } else {
                BoxPlane::NegX
            },
            false,
        );
    }
    if ay + epsilon >= max_axis {
        return (
            if ny >= 0.0 {
                BoxPlane::PosY
            } else {
                BoxPlane::NegY
            },
            false,
        );
    }
    (
        if nz >= 0.0 {
            BoxPlane::PosZ
        } else {
            BoxPlane::NegZ
        },
        false,
    )
}

/// Unnormalized face normal: the signed fan sum of the face loop relative to
/// its first corner. `None` for faces with fewer than three corners or any
/// corner without a positioned destination vertex.
pub(crate) fn face_normal_sum(mesh: &Mesh, face: FaceId) -> Option<[f32; 3]> {
    let corners = mesh.face_loop(face).collect::<Vec<_>>();
    if corners.len() < 3 {
        return None;
    }
    // The signed fan sum also handles concave polygons. Taking all products
    // relative to a face vertex avoids cancellation from its world offset.
    let origin = corner_position(mesh, corners[0])?;
    let mut vector = [0.0_f32; 3];
    for pair in corners[1..].windows(2) {
        let current = sub(corner_position(mesh, pair[0])?, origin);
        let next = sub(corner_position(mesh, pair[1])?, origin);
        vector = add(vector, cross(current, next));
    }
    Some(vector)
}

/// Position of the destination vertex of `corner`, when both are live.
pub(crate) fn corner_position(mesh: &Mesh, corner: CornerId) -> Option<[f32; 3]> {
    let vertex = mesh.to_vertex(corner)?;
    mesh.vertex_position(vertex).copied()
}

#[cfg(test)]
mod tests {
    use crate::MeshBuilder;

    use super::{BoxPlane, DEFAULT_BOX_NORMAL_EPSILON, dominant_box_plane, project_box_position};

    fn quad(points: [[f32; 3]; 4]) -> crate::Mesh {
        let mut builder = MeshBuilder::new();
        for p in points {
            builder.push_vertex(p);
        }
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        builder.build().expect("build should succeed").mesh
    }

    #[test]
    fn dominant_plane_follows_signed_face_normal() {
        let cases = [
            (
                [
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [0.0, 1.0, 0.0],
                ],
                BoxPlane::PosZ,
            ),
            (
                [
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [1.0, 0.0, 0.0],
                ],
                BoxPlane::NegZ,
            ),
            (
                [
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [0.0, 1.0, 1.0],
                    [0.0, 0.0, 1.0],
                ],
                BoxPlane::PosX,
            ),
            (
                [
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.0, 1.0, 1.0],
                    [0.0, 1.0, 0.0],
                ],
                BoxPlane::NegX,
            ),
            (
                [
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0],
                ],
                BoxPlane::PosY,
            ),
            (
                [
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0],
                ],
                BoxPlane::NegY,
            ),
        ];
        for (points, expected) in cases {
            let mesh = quad(points);
            let face = mesh.faces().next().expect("one face");
            assert_eq!(
                dominant_box_plane(&mesh, face, DEFAULT_BOX_NORMAL_EPSILON),
                (expected, false),
                "points {points:?}"
            );
        }
    }

    #[test]
    fn degenerate_face_falls_back_to_positive_z() {
        let mesh = quad([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]]);
        let face = mesh.faces().next().expect("one face");
        assert_eq!(
            dominant_box_plane(&mesh, face, DEFAULT_BOX_NORMAL_EPSILON),
            (BoxPlane::PosZ, true)
        );
    }

    #[test]
    fn tie_break_prefers_x_then_y() {
        // Diagonal faces whose normal has two equal dominant components.
        let xy = quad([
            [0.0, 0.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, -1.0, 1.0],
            [0.0, 0.0, 1.0],
        ]);
        let face = xy.faces().next().expect("one face");
        assert_eq!(
            dominant_box_plane(&xy, face, DEFAULT_BOX_NORMAL_EPSILON).0,
            BoxPlane::NegX
        );
        let yz = quad([
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, -1.0],
            [0.0, 1.0, -1.0],
        ]);
        let face = yz.faces().next().expect("one face");
        assert_eq!(
            dominant_box_plane(&yz, face, DEFAULT_BOX_NORMAL_EPSILON).0,
            BoxPlane::PosY
        );
    }

    #[test]
    fn projection_applies_scale_then_offset() {
        let p = [2.0, 3.0, 5.0];
        assert_eq!(
            project_box_position(p, BoxPlane::PosZ, 1.0, [0.0; 2]),
            [2.0, 3.0]
        );
        assert_eq!(
            project_box_position(p, BoxPlane::PosX, 2.0, [1.0, -1.0]),
            [-9.0, 5.0]
        );
        assert_eq!(
            project_box_position(p, BoxPlane::NegY, 0.5, [0.0; 2]),
            [1.0, 2.5]
        );
    }
}
