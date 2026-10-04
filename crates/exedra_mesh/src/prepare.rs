// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checked preparation of indexed polygon soup before topology construction.
//!
//! This module owns geometric validation and source-corner triangulation. It
//! does not weld vertices, build half-edge topology, or interpret attributes.

use alloc::vec::Vec;
use exedra_triangulate::predicates::{Orientation, Orientation3d, orient2d, orient3d};
use exedra_triangulate::{PolygonInput, TriError, TriParams, triangulate_preserving_boundary};

/// Treatment of exactly coincident consecutive corners, including the closing edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CoincidentCornerPolicy {
    /// Reject them; the caller retains control over potentially different attributes.
    #[default]
    Reject,
    /// Keep the first corner of each run and report discarded source offsets.
    ///
    /// A closing duplicate of the first corner is discarded. Equality is exact
    /// position equality, whether the corners reference the same vertex or
    /// different vertices. Nonconsecutive coincidences remain errors for
    /// nondegenerate faces; [`DegenerateFacePolicy`] takes precedence otherwise.
    DropConsecutive,
}

/// Treatment of faces whose positions are not exactly coplanar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WarpedFacePolicy {
    /// Reject any departure from exact coplanarity; no tolerance is implied.
    #[default]
    Reject,
    /// Triangulate the dominant-axis projection and retain original 3D positions.
    ///
    /// Only a simple projection is supported. This chooses a piecewise planar
    /// surface; it does not establish a unique surface for a warped polygon.
    Project,
}

/// Treatment of faces with fewer than three retained corners or collinear positions.
///
/// Applied after index, finite-position, and consecutive-coincidence checks,
/// before projection simplicity checks. Wholly collinear faces follow this
/// policy even when their loops backtrack or contain nonconsecutive coincidences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DegenerateFacePolicy {
    /// Return an error instead of triangles.
    #[default]
    Reject,
    /// Emit no triangles and report the original face offset.
    Drop,
}

/// Explicit preparation policy; defaults preserve authored data by rejecting cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrepareParams {
    /// Policy for repeated/coincident consecutive positions.
    pub coincident_corners: CoincidentCornerPolicy,
    /// Policy for nonplanar faces.
    pub warped_faces: WarpedFacePolicy,
    /// Policy for geometrically degenerate faces.
    pub degenerate_faces: DegenerateFacePolicy,
    /// Maximum authored corners per face, checked before allocation or pair scans.
    ///
    /// Default: 1024. Validation and boundary restoration use quadratic work;
    /// shared ear clipping can take cubic work in the worst case. Callers can
    /// lower this limit to bound per-face work on untrusted input.
    pub max_corners_per_face: usize,
}

impl Default for PrepareParams {
    fn default() -> Self {
        Self {
            coincident_corners: CoincidentCornerPolicy::Reject,
            warped_faces: WarpedFacePolicy::Reject,
            degenerate_faces: DegenerateFacePolicy::Reject,
            max_corners_per_face: 1024,
        }
    }
}

/// Offset into the original polygon soup, independent of topology IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceCorner {
    /// Offset into the `face_loops` slice.
    pub face: usize,
    /// Local offset into that original face loop, before cleanup.
    pub corner: usize,
}

/// Triangle provenance suitable for remapping corner attributes and face subsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedTriangle {
    /// Offset into the original `face_loops` slice.
    pub face: usize,
    /// Local offsets into that original face loop, in its authored winding.
    ///
    /// Position indices are `face_loops[face][corners[k]]`. Face-varying UVs
    /// and normals use these corner offsets; material subsets use `face`.
    pub corners: [usize; 3],
}

/// Preparation result. Original positions and face loops are never modified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreparedPolygons {
    /// Nonzero-area triangles referring only to original source corners.
    pub triangles: Vec<PreparedTriangle>,
    /// Corners explicitly discarded by [`CoincidentCornerPolicy::DropConsecutive`].
    pub discarded_corners: Vec<SourceCorner>,
    /// Faces explicitly omitted by [`DegenerateFacePolicy::Drop`].
    pub dropped_faces: Vec<usize>,
    /// Noncoplanar faces accepted by [`WarpedFacePolicy::Project`].
    pub projected_faces: Vec<usize>,
}

/// Failure at one source face. No partial preparation result is returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrepareError {
    /// Offset into the original `face_loops` slice.
    pub face: usize,
    /// Specific input or geometry failure.
    pub kind: PrepareErrorKind,
}

