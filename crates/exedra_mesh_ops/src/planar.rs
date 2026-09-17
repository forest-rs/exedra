// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Projection of ordered mesh loops onto their Newell-normal, centroid plane.
//!
//! This geometric projection does not establish polygon simplicity, a profile,
//! or correspondence between separate loops. Consumers validate their own shape
//! requirements. The automatically chosen axes can change abruptly as the normal
//! changes; use authored workplanes when orientation must remain controlled.

use crate::math::FloatExt as _;
use alloc::vec::Vec;
use exedra_math::{dot, sub};
use exedra_mesh::{HalfEdgeId, Mesh, VertexId};

/// A projected mesh loop and its frame in mesh coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopProjection {
    /// Counter-clockwise projected points, starting at the lowest vertex index.
    pub points: Vec<[f64; 2]>,
    /// Source vertices in the same order as `points`.
    pub vertices: Vec<VertexId>,
    /// Arithmetic centroid of the loop vertices.
    pub origin: [f64; 3],
    /// Unit local x axis: the least-aligned world axis projected into the plane.
    pub tangent: [f64; 3],
    /// Unit local y axis, `normal` crossed with `tangent`.
    pub bitangent: [f64; 3],
    /// Unit Newell normal following the input loop winding.
    pub normal: [f64; 3],
    /// Largest source-vertex distance from the plane, in mesh units.
    pub planar_deviation: f64,
}

/// Failure to project a mesh loop.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LoopProjectionError {
    /// Fewer than three vertices were supplied.
    TooShort {
        /// Number of vertices found.
        count: usize,
    },
    /// Edges are stale or do not form one closed, ordered loop.
    BrokenLoop,
    /// The tolerance is negative or nonfinite.
    InvalidTolerance,
    /// Positions are absent/nonfinite, or the loop has no finite nonzero normal.
    InvalidGeometry,
    /// The measured deviation exceeds the tolerance.
    NonPlanar {
        /// Measured maximum distance, in mesh units.
        deviation: f64,
        /// Requested maximum distance, in mesh units.
        allowed: f64,
    },
}

impl core::fmt::Display for LoopProjectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { count } => {
                write!(f, "loop needs at least three vertices, found {count}")
            }
            Self::BrokenLoop => write!(f, "edges do not form a live, closed, ordered loop"),
            Self::InvalidTolerance => write!(f, "loop tolerance must be finite and nonnegative"),
            Self::InvalidGeometry => write!(f, "loop has invalid positions or a degenerate normal"),
            Self::NonPlanar { deviation, allowed } => write!(
                f,
                "loop deviates {deviation} from its Newell plane (allowed {allowed})"
            ),
        }
    }
}
impl core::error::Error for LoopProjectionError {}

