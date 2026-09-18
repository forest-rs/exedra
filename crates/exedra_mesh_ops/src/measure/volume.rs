// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::Sum;
use exedra_math::{Placement3, cross, dot, sub};
use exedra_mesh::TriMesh;

/// Algebraic volume of the supplied triangle winding, in cubed coordinate units.
///
/// This is a measurement, not a solid-validity certificate. Enclosed volume is
/// meaningful for closed, consistently oriented boundaries. Positive total
/// volume does not prove that every component or triangle faces outward; inward
/// cavity shells correctly subtract volume. Open surfaces depend on the reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignedVolume {
    /// Signed sum of tetrahedron volumes relative to [`Self::reference`].
    pub value: f64,
    /// First referenced position, or `None` for an empty triangle list.
    /// Translating to this point reduces cancellation for distant small bodies.
    pub reference: Option<[f64; 3]>,
    /// Number of indexed triangles examined. No topology or tessellation work.
    pub triangles_examined: usize,
}

impl SignedVolume {
    /// Transports a measurement through an affine placement in constant time.
    ///
    /// Multiplies the signed volume by the linear determinant and transforms the
    /// reference point. The original triangle winding is preserved: a reflection
    /// changes the sign. This does not repair mesh winding or account for later
    /// rounding by a renderer. No triangles are revisited.
    ///
    /// # Errors
    /// Refuses nonfinite/singular placements and nonrepresentable results.
    pub fn transformed(self, placement: &Placement3) -> Result<Self, VolumeError> {
        if !placement.is_finite() {
            return Err(VolumeError::NonFinitePlacement);
        }
        let determinant = placement.linear_determinant();
        if !determinant.is_finite() {
            return Err(VolumeError::NumericLimit);
        }
        if determinant == 0.0 {
            return Err(VolumeError::SingularPlacement);
        }
        let value = self.value * determinant;
        if !value.is_finite() || (self.value != 0.0 && (value == 0.0 || value.is_subnormal())) {
            return Err(VolumeError::NumericLimit);
        }
        let reference = self
            .reference
            .map(|p| placement.rows.map(|r| dot([r[0], r[1], r[2]], p) + r[3]));
        if reference.iter().flatten().any(|v| !v.is_finite()) {
            return Err(VolumeError::NumericLimit);
        }
        Ok(Self {
            value,
            reference,
            ..self
        })
    }
}

/// Refusal to measure indexed triangles or transport their signed volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VolumeError {
    /// The index buffer does not contain whole triangles.
    IncompleteTriangle {
        /// Number of indices supplied.
        indices: usize,
    },
    /// A triangle references a position outside the position buffer.
    InvalidIndex {
        /// Triangle index in the supplied index buffer.
        triangle: usize,
        /// Invalid vertex index.
        vertex: u32,
    },
    /// A referenced position has a NaN or infinite coordinate.
    NonFinitePosition {
        /// Index of that position.
        vertex: u32,
    },
    /// The placement contains nonfinite coordinates.
    NonFinitePlacement,
    /// A placement has a zero determinant, including underflow to zero.
    SingularPlacement,
    /// Arithmetic cannot produce a finite, representable result.
    NumericLimit,
}

impl core::fmt::Display for VolumeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IncompleteTriangle { indices } => {
                write!(f, "{indices} indices do not form whole triangles")
            }
            Self::InvalidIndex { triangle, vertex } => {
                write!(f, "triangle {triangle} references absent vertex {vertex}")
            }
            Self::NonFinitePosition { vertex } => write!(f, "vertex {vertex} is not finite"),
            Self::NonFinitePlacement => f.write_str("volume placement is not finite"),
            Self::SingularPlacement => f.write_str("volume placement is singular"),
            Self::NumericLimit => f.write_str("signed volume exceeds numeric limits"),
        }
    }
}
impl core::error::Error for VolumeError {}

