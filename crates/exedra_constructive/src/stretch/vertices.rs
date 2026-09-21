// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Constructive attribution for topology-preserving vertex displacement.

use crate::ir::{Placement3, VertexStretchStep};
use crate::tessellate::TessellatedBody;
use exedra_mesh_ops::stretch::{VertexStretchError, stretch_vertices};

pub(crate) fn stretch_body_vertices(
    source: &TessellatedBody,
    steps: &[VertexStretchStep],
    world: &Placement3,
) -> Result<TessellatedBody, VertexStretchError> {
    source
        .source_map
        .check(&source.mesh)
        .map_err(|_| VertexStretchError::InvalidMesh)?;
    let mesh = stretch_vertices(&source.mesh, steps, world)?;
    Ok(TessellatedBody {
        source_map: source.source_map.repinned(&mesh),
        mesh,
        face_materials: source.face_materials.clone(),
        sweep_checks: None,
        path_sampling: None,
        loft_sampling: None,
        refinement: None,
    })
}

pub(crate) const fn vertex_refusal_code(error: VertexStretchError) -> &'static str {
    match error {
        VertexStretchError::InvalidInput => "eval.stretch_vertices.invalid_input",
        VertexStretchError::InvalidMesh => "eval.stretch_vertices.invalid_mesh",
        VertexStretchError::UnsupportedFace(_) => "eval.stretch_vertices.unsupported_face",
        VertexStretchError::SingularTransform => "eval.stretch_vertices.singular_transform",
        VertexStretchError::NumericLimit => "eval.stretch_vertices.numeric_limit",
        VertexStretchError::DegenerateTriangle(_) => "eval.stretch_vertices.degenerate_triangle",
        _ => "eval.stretch_vertices.refused",
    }
}

#[cfg(test)]
mod tests;
