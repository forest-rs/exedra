// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::backend::{Backend, Result};
use exedra_scene::{Time, composition::*, schemas};
use serde_json::json;
use wasm_bindgen::prelude::*;

/// Owned transfer delta. Empty buffers mean no update, never empty geometry.
#[derive(Debug)]
#[wasm_bindgen]
pub struct ScenePacket {
    metadata: String,
    positions: Vec<f32>,
    normals: Vec<f32>,
    matrices: Vec<f64>,
}
#[wasm_bindgen]
impl ScenePacket {
    /// Structured identities, changed instance paths and exact transfer counts.
    pub fn metadata(&self) -> String {
        self.metadata.clone()
    }
    /// Moves changed composed polygon positions once; no-op returns an empty array.
    pub fn take_positions(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.positions)
    }
    /// Moves face-varying composed normals once, in the same expanded corner order.
    pub fn take_normals(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.normals)
    }
    /// Moves f64 instance transforms in native row-vector order.
    /// The renderer's f32 occurrence placement is a separate approximation.
    pub fn take_matrices(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.matrices)
    }
}

pub(crate) fn composed_arrays(b: &Backend<'_>) -> Result<[TypedArray; 5]> {
    let source = b
        .state
        .snapshot
        .as_ref()
        .and_then(|s| s.sources.first())
        .ok_or("sync before extracting a scene")?;
    Ok(mesh_values_at(b, source.output)?.map(|(array, _)| array))
}
pub(crate) fn mesh_values_at(
    b: &Backend<'_>,
    path: PathId,
) -> Result<[(TypedArray, Option<Provenance>); 5]> {
    let prim = b
        .live
        .stage()
        .view(b.store)
        .prim(path)
        .ok_or("published mesh is missing")?;
    let get = |name| -> Result<(TypedArray, Option<Provenance>)> {
        let resolved = prim
            .attribute(name)
            .and_then(|a| a.get(Time::Default))
            .ok_or("missing mesh attribute")?;
        match resolved.value {
            Value::TypedArray(array) => Ok((array, resolved.provenance)),
            _ => Err("mesh extraction requires typed native arrays".into()),
        }
    };
    Ok([
        get("points")?,
        get("faceVertexCounts")?,
        get("faceVertexIndices")?,
        get("primvars:normals")?,
        get(exedra_scene::NATIVE_FACE)?,
    ])
}

pub(crate) fn producer_output(b: &Backend<'_>, arrays: &[TypedArray; 5]) -> bool {
    let Some(source) = b.state.snapshot.as_ref().and_then(|s| s.sources.first()) else {
        return false;
    };
    let Some(layer) = b.store.layer(b.state.layers.generated) else {
        return false;
    };
    ["points","faceVertexCounts","faceVertexIndices","primvars:normals",exedra_scene::NATIVE_FACE]
        .iter().zip(arrays).all(|(name,array)| {
            let Some(token) = b.store.tokens().lookup(name) else { return false; };
            let site = SpecPath::from_property_path(PropertyPath::new(source.output,token));
            matches!(layer.property_at(&site).and_then(|s|s.default.as_ref()),Some(Value::TypedArray(owned)) if owned.shares_storage(array))
        })
}

