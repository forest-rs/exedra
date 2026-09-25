// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simultaneous plane-selected displacement without rebuilding topology.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use exedra_math::{Placement3, Plane3};
use exedra_mesh::{FaceId, Mesh};
use exedra_triangulate::predicates::{Orientation, Orientation3d, orient2d, plane_side};

use super::inverse3;

/// One displacement selected by an oriented plane in operation-local space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct VertexStretchStep {
    /// Vertices strictly in the positive half-space receive this step.
    /// Selection uses the authored equation; on-plane vertices do not move.
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
/// local frame into mesh coordinates. Selection uses an exact sign predicate
/// on the unnormalized plane coefficients; normalization affects displacement
/// only. Nonidentity placement transports the plane in f64 by inverse transpose
/// and the displacement by the forward linear map, including reflections.
/// The sign is exact for those transported coefficient bits and the supplied
/// mesh positions; it cannot recover coordinates lost to earlier rounding.
/// Evaluate locally before placement when original boundary ownership matters.
/// Negative lengths move vertices backward; they do not remove a slab.
///
/// Open meshes are accepted. Topology, IDs, UVs, regions and other attributes
/// remain unchanged, except authored corner normals on faces receiving unequal
/// *stored* movements are cleared for subsequent derivation. Unchanged and
/// rigidly translated faces retain their overrides. Movements are compared as
/// exact differences of stored positions, including wide exponent spans.
/// This face-local policy does not infer authored smoothing neighborhoods; a
/// retained override can differ from a derived normal on its changed neighbor.
/// Choose full derived-normal extraction for consistent geometric smoothing.
/// Authored smoothing across disconnected seams is not reconstructed.
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
        .map(|step| PreparedStep::new(step, placement))
        .collect::<Result<Vec<_>, _>>()?;
    let mut positions = BTreeMap::new();
    for vertex in source.vertices() {
        let original = source
            .vertex_position(vertex)
            .expect("validated position")
            .map(f64::from);
        let mut displacement = [0.0; 3];
        for step in &steps {
            let side = plane_side(step.plane.normal, step.plane.distance, original)
                .ok_or(VertexStretchError::NumericLimit)?;
            if side == Orientation3d::Above {
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
        let stored_movement =
            core::array::from_fn::<_, 3, _>(|i| exact_difference(f64::from(moved[i]), original[i]));
        positions.insert(vertex, (moved, stored_movement));
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

// A sum and its residual distinguish actual f32 movements even when their
// exponent span exceeds f64's significand. Finite f32 differences cannot
// overflow or underflow f64 intermediates.
fn exact_difference(a: f64, b: f64) -> (f64, f64) {
    let sum = a - b;
    let bv = sum - a;
    let av = sum - bv;
    (sum, (a - av) + (-b - bv))
}

struct PreparedStep {
    plane: Plane3,
    displacement: [f64; 3],
}
impl PreparedStep {
    fn new(step: &VertexStretchStep, placement: &Placement3) -> Result<Self, VertexStretchError> {
        let (unit, _) = step
            .plane
            .normalized()
            .ok_or(VertexStretchError::InvalidInput)?;
        let linear = placement.rows.map(|r| [r[0], r[1], r[2]]);
        let inverse = inverse3(linear).ok_or(VertexStretchError::SingularTransform)?;
        // Do not normalize the selection equation. At identity the exact
        // predicate sees the original coefficients, including oblique planes.
        let normal = core::array::from_fn(|i| {
            inverse[0][i] * step.plane.normal[0]
                + inverse[1][i] * step.plane.normal[1]
                + inverse[2][i] * step.plane.normal[2]
        });
        let distance = step.plane.distance
            + normal[0] * placement.rows[0][3]
            + normal[1] * placement.rows[1][3]
            + normal[2] * placement.rows[2][3];
        let local = unit.map(|v| v * step.length);
        let displacement = linear.map(|r| r[0] * local[0] + r[1] * local[1] + r[2] * local[2]);
        if !distance.is_finite()
            || normal.iter().chain(&displacement).any(|v| !v.is_finite())
            || normal.iter().all(|v| *v == 0.0)
        {
            return Err(VertexStretchError::NumericLimit);
        }
        Ok(Self {
            plane: Plane3 { normal, distance },
            displacement,
        })
    }
}

#[cfg(test)]
mod tests;
