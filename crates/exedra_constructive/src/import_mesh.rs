// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Imported mesh transforms and restoration of serialized corner channels.

use crate::tessellate::TessellateError;
use alloc::vec::Vec;
use exedra_math::Placement3;
use exedra_mesh::Mesh;
use exedra_mesh_ops::transform::{TransformError, transform as transform_mesh};

/// Restores sparse corner channels by the destination-vertex phase of each loop.
pub(crate) fn restore_corner_channels(
    mesh: &mut Mesh,
    loops: &[Vec<u32>],
    uvs: &[Vec<Option<[f32; 2]>>],
    uvs1: &[Vec<Option<[f32; 2]>>],
    normals: &[Vec<Option<[f32; 3]>>],
    uv1_rule: Option<exedra_mesh::attributes::Propagation>,
    uv1_default: Option<[f32; 2]>,
) -> Result<(), ()> {
    fn valid<const N: usize>(loops: &[Vec<u32>], channel: &[Vec<Option<[f32; N]>>]) -> bool {
        channel.is_empty()
            || (channel.len() == loops.len()
                && channel
                    .iter()
                    .zip(loops)
                    .all(|(values, face)| values.len() == face.len())
                && channel
                    .iter()
                    .flatten()
                    .flatten()
                    .flatten()
                    .all(|x| x.is_finite()))
    }
    if !valid(loops, uvs)
        || !valid(loops, uvs1)
        || !valid(loops, normals)
        || uv1_default.is_some_and(|uv| !uv.into_iter().all(f32::is_finite))
    {
        return Err(());
    }
    if uvs.is_empty()
        && uvs1.is_empty()
        && normals.is_empty()
        && uv1_rule.is_none()
        && uv1_default.is_none()
    {
        return Ok(());
    }
    let faces: Vec<_> = mesh.faces().collect();
    if faces.len() != loops.len() {
        return Err(());
    }
    if let Some(default) = uv1_default {
        mesh.define_dense_layer(exedra_mesh::attr::CORNER_UV1, default)
            .map_err(|_| ())?;
    }
    let mut corners = Vec::new();
    let mut edit = mesh.edit();
    for (index, face) in faces.into_iter().enumerate() {
        corners.clear();
        corners.extend(edit.mesh().face_loop(face));
        let first = *corners.first().ok_or(())?;
        let vertex = edit.mesh().to_vertex(first).ok_or(())?;
        let phase = loops[index]
            .iter()
            .position(|&v| v == vertex.index())
            .ok_or(())?;
        for (offset, &corner) in corners.iter().enumerate() {
            let position = (phase + offset) % loops[index].len();
            if edit.mesh().to_vertex(corner).ok_or(())?.index() != loops[index][position] {
                return Err(());
            }
            if let Some(uv) = uvs.get(index).and_then(|values| values[position]) {
                exedra_mesh::op::set_corner_uv(&mut edit, corner, uv).map_err(|_| ())?;
            }
            if let Some(uv) = uvs1.get(index).and_then(|values| values[position]) {
                exedra_mesh::op::set_attribute(
                    &mut edit,
                    exedra_mesh::attr::CORNER_UV1,
                    corner,
                    uv,
                )
                .map_err(|_| ())?;
            }
            if let Some(normal) = normals.get(index).and_then(|values| values[position]) {
                exedra_mesh::op::set_corner_normal_override(&mut edit, corner, Some(normal))
                    .map_err(|_| ())?;
            }
        }
    }
    let _: () = edit.finish();
    if let Some(rule) = uv1_rule {
        let key = exedra_mesh::attr::CORNER_UV1;
        if mesh.attrs().layer_kind(key.domain(), key.name()).is_none() {
            mesh.define_sparse_layer(key).map_err(|_| ())?;
        }
        mesh.set_layer_propagation(key, rule).map_err(|_| ())?;
    }
    Ok(())
}

pub(crate) fn transform(source: &Mesh, placement: &Placement3) -> Result<Mesh, TessellateError> {
    transform_mesh(source, placement).map_err(|error| match error {
        TransformError::NonFiniteGeometry => TessellateError::NonFiniteGeometry,
        TransformError::CollapsedGeometry => TessellateError::CollapsedGeometry,
        TransformError::Build(error) => TessellateError::Build(error),
        TransformError::InvalidMesh(errors) => TessellateError::InvalidMesh(errors),
    })
}
