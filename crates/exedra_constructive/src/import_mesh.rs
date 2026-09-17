// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Recipe error mapping for the shared affine mesh transform.

use crate::tessellate::TessellateError;
use exedra_math::Placement3;
use exedra_mesh::Mesh;
use exedra_mesh_ops::transform::{TransformError, transform as transform_mesh};

pub(crate) fn transform(source: &Mesh, placement: &Placement3) -> Result<Mesh, TessellateError> {
    transform_mesh(source, placement).map_err(|error| match error {
        TransformError::NonFiniteGeometry => TessellateError::NonFiniteGeometry,
        TransformError::CollapsedGeometry => TessellateError::CollapsedGeometry,
        TransformError::Build(error) => TessellateError::Build(error),
        TransformError::InvalidMesh(errors) => TessellateError::InvalidMesh(errors),
    })
}