/// Checked preparation failures, with local corner offsets where applicable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareErrorKind {
    /// Authored face size exceeds the configured or `u32` index limit.
    CornerLimit,
    /// A corner references a missing position.
    MissingPosition {
        /// Local authored corner offset.
        corner: usize,
        /// Invalid source position index.
        index: u32,
    },
    /// A referenced position contains a nonfinite coordinate.
    NonFinitePosition {
        /// Local authored corner offset.
        corner: usize,
    },
    /// Consecutive corners occupy exactly the same position.
    CoincidentCorners {
        /// Retained/first corner offset.
        first: usize,
        /// Repeated corner offset.
        second: usize,
    },
    /// Fewer than three retained corners, or all positions are exactly collinear.
    DegenerateFace,
    /// Positions are not exactly coplanar under the reject policy.
    WarpedFace,
    /// Newell projection has zero signed area; this is not proof of degeneracy.
    ZeroProjectedArea,
    /// Projected nonadjacent edges cross or touch, or an adjacent edge doubles back.
    NonSimpleProjection,
    /// The shared planar triangulator could not construct a valid cover.
    Triangulation(TriError),
}

impl core::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "polygon {}: {:?}", self.face, self.kind)
    }
}

impl core::error::Error for PrepareError {}

/// Checks and triangulates indexed polygon soup with explicit geometric policy.
///
/// Reuses the shared exact-sign planar triangulator, retaining distinct
/// collinear boundary corners and authored winding. No vertices are generated,
/// moved, or welded, including between faces. Only referenced positions are
/// validated. After index, finite-position, and consecutive-coincidence checks,
/// the degenerate policy applies before projection simplicity checks. Wholly
/// collinear loops can therefore be dropped even when they backtrack or contain
/// nonconsecutive coincidences. Nondegenerate faces with crossing/touching
/// projections are rejected without a fan fallback. Degenerate dropping never
/// treats a zero signed projected area alone as proof of a zero-area 3D face.
///
/// Three retained corners use exact collinearity checks and emit their authored
/// winding directly. Larger polygons use floating-point Newell and winding
/// sums, which may conservatively reject valid faces at extreme dynamic ranges
/// despite exact-sign local predicates.
///
/// Work and temporary memory are bounded by `max_corners_per_face` per face;
/// total output scales with the input. No tolerance or near-equality cleanup
/// is implicit. Callers choose how to transfer attributes at discarded corners.
///
/// # Example
///
/// ```
/// use exedra_mesh::{PrepareParams, prepare_polygons};
/// let points = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
/// let loops: [&[u32]; 1] = [&[0, 1, 2]];
/// let corner_uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
/// let prepared = prepare_polygons(&points, &loops, &PrepareParams::default())?;
/// let triangle = prepared.triangles[0];
/// let position_indices = triangle.corners.map(|c| loops[triangle.face][c]);
/// let triangle_uvs = triangle.corners.map(|c| corner_uvs[c]);
/// assert_eq!(position_indices, [2, 0, 1]);
/// assert_eq!(triangle_uvs, [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0]]);
/// # Ok::<(), exedra_mesh::PrepareError>(())
/// ```
///
/// # Errors
///
/// Returns the first face failure in source order, including policy rejections,
/// missing/nonfinite positions, budget violations, and triangulation errors.
pub fn prepare_polygons(
    positions: &[[f32; 3]],
    face_loops: &[&[u32]],
    params: &PrepareParams,
) -> Result<PreparedPolygons, PrepareError> {
    let mut result = PreparedPolygons::default();
    for (face, indices) in face_loops.iter().enumerate() {
        prepare_face(positions, indices, face, params, &mut result)
            .map_err(|kind| PrepareError { face, kind })?;
    }
    Ok(result)
}

