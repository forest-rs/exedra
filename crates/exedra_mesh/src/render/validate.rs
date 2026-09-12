// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_triangulate::predicates::{Orientation, orient2d};

use super::TriMesh;

/// Invalid geometry in a render triangle buffer.
///
/// Indices here refer to the emitted buffer, not source mesh face or vertex IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TriMeshGeometryError {
    /// The index buffer does not contain complete triples.
    IncompleteTriangle {
        /// Number of indices in the buffer.
        index_count: usize,
    },
    /// A render position contains a NaN or infinity.
    NonFinitePosition {
        /// Index of the position in the vertex buffer.
        vertex: usize,
    },
    /// An index refers beyond the position buffer.
    InvalidIndex {
        /// Offset in the index buffer.
        offset: usize,
        /// The invalid vertex index stored there.
        vertex: u32,
    },
    /// The three emitted positions are exactly collinear or coincident.
    DegenerateTriangle {
        /// Zero-based triangle index (index-buffer offset divided by three).
        triangle: usize,
    },
}

impl core::fmt::Display for TriMeshGeometryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IncompleteTriangle { index_count } => {
                write!(f, "incomplete triangle in {index_count} indices")
            }
            Self::NonFinitePosition { vertex } => {
                write!(f, "nonfinite render position at vertex {vertex}")
            }
            Self::InvalidIndex { offset, vertex } => write!(
                f,
                "render index {offset} references missing vertex {vertex}"
            ),
            Self::DegenerateTriangle { triangle } => {
                write!(f, "render triangle {triangle} has zero area")
            }
        }
    }
}

impl core::error::Error for TriMeshGeometryError {}

impl TriMesh {
    /// Checks that the emitted positions and triangles are geometrically usable.
    ///
    /// Validates complete index triples, finite positions (including unused
    /// vertices), in-bounds indices and nonzero triangle area. Collinearity uses
    /// exact predicates on the stored `f32` positions promoted to `f64`: tiny
    /// valid triangles pass, while geometry collapsed at render precision fails.
    /// Work is linear in the number of positions and triangles, with no allocation.
    ///
    /// Empty geometry is valid. This check does not establish closure, manifoldness,
    /// consistent winding, absence of self-intersections or valid UVs/normals.
    /// It does not mutate or repair the buffers and does not run automatically
    /// during extraction. Callers can apply it before export or to generated fixtures.
    ///
    /// # Errors
    ///
    /// Reports incomplete triples first, then the first nonfinite position, then
    /// the first invalid index or degenerate triangle in triangle order.
    pub fn validate_geometry(&self) -> Result<(), TriMeshGeometryError> {
        if !self.indices.len().is_multiple_of(3) {
            return Err(TriMeshGeometryError::IncompleteTriangle {
                index_count: self.indices.len(),
            });
        }
        for (vertex, position) in self.positions.iter().enumerate() {
            if !position.iter().all(|v| v.is_finite()) {
                return Err(TriMeshGeometryError::NonFinitePosition { vertex });
            }
        }
        for (triangle, indices) in self.indices.chunks_exact(3).enumerate() {
            let mut points = [[0.0; 3]; 3];
            for (corner, &vertex) in indices.iter().enumerate() {
                points[corner] = self
                    .positions
                    .get(vertex as usize)
                    .ok_or(TriMeshGeometryError::InvalidIndex {
                        offset: triangle * 3 + corner,
                        vertex,
                    })?
                    .map(f64::from);
            }
            let collinear = [(0, 1), (1, 2), (2, 0)].into_iter().all(|(a, b)| {
                let [p, q, r] = points.map(|p| [p[a], p[b]]);
                orient2d(p, q, r) == Orientation::Collinear
            });
            if collinear {
                return Err(TriMeshGeometryError::DegenerateTriangle { triangle });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn triangle(positions: [[f32; 3]; 3]) -> TriMesh {
        TriMesh {
            indices: vec![0, 1, 2],
            positions: positions.to_vec(),
            ..TriMesh::default()
        }
    }

    #[test]
    fn accepts_empty_and_small_valid_geometry_in_every_plane_and_winding() {
        assert_eq!(TriMesh::default().validate_geometry(), Ok(()));
        for scale in [f32::from_bits(1), 1e-20, 1.0, f32::MAX] {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let mut points = [[0.0; 3]; 3];
                points[1][a] = scale;
                points[2][b] = scale;
                let mut mesh = triangle(points);
                assert_eq!(mesh.validate_geometry(), Ok(()));
                mesh.indices.reverse();
                assert_eq!(mesh.validate_geometry(), Ok(()));
            }
        }
    }

    #[test]
    fn reports_malformed_buffers_without_panicking() {
        let mut mesh = triangle([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        mesh.indices.push(0);
        assert_eq!(
            mesh.validate_geometry(),
            Err(TriMeshGeometryError::IncompleteTriangle { index_count: 4 })
        );
        mesh.indices.pop();
        mesh.indices[2] = u32::MAX;
        assert_eq!(
            mesh.validate_geometry(),
            Err(TriMeshGeometryError::InvalidIndex {
                offset: 2,
                vertex: u32::MAX
            })
        );
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            mesh.positions.push([value, 0.0, 0.0]);
            assert_eq!(
                mesh.validate_geometry(),
                Err(TriMeshGeometryError::NonFinitePosition { vertex: 3 })
            );
            mesh.positions.pop();
        }
    }

    #[test]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "exercise collapse at render precision"
    )]
    fn distinguishes_zero_area_from_tiny_area_at_render_precision() {
        let mut mesh = triangle([[0.0; 3], [1.0, 2.0, 3.0], [2.0, 4.0, 6.0]]);
        assert_eq!(
            mesh.validate_geometry(),
            Err(TriMeshGeometryError::DegenerateTriangle { triangle: 0 })
        );
        mesh.positions[2][2] = 6.0_f32.next_up();
        assert_eq!(mesh.validate_geometry(), Ok(()));
        mesh.indices.extend_from_slice(&[0, 1, 1]);
        assert_eq!(
            mesh.validate_geometry(),
            Err(TriMeshGeometryError::DegenerateTriangle { triangle: 1 })
        );

        // A valid f64 triangle can lose its entire altitude on conversion to f32.
        let points = [[1.0_f64; 3], [2.0, 1.0, 1.0], [1.0, 1.0 + 1e-10, 1.0]];
        assert_eq!(
            triangle(points.map(|p| p.map(|v| v as f32))).validate_geometry(),
            Err(TriMeshGeometryError::DegenerateTriangle { triangle: 0 })
        );
    }
}
