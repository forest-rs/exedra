// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::SceneErrorKind;
use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use exedra_compile::{CompilePolicy, NativeRealization, NativeSource};
use exedra_mesh::{CornerId, Mesh, NormalParams, NormalsSource, UvSource, attr};
use layerstack::{PropertyType, TypedArray, Value};
use layerstack_schemas::{GeneratedMesh, MeshPrimvar, ValidatedMesh};

#[derive(Debug)]
pub(crate) struct Polygon {
    pub(crate) mesh: ValidatedMesh,
    pub(crate) extent: Arc<Vec<[f32; 3]>>,
    pub(crate) payload_bytes: usize,
    pub(crate) points: usize,
}
impl Polygon {
    pub(crate) fn new(
        mesh: &Mesh,
        policy: &CompilePolicy,
        native: Option<&NativeRealization>,
        source: Option<&NativeSource>,
    ) -> Result<Self, SceneErrorKind> {
        polygon_properties(mesh, policy, native, source)
    }
}

fn polygon_properties(
    mesh: &Mesh,
    policy: &CompilePolicy,
    native: Option<&NativeRealization>,
    source: Option<&NativeSource>,
) -> Result<Polygon, SceneErrorKind> {
    let error = SceneErrorKind::UnrepresentableGeometry;
    let mut positions = Vec::new();
    let mut lookup = BTreeMap::new();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in mesh.vertices() {
        let position = *mesh.vertex_position(vertex).expect("evaluated live vertex");
        if !position.iter().all(|v| v.is_finite()) {
            return Err(error);
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
        let index = i32::try_from(positions.len()).map_err(|_| error.clone())?;
        lookup.insert(vertex, index);
        positions.push(position);
    }
    let derived = mesh.derive_corner_normals(&NormalParams::default());
    let normal_overrides = mesh.attrs().sparse(attr::CORNER_NORMAL_OVERRIDE);
    let uv_layer = mesh.attrs().sparse(attr::CORNER_UV);
    let region_layer = mesh.attrs().dense(attr::FACE_REGION);
    let mut counts = Vec::new();
    let mut indices = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut all_uvs = true;
    let mut regions = Vec::new();
    let mut native_faces = Vec::new();
    for (index, face) in mesh.faces().enumerate() {
        if let Some(native) = native {
            let &(mapped, id) = native
                .mesh_faces
                .get(index)
                .ok_or(SceneErrorKind::NativeCorrespondence)?;
            if mapped != face {
                return Err(SceneErrorKind::NativeCorrespondence);
            }
            native_faces.push(id.0);
        }
        let (plane, _) =
            exedra_mesh::dominant_box_plane(mesh, face, exedra_mesh::DEFAULT_BOX_NORMAL_EPSILON);
        let start = indices.len();
        for edge in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(edge).expect("evaluated face edge");
            indices.push(lookup[&vertex]);
            let authored = normal_overrides.and_then(|layer| layer.get(edge.as_id()).copied());
            let normal = match policy.normals {
                NormalsSource::Derived => derived.get(CornerId::from(edge)),
                NormalsSource::CustomOnly => authored,
                NormalsSource::CustomOrDerived => {
                    authored.or_else(|| derived.get(CornerId::from(edge)))
                }
            }
            .unwrap_or([0.0; 3]);
            if !normal.iter().all(|v| v.is_finite()) {
                return Err(error);
            }
            normals.push(normal);
            let uv = uv_layer
                .and_then(|layer| layer.get(edge.as_id()).copied())
                .or_else(|| match policy.uvs {
                    UvSource::CustomOnly => None,
                    UvSource::CustomOrBoxProjected { scale } => exedra_mesh::project_corner_box(
                        mesh,
                        CornerId::from(edge),
                        plane,
                        scale,
                        [0.0; 2],
                    ),
                });
            all_uvs &= uv.is_some();
            if let Some(uv) = uv {
                if !uv.iter().all(|v| v.is_finite()) {
                    return Err(error);
                }
                uvs.push(uv);
            }
        }
        counts.push(i32::try_from(indices.len() - start).map_err(|_| error.clone())?);
        regions.push(
            region_layer
                .and_then(|layer| layer.get(face.as_id()).copied())
                .unwrap_or(0),
        );
    }
    if native.is_some_and(|native| native.mesh_faces.len() != counts.len()) {
        return Err(SceneErrorKind::NativeCorrespondence);
    }
    let points = positions.len();
    let mut payload_bytes = positions.len() * 12
        + counts.len() * 4
        + indices.len() * 4
        + normals.len() * 12
        + regions.len() * 4
        + 48;
    let primvar = |name: &str, ty, value, interpolation: &str| MeshPrimvar {
        name: name.into(),
        type_name: PropertyType::parse(ty).expect("registered USD value type"),
        value,
        interpolation: interpolation.into(),
        element_size: 1,
        indices: None,
    };
    let mut primvars = alloc::vec![
        primvar(
            "primvars:normals",
            "normal3f[]",
            Value::TypedArray(TypedArray::Vec3f(Arc::new(normals))),
            "faceVarying"
        ),
        primvar(
            "primvars:exedra:region",
            "uint[]",
            Value::TypedArray(TypedArray::UInt(Arc::new(regions))),
            "uniform"
        ),
    ];
    if native.is_some() {
        payload_bytes += native_faces.len() * 4;
        primvars.push(primvar(
            crate::NATIVE_FACE,
            "uint[]",
            Value::TypedArray(TypedArray::UInt(Arc::new(native_faces))),
            "uniform",
        ));
    }
    if all_uvs {
        payload_bytes += uvs.len() * 8;
        primvars.push(primvar(
            "primvars:st",
            "texCoord2f[]",
            Value::TypedArray(TypedArray::Vec2f(Arc::new(uvs))),
            "faceVarying",
        ));
    }
    let mesh = GeneratedMesh {
        points: Arc::new(positions),
        face_vertex_counts: Arc::new(counts),
        face_vertex_indices: Arc::new(indices),
        primvars,
    }
    .into_validated()
    .map_err(SceneErrorKind::Mesh)?;
    let mut extent = if points == 0 {
        Vec::new()
    } else {
        alloc::vec![min, max]
    };
    if let Some(source) = source {
        let bounds = source
            .conservative_bounds()
            .map_err(SceneErrorKind::Bounds)?;
        if extent.is_empty() {
            extent = alloc::vec![[0.0;3];2];
            for axis in 0..3 {
                extent[0][axis] = outward(bounds[0][axis], false)?;
                extent[1][axis] = outward(bounds[1][axis], true)?;
            }
        }
        for axis in 0..3 {
            extent[0][axis] = extent[0][axis].min(outward(bounds[0][axis], false)?);
            extent[1][axis] = extent[1][axis].max(outward(bounds[1][axis], true)?);
        }
    }
    Ok(Polygon {
        extent: Arc::new(extent),
        mesh,
        payload_bytes,
        points,
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "finite narrowed bounds are adjusted outwards and checked below"
)]
fn outward(value: f64, upper: bool) -> Result<f32, SceneErrorKind> {
    let mut result = value as f32;
    if upper && f64::from(result) < value {
        result = result.next_up();
    }
    if !upper && f64::from(result) > value {
        result = result.next_down();
    }
    if !result.is_finite() {
        return Err(SceneErrorKind::UnrepresentableGeometry);
    }
    Ok(result)
}