fn prepare_face(
    positions: &[[f32; 3]],
    indices: &[u32],
    face: usize,
    params: &PrepareParams,
    result: &mut PreparedPolygons,
) -> Result<(), PrepareErrorKind> {
    if indices.len() > params.max_corners_per_face || u32::try_from(indices.len()).is_err() {
        return Err(PrepareErrorKind::CornerLimit);
    }
    let mut points = Vec::<[f64; 3]>::with_capacity(indices.len());
    let mut corners = Vec::with_capacity(indices.len());
    for (corner, &index) in indices.iter().enumerate() {
        let p = positions
            .get(index as usize)
            .ok_or(PrepareErrorKind::MissingPosition { corner, index })?;
        if !p.iter().all(|v| v.is_finite()) {
            return Err(PrepareErrorKind::NonFinitePosition { corner });
        }
        let p = p.map(f64::from);
        if points.last() == Some(&p) {
            discard_corner(
                *corners.last().expect("point has source corner"),
                corner,
                face,
                params,
                result,
            )?;
        } else {
            points.push(p);
            corners.push(corner);
        }
    }
    if points.len() > 1 && points.first() == points.last() {
        discard_corner(
            corners[0],
            *corners.last().expect("nonempty ring"),
            face,
            params,
            result,
        )?;
        points.pop();
        corners.pop();
    }
    let plane = if points.len() >= 3 {
        (2..points.len()).find(|&i| !collinear(points[0], points[1], points[i]))
    } else {
        None
    };
    let Some(apex) = plane else {
        return match params.degenerate_faces {
            DegenerateFacePolicy::Reject => Err(PrepareErrorKind::DegenerateFace),
            DegenerateFacePolicy::Drop => {
                result.dropped_faces.push(face);
                Ok(())
            }
        };
    };
    if points.len() == 3 {
        // Exact noncollinearity already proves a valid planar triangle. Avoid
        // Newell/winding sums, which can lose its area at extreme dynamic ranges.
        // Match the shared ear clipper's deterministic cyclic corner order.
        result.triangles.push(PreparedTriangle {
            face,
            corners: [corners[2], corners[0], corners[1]],
        });
        return Ok(());
    }
    if points
        .iter()
        .any(|&p| orient3d(points[0], points[1], points[apex], p) != Orientation3d::Coplanar)
    {
        match params.warped_faces {
            WarpedFacePolicy::Reject => return Err(PrepareErrorKind::WarpedFace),
            WarpedFacePolicy::Project => result.projected_faces.push(face),
        }
    }
    let mut normal = [0.0_f64; 3];
    // Translate to the first point before summing projected signed areas.
    for i in 1..points.len() - 1 {
        let a = core::array::from_fn::<_, 3, _>(|k| points[i][k] - points[0][k]);
        let b = core::array::from_fn::<_, 3, _>(|k| points[i + 1][k] - points[0][k]);
        for k in 0..3 {
            normal[k] += a[(k + 1) % 3] * b[(k + 2) % 3] - a[(k + 2) % 3] * b[(k + 1) % 3];
        }
    }
    let mut axis = 0;
    for k in 1..3 {
        if normal[k].abs() > normal[axis].abs() {
            axis = k;
        }
    }
    if normal[axis] == 0.0 {
        return Err(PrepareErrorKind::ZeroProjectedArea);
    }
    let (u, v) = if normal[axis] > 0.0 {
        ((axis + 1) % 3, (axis + 2) % 3)
    } else {
        ((axis + 2) % 3, (axis + 1) % 3)
    };
    let projected: Vec<_> = points.iter().map(|p| [p[u], p[v]]).collect();
    if !simple_projection(&projected) {
        return Err(PrepareErrorKind::NonSimpleProjection);
    }
    let cover = triangulate_preserving_boundary(
        &PolygonInput {
            outer: &projected,
            holes: &[],
        },
        &TriParams::ear_clip(),
    )
    .map_err(PrepareErrorKind::Triangulation)?;
    result.triangles.extend(
        cover
            .triangles
            .into_iter()
            .map(|triangle| PreparedTriangle {
                face,
                corners: triangle.map(|i| corners[i as usize]),
            }),
    );
    Ok(())
}

fn discard_corner(
    first: usize,
    second: usize,
    face: usize,
    params: &PrepareParams,
    result: &mut PreparedPolygons,
) -> Result<(), PrepareErrorKind> {
    match params.coincident_corners {
        CoincidentCornerPolicy::Reject => {
            Err(PrepareErrorKind::CoincidentCorners { first, second })
        }
        CoincidentCornerPolicy::DropConsecutive => {
            result.discarded_corners.push(SourceCorner {
                face,
                corner: second,
            });
            Ok(())
        }
    }
}

fn collinear(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> bool {
    (0..3).all(|k| {
        orient2d(
            [a[k], a[(k + 1) % 3]],
            [b[k], b[(k + 1) % 3]],
            [c[k], c[(k + 1) % 3]],
        ) == Orientation::Collinear
    })
}

fn on_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    p[0] >= a[0].min(b[0])
        && p[0] <= a[0].max(b[0])
        && p[1] >= a[1].min(b[1])
        && p[1] <= a[1].max(b[1])
}

fn simple_projection(points: &[[f64; 2]]) -> bool {
    let n = points.len();
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        let previous = points[(i + n - 1) % n];
        if a == b
            || (orient2d(previous, a, b) == Orientation::Collinear && !on_segment(a, previous, b))
        {
            return false;
        }
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let c = points[j];
            let d = points[(j + 1) % n];
            if a[0].max(b[0]) < c[0].min(d[0])
                || c[0].max(d[0]) < a[0].min(b[0])
                || a[1].max(b[1]) < c[1].min(d[1])
                || c[1].max(d[1]) < a[1].min(b[1])
            {
                continue;
            }
            let ab_c = orient2d(a, b, c);
            let ab_d = orient2d(a, b, d);
            let cd_a = orient2d(c, d, a);
            let cd_b = orient2d(c, d, b);
            if (ab_c == Orientation::Collinear && on_segment(c, a, b))
                || (ab_d == Orientation::Collinear && on_segment(d, a, b))
                || (cd_a == Orientation::Collinear && on_segment(a, c, d))
                || (cd_b == Orientation::Collinear && on_segment(b, c, d))
                || (ab_c != Orientation::Collinear
                    && ab_d != Orientation::Collinear
                    && ab_c != ab_d
                    && cd_a != Orientation::Collinear
                    && cd_b != Orientation::Collinear
                    && cd_a != cd_b)
            {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod sponza_tests;
#[cfg(test)]
mod tests;