/// Measures the algebraic signed volume of indexed render triangles.
///
/// Uses f64 arithmetic, a translated reference and compensated summation. This
/// is not an exact predicate or a certified numerical error bound. Empty input
/// returns zero. Only referenced positions are inspected; normals, regions and
/// other render attributes are irrelevant. Degenerate triangles contribute zero.
/// No closure, manifoldness or self-intersection check is implied.
///
/// # Errors
/// Identifies incomplete triangles, bad indices and nonfinite referenced positions.
/// Nonfinite arithmetic is refused without returning a partial measurement.
pub fn signed_volume(mesh: &TriMesh) -> Result<SignedVolume, VolumeError> {
    let (triangles, remainder) = mesh.indices.as_chunks::<3>();
    if !remainder.is_empty() {
        return Err(VolumeError::IncompleteTriangle {
            indices: mesh.indices.len(),
        });
    }
    let mut sum = Sum::default();
    let mut reference = None;
    for (triangle, indices) in triangles.iter().enumerate() {
        let mut points = [[0.0; 3]; 3];
        for (point, &vertex) in points.iter_mut().zip(indices) {
            let position = mesh
                .positions
                .get(vertex as usize)
                .ok_or(VolumeError::InvalidIndex { triangle, vertex })?;
            if position.iter().any(|v| !v.is_finite()) {
                return Err(VolumeError::NonFinitePosition { vertex });
            }
            *point = position.map(f64::from);
        }
        let origin = *reference.get_or_insert(points[0]);
        let [a, b, c] = points.map(|p| sub(p, origin));
        sum.add(dot(a, cross(b, c)) / 6.0);
    }
    let value = sum.total();
    if !value.is_finite() {
        return Err(VolumeError::NumericLimit);
    }
    Ok(SignedVolume {
        value,
        reference,
        triangles_examined: triangles.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use exedra_math::narrow;

    fn tetrahedron() -> TriMesh {
        TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 3.0, 0.0],
                [0.0, 0.0, 4.0],
            ],
            indices: vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3],
            ..TriMesh::default()
        }
    }

    #[test]
    fn volume_retains_winding_and_survives_translation() {
        let mut mesh = tetrahedron();
        assert_eq!(signed_volume(&mesh).unwrap().value, 4.0);
        for p in &mut mesh.positions {
            for coordinate in p {
                *coordinate += 1_000_000.0;
            }
        }
        let measured = signed_volume(&mesh).unwrap();
        assert_eq!(measured.value, 4.0);
        assert_eq!(measured.triangles_examined, 4);
        for triangle in mesh.indices.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
        assert_eq!(signed_volume(&mesh).unwrap().value, -4.0);
        let placement =
            Placement3::from_axes([-2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0], [1.0; 3]);
        assert_eq!(measured.transformed(&placement).unwrap().value, -96.0);
    }

    #[test]
    fn cavity_subtracts_and_affine_transport_matches_transformed_triangles() {
        let mut mesh = tetrahedron();
        let cavity = tetrahedron();
        mesh.positions
            .extend(cavity.positions.iter().map(|p| p.map(|v| v * 0.25 + 0.25)));
        for triangle in cavity.indices.chunks_exact(3) {
            mesh.indices
                .extend([triangle[2] + 4, triangle[1] + 4, triangle[0] + 4]);
        }
        // Unreferenced positions and degenerate triangles add no volume.
        mesh.positions.push([f32::NAN; 3]);
        mesh.indices.extend([0, 0, 0]);
        let measured = signed_volume(&mesh).unwrap();
        assert_eq!(measured.value, 4.0 - 4.0 / 64.0);
        let placement = Placement3::from_axes(
            [2.0, 0.0, 0.0],
            [1.0, 3.0, 0.0],
            [0.0, 1.0, -4.0],
            [8.0, 16.0, 32.0],
        );
        for p in &mut mesh.positions {
            let original = p.map(f64::from);
            *p = narrow(
                placement
                    .rows
                    .map(|r| dot([r[0], r[1], r[2]], original) + r[3]),
            );
        }
        assert_eq!(
            measured.transformed(&placement).unwrap(),
            signed_volume(&mesh).unwrap()
        );
    }

    #[test]
    fn malformed_buffers_and_placements_are_identified() {
        assert_eq!(signed_volume(&TriMesh::default()).unwrap().value, 0.0);
        let mut mesh = tetrahedron();
        mesh.indices.push(0);
        assert_eq!(
            signed_volume(&mesh),
            Err(VolumeError::IncompleteTriangle { indices: 13 })
        );
        mesh.indices.pop();
        mesh.indices[5] = 99;
        assert_eq!(
            signed_volume(&mesh),
            Err(VolumeError::InvalidIndex {
                triangle: 1,
                vertex: 99
            })
        );
        mesh.indices[5] = 3;
        mesh.positions[3][0] = f32::NAN;
        assert_eq!(
            signed_volume(&mesh),
            Err(VolumeError::NonFinitePosition { vertex: 3 })
        );
        let measured = signed_volume(&tetrahedron()).unwrap();
        let singular = Placement3::from_axes([0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]);
        assert_eq!(
            measured.transformed(&singular),
            Err(VolumeError::SingularPlacement)
        );
        assert_eq!(
            measured.transformed(&Placement3::translate(f64::NAN, 0.0, 0.0)),
            Err(VolumeError::NonFinitePlacement)
        );
        for axes in [
            [[f64::MAX, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [[1e-200, 0.0, 0.0], [0.0, 1e-100, 0.0], [0.0, 0.0, 1e-10]],
        ] {
            let placement = Placement3::from_axes(axes[0], axes[1], axes[2], [0.0; 3]);
            assert_eq!(
                measured.transformed(&placement),
                Err(VolumeError::NumericLimit)
            );
        }
    }
}
