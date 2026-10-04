// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::SceneErrorKind;
use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use exedra_compile::CompilePolicy;
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
    pub(crate) fn new(mesh: &Mesh, policy: &CompilePolicy) -> Result<Self, SceneErrorKind> {
        polygon_properties(mesh, policy)
    }
}

fn polygon_properties(mesh: &Mesh, policy: &CompilePolicy) -> Result<Polygon, SceneErrorKind> {
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
    for face in mesh.faces() {
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
    let points = positions.len();
    let mut payload_bytes = positions.len() * 12
        + counts.len() * 4
        + indices.len() * 4
        + normals.len() * 12
        + regions.len() * 4
        + 48;
    let primvar = |name: &str, ty, element, value, interpolation: &str| MeshPrimvar {
        name: name.into(),
        type_name: PropertyType::new(ty, true, element),
        value,
        interpolation: interpolation.into(),
        element_size: 1,
        indices: None,
    };
    let mut primvars = alloc::vec![
        primvar(
            "primvars:normals",
            "normal3f",
            Value::Vec3f([0.0; 3]),
            Value::TypedArray(TypedArray::Vec3f(Arc::new(normals))),
            "faceVarying"
        ),
        primvar(
            "primvars:exedra:region",
            "uint",
            Value::UInt(0),
            Value::TypedArray(TypedArray::UInt(Arc::new(regions))),
            "uniform"
        ),
    ];
    if all_uvs {
        payload_bytes += uvs.len() * 8;
        primvars.push(primvar(
            "primvars:st",
            "texCoord2f",
            Value::Vec2f([0.0; 2]),
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
    Ok(Polygon {
        extent: Arc::new(if points == 0 {
            Vec::new()
        } else {
            alloc::vec![min, max]
        }),
        mesh,
        payload_bytes,
        points,
    })
}
