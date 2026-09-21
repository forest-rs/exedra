// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simultaneous plane-selected displacement without rebuilding topology.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use exedra_math::{Placement3, Plane3};
use exedra_mesh::{FaceId, Mesh};
use exedra_triangulate::predicates::{Orientation, orient2d};

use super::{StretchError, WorldStretch};

/// One displacement selected by an oriented plane in operation-local space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct VertexStretchStep {
    /// Vertices strictly in the positive half-space receive this step.
    /// The plane is normalized before use; on-plane vertices do not move.
    pub plane: Plane3,
    /// Signed displacement along the normalized local plane normal.
    pub length: f64,
}

/// A vertex stretch refusal. The input mesh is never mutated.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VertexStretchError {
    /// No steps, a nonfinite value, or an invalid plane.
    InvalidInput,
    /// Invalid input topology or nonfinite input positions.
    InvalidMesh,
    /// An active operation requires triangle faces.
    UnsupportedFace(FaceId),
    /// The placement cannot transport the selection planes.
    SingularTransform,
    /// A transformed plane, displacement or output position is unrepresentable.
    NumericLimit,
    /// A triangle has zero area at the output's stored precision.
    DegenerateTriangle(FaceId),
}

impl core::fmt::Display for VertexStretchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("invalid vertex stretch steps or placement"),
            Self::InvalidMesh => f.write_str("invalid vertex stretch source mesh"),
            Self::UnsupportedFace(face) => {
                write!(f, "vertex stretch requires a triangle at {face:?}")
            }
            Self::SingularTransform => f.write_str("singular vertex stretch placement"),
            Self::NumericLimit => f.write_str("vertex stretch exceeds numeric limits"),
            Self::DegenerateTriangle(face) => {
                write!(f, "vertex stretch collapses triangle {face:?}")
            }
        }
    }
}
impl core::error::Error for VertexStretchError {}

/// Displaces triangle-mesh vertices selected by planes, without cutting faces.
///
/// Each step classifies the original positions; displacements accumulate in
/// supplied order in f64 and narrow once to f32. `placement` maps the steps'
/// local frame into mesh coordinates. Planes use inverse-transpose transport,
/// while displacement uses the forward linear map, including reflections.
/// Negative lengths move vertices backward; they do not remove a slab.
///
/// Open meshes are accepted. Topology, IDs, UVs, regions and other attributes
/// remain unchanged, except authored corner normals on faces receiving unequal
/// displacements are cleared for subsequent derivation. Rigidly translated
/// faces retain their overrides. Smoothing follows existing connectivity and
/// sharpness; authored smoothing across disconnected seams is not reconstructed.
///
/// All-zero steps return an unchanged clone, including polygon faces. Active
/// operations require triangles and reject zero-area output using exact
/// predicates on stored positions. This does not certify orientation or freedom
/// from self-intersection. No UV-density adjustment or geometric repair occurs.
///
/// # Errors
/// Returns a typed refusal for invalid parameters, topology, unsupported faces,
/// singular transport, numeric overflow or collapsed output. No partial result
/// is returned and `source` is never modified.
pub fn stretch_vertices(
    source: &Mesh,
    steps: &[VertexStretchStep],
    placement: &Placement3,
) -> Result<Mesh, VertexStretchError> {
    if steps.is_empty()
        || !placement.is_finite()
        || steps
            .iter()
            .any(|step| !step.length.is_finite() || step.plane.normalized().is_none())
    {
        return Err(VertexStretchError::InvalidInput);
    }
    if !source.validate_deep().is_empty()
        || source.vertices().any(|v| {
            source
                .vertex_position(v)
                .is_none_or(|p| p.iter().any(|x| !x.is_finite()))
        })
    {
        return Err(VertexStretchError::InvalidMesh);
    }
    if steps.iter().all(|step| step.length == 0.0) {
        return Ok(source.clone());
    }
    let steps = steps
        .iter()
        .filter(|step| step.length != 0.0)
        .map(|step| {
            WorldStretch::prepare(&step.plane, step.length, placement, false).map_err(|error| {
                match error {
                    StretchError::SingularTransform => VertexStretchError::SingularTransform,
                    StretchError::InvalidInput => VertexStretchError::InvalidInput,
                    _ => VertexStretchError::NumericLimit,
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut positions = BTreeMap::new();
    for vertex in source.vertices() {
        let original = source
            .vertex_position(vertex)
            .expect("validated position")
            .map(f64::from);
        let mut displacement = [0.0; 3];
        for step in &steps {
            let side = step.signed(original);
            if !side.is_finite() {
                return Err(VertexStretchError::NumericLimit);
            }
            if side > 0.0 {
                for (sum, delta) in displacement.iter_mut().zip(step.displacement) {
                    *sum += delta;
                }
            }
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "checked f64-to-f32 mesh boundary"
        )]
        let moved = core::array::from_fn(|i| (original[i] + displacement[i]) as f32);
        if displacement.iter().any(|x| !x.is_finite()) || moved.iter().any(|x| !x.is_finite()) {
            return Err(VertexStretchError::NumericLimit);
        }
        positions.insert(vertex, (moved, displacement));
    }
    let mut clear_normals = Vec::new();
    for face in source.faces() {
        let corners = source.face_loop(face).collect::<Vec<_>>();
        let [a, b, c] = corners.as_slice() else {
            return Err(VertexStretchError::UnsupportedFace(face));
        };
        let values = [a, b, c]
            .map(|corner| positions[&source.to_vertex(*corner).expect("validated corner")]);
        let points = values.map(|(position, _)| position.map(f64::from));
        if [(0, 1), (1, 2), (2, 0)].into_iter().all(|(x, y)| {
            let [a, b, c] = points.map(|p| [p[x], p[y]]);
            orient2d(a, b, c) == Orientation::Collinear
        }) {
            return Err(VertexStretchError::DegenerateTriangle(face));
        }
        if values[0].1 != values[1].1 || values[0].1 != values[2].1 {
            clear_normals.extend(corners);
        }
    }
    let mut mesh = source.clone();
    let mut edit = mesh.edit();
    for (vertex, (position, _)) in positions {
        if edit.mesh().vertex_position(vertex) != Some(&position) {
            exedra_mesh::op::set_vertex_position(&mut edit, vertex, position)
                .map_err(|_| VertexStretchError::InvalidMesh)?;
        }
    }
    for corner in clear_normals {
        exedra_mesh::op::set_corner_normal_override(&mut edit, corner, None)
            .map_err(|_| VertexStretchError::InvalidMesh)?;
    }
    let _: () = edit.finish();
    Ok(mesh)
}

#[cfg(test)]
mod tests;