pub(crate) fn pack(b: &mut Backend<'_>, realm: &str) -> Result<ScenePacket> {
    b.live.synchronize(b.store);
    let arrays = composed_arrays(b)?;
    let (revision, prototype_checks, instance_checks) = crate::correspondence::validate_all(b)?;
    let changed = b
        .state
        .last_mesh
        .as_ref()
        .is_none_or(|old| !old.iter().zip(&arrays).all(|(a, b)| a.shares_storage(b)));
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut display_abs = if changed {
        [0.0_f64; 3]
    } else {
        b.state.display_abs
    };
    let [
        TypedArray::Vec3f(points),
        TypedArray::Int(counts),
        TypedArray::Int(indices),
        TypedArray::Vec3f(normal_values),
        TypedArray::UInt(faces),
    ] = &arrays
    else {
        return Err("unsupported composed mesh attribute types".into());
    };
    if changed
        && (counts.iter().any(|&count| count != 3)
            || counts.len().checked_mul(3) != Some(indices.len())
            || normal_values.len() != indices.len()
            || faces.len() != counts.len())
    {
        return Err(
            "scene demo renderer requires triangles, face-varying normals and uniform native IDs"
                .into(),
        );
    }
    if changed {
        positions.reserve(indices.len() * 3);
        normals.reserve(normal_values.len() * 3);
        for (&index, normal) in indices.iter().zip(normal_values.iter()) {
            let point = usize::try_from(index)
                .ok()
                .and_then(|i| points.get(i))
                .ok_or("invalid composed mesh index")?;
            if !point.iter().chain(normal).all(|v| v.is_finite()) {
                return Err("nonfinite composed display data".into());
            }
            positions.extend_from_slice(point);
            normals.extend_from_slice(normal);
            for (bound, coordinate) in display_abs.iter_mut().zip(point) {
                *bound = bound.max(f64::from(*coordinate).abs());
            }
        }
    }
    let scene = schemas::Scene::new(b.live.stage(), b.store);
    let mut transforms = schemas::XformCache::new(Time::Default);
    let mut matrices = Vec::with_capacity(b.state.pending_placements.len() * 16);
    let mut paths = Vec::with_capacity(b.state.pending_placements.len());
    let mut updates = Vec::with_capacity(b.state.pending_placements.len());
    if changed {
        for matrix in b.state.last_transforms.values() {
            validate_gpu_placement(matrix, display_abs)?;
        }
    }
    for &path in &b.state.pending_placements {
        let prim = b
            .live
            .stage()
            .view(b.store)
            .prim(path)
            .ok_or("instance disappeared")?;
        if !prim.is_instance() {
            return Err("placement is no longer a native instance".into());
        }
        let matrix = transforms
            .local_to_world(&scene, path)
            .ok_or("invalid instance transform")?;
        if !matrix.iter().flatten().all(|v| v.is_finite()) {
            return Err("nonfinite instance transform".into());
        }
        validate_gpu_placement(&matrix, display_abs)?;
        if b.state.reset_instances || b.state.last_transforms.get(&path) != Some(&matrix) {
            matrices.extend(matrix.into_iter().flatten());
            paths.push(b.store.paths().display(path, b.store.tokens()));
            updates.push((path, matrix));
        }
    }
    let snapshot = b.state.snapshot.as_ref().expect("checked composed source");
    let metadata = json!({
        "geometryChanged":changed,"source":snapshot.sources[0].source,"sourceScope":format!("{realm}/source/{}",b.state.source_revision),"instances":paths,"resetInstances":b.state.reset_instances,
        "triangles":counts.len(),"nativePicking":producer_output(b,&arrays),
        "geometryBytes":(positions.len()+normals.len())*size_of::<f32>(),
        "transformBytes":matrices.len()*size_of::<f64>(),
        "transformQueries":b.state.pending_placements.len(),
        "prototypeChecks":prototype_checks,"instanceChecks":instance_checks,
        "instanceCount":b.state.placements.len(),"matrixConvention":"row-vector",
        "accuracySpace":"source-local","occurrencePrecision":"renderer f32, separately from source-local geometry accuracy"
    }).to_string();
    b.state.last_mesh = Some(arrays);
    b.state.verified_instances = Some((revision, b.state.source_revision));
    b.state.display_abs = display_abs;
    if b.state.reset_instances {
        b.state
            .last_transforms
            .retain(|path, _| b.state.placement_set.contains(path));
    }
    b.state.last_transforms.extend(updates);
    b.state.pending_placements.clear();
    b.state.reset_instances = false;
    Ok(ScenePacket {
        metadata,
        positions,
        normals,
        matrices,
    })
}

fn validate_gpu_placement(matrix: &[[f64; 4]; 4], display_abs: [f64; 3]) -> Result<()> {
    // Leave a rounding margin for matrix narrowing and the GPU dot products.
    let limit = f64::from(f32::MAX) * (1.0 - 16.0 * f64::from(f32::EPSILON));
    if matrix
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > limit)
    {
        return Err("native instance matrix exceeds the renderer's f32 range".into());
    }
    // A conservative bound for every emitted point, using native row-vector
    // order. Rendering precision is separate from source-local realization.
    for (axis, translation) in matrix[3][..3].iter().enumerate() {
        let bound = translation.abs()
            + (0..3)
                .map(|i| display_abs[i] * matrix[i][axis].abs())
                .sum::<f64>();
        if !bound.is_finite() || bound > limit {
            return Err("placed geometry exceeds the renderer's f32 range".into());
        }
    }
    Ok(())
}