/// Projects a closed ordered half-edge loop into a deterministic local frame.
///
/// The origin is the vertex centroid, and the normal is Newell's area sum.
/// This is not a least-squares fit. Axis ties prefer x, then y, then z.
/// Input edges must join end to start, including the closing pair. The result
/// is rooted at the lowest vertex index and oriented counter-clockwise without
/// changing that root. This ordering does not infer loft correspondence.
///
/// Work and storage are linear in the supplied edge count. `max_planar_deviation`
/// is a finite, nonnegative distance in mesh units. The mesh is never modified.
/// Polygon simplicity and boundary contact are left to the consuming operation.
///
/// # Errors
/// Returns a typed failure for invalid edges, geometry or tolerance, or excessive
/// deviation. No missing position is replaced or invalid tolerance ignored.
pub fn project_loop(
    mesh: &Mesh,
    loop_edges: &[HalfEdgeId],
    max_planar_deviation: f64,
) -> Result<LoopProjection, LoopProjectionError> {
    use LoopProjectionError as Error;
    if !max_planar_deviation.is_finite() || max_planar_deviation < 0.0 {
        return Err(Error::InvalidTolerance);
    }
    // Collect the loop's vertices in traversal order.
    let mut vertices: Vec<VertexId> = Vec::with_capacity(loop_edges.len());
    for (i, &edge) in loop_edges.iter().enumerate() {
        let to = mesh.to_vertex(edge).ok_or(Error::BrokenLoop)?;
        let next = loop_edges[(i + 1) % loop_edges.len()];
        if mesh.from_vertex(next) != Some(to) {
            return Err(Error::BrokenLoop);
        }
        vertices.push(to);
    }
    vertices.dedup();
    if vertices.len() >= 2 && vertices.first() == vertices.last() {
        vertices.pop();
    }
    if vertices.len() < 3 {
        return Err(Error::TooShort {
            count: vertices.len(),
        });
    }

    // Canonical root: rotate so the lowest vertex id leads.
    let root = vertices
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| v.index())
        .map(|(i, _)| i)
        .unwrap_or(0);
    vertices.rotate_left(root);

    let positions: Vec<[f64; 3]> = vertices
        .iter()
        .map(|&v| {
            let p = mesh.vertex_position(v).ok_or(Error::InvalidGeometry)?;
            if p.iter().any(|value| !value.is_finite()) {
                return Err(Error::InvalidGeometry);
            }
            Ok(p.map(f64::from))
        })
        .collect::<Result<_, Error>>()?;

    // Newell normal + centroid origin.
    let mut normal = [0.0_f64; 3];
    let mut origin = [0.0_f64; 3];
    let n = positions.len();
    for i in 0..n {
        let a = positions[i];
        let b = positions[(i + 1) % n];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
        for axis in 0..3 {
            origin[axis] += a[axis] / n as f64;
        }
    }
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt_ext();
    if !len.is_finite() || len <= 0.0 {
        return Err(Error::InvalidGeometry);
    }
    for c in &mut normal {
        *c /= len;
    }

    // Deterministic in-plane frame: Gram-Schmidt the world axis least
    // aligned with the normal (ties x before y before z).
    fn fabs(v: f64) -> f64 {
        if v < 0.0 { -v } else { v }
    }
    let abs = [fabs(normal[0]), fabs(normal[1]), fabs(normal[2])];
    let axis_index = if abs[0] <= abs[1] && abs[0] <= abs[2] {
        0
    } else if abs[1] <= abs[2] {
        1
    } else {
        2
    };
    let mut axis = [0.0; 3];
    axis[axis_index] = 1.0;
    let dot = axis[0] * normal[0] + axis[1] * normal[1] + axis[2] * normal[2];
    let mut tangent = [
        axis[0] - dot * normal[0],
        axis[1] - dot * normal[1],
        axis[2] - dot * normal[2],
    ];
    let tangent_len =
        (tangent[0] * tangent[0] + tangent[1] * tangent[1] + tangent[2] * tangent[2]).sqrt_ext();
    for c in &mut tangent {
        *c /= tangent_len;
    }
    let bitangent = [
        normal[1] * tangent[2] - normal[2] * tangent[1],
        normal[2] * tangent[0] - normal[0] * tangent[2],
        normal[0] * tangent[1] - normal[1] * tangent[0],
    ];

    // Project; measure planarity.
    let deviation =
        max_plane_deviation(&positions, normal, origin).ok_or(Error::InvalidGeometry)?;
    let projected: Vec<[f64; 2]> = positions
        .iter()
        .map(|p| {
            let d = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
            [
                d[0] * tangent[0] + d[1] * tangent[1] + d[2] * tangent[2],
                d[0] * bitangent[0] + d[1] * bitangent[1] + d[2] * bitangent[2],
            ]
        })
        .collect();
    if deviation > max_planar_deviation {
        return Err(Error::NonPlanar {
            deviation,
            allowed: max_planar_deviation,
        });
    }

    // Orient counter-clockwise; flip by reversal
    // (keeping the canonical root leading) when the projection winds
    // clockwise.
    let mut area = 0.0;
    for i in 0..projected.len() {
        let a = projected[i];
        let b = projected[(i + 1) % projected.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    let (ordered_points, ordered_vertices): (Vec<[f64; 2]>, Vec<VertexId>) = if area < 0.0 {
        let mut points = projected;
        let mut verts = vertices;
        points[1..].reverse();
        verts[1..].reverse();
        (points, verts)
    } else {
        (projected, vertices)
    };

    Ok(LoopProjection {
        points: ordered_points,
        vertices: ordered_vertices,
        origin,
        tangent,
        bitangent,
        normal,
        planar_deviation: deviation,
    })
}

/// Shared distance measurement; callers choose their plane and tolerance.
pub(crate) fn max_plane_deviation(
    points: &[[f64; 3]],
    normal: [f64; 3],
    origin: [f64; 3],
) -> Option<f64> {
    let mut maximum = 0.0_f64;
    for &point in points {
        let distance = dot(normal, sub(point, origin)).abs();
        if !distance.is_finite() {
            return None;
        }
        maximum = maximum.max(distance);
    }
    Some(maximum)
}
