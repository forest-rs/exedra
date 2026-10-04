// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::SceneErrorKind;
use alloc::{collections::BTreeMap, vec::Vec};
use exedra_compile::CompilePolicy;
use exedra_mesh::{CornerId, Mesh, NormalParams, NormalsSource, UvSource, attr};
use layerstack::{PropertySpec, PropertyType, TokenInterner, Value};
use layerstack_schemas::value;

#[derive(Debug)]
pub(crate) struct Polygon {
    pub(crate) properties: Vec<(&'static str, PropertySpec)>,
    pub(crate) payload_bytes: usize,
    pub(crate) points: usize,
}
impl Polygon {
    pub(crate) fn new(
        mesh: &Mesh,
        policy: &CompilePolicy,
        tokens: &mut TokenInterner,
    ) -> Result<Self, SceneErrorKind> {
        polygon_properties(mesh, policy, tokens)
    }
}

fn polygon_properties(
    mesh: &Mesh,
    policy: &CompilePolicy,
    tokens: &mut TokenInterner,
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
        for i in 0..3 {
            min[i] = min[i].min(position[i]);
            max[i] = max[i].max(position[i]);
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
        + 24;
    let attribute = |ty, array, element, default| {
        PropertySpec::typed_attribute(PropertyType::new(ty, array, element)).with_default(default)
    };
    let interpolation = tokens.intern("interpolation");
    let face_varying = tokens.intern("faceVarying");
    let uniform = tokens.intern("uniform");
    let mut properties = alloc::vec![
        (
            "points",
            attribute(
                "point3f",
                true,
                Value::Vec3f([0.0; 3]),
                value::write_float3_array(&positions, tokens)
            )
        ),
        (
            "faceVertexCounts",
            attribute(
                "int",
                true,
                Value::Int(0),
                value::write_int_array(&counts, tokens)
            )
        ),
        (
            "faceVertexIndices",
            attribute(
                "int",
                true,
                Value::Int(0),
                value::write_int_array(&indices, tokens)
            )
        ),
        (
            "normals",
            attribute(
                "normal3f",
                true,
                Value::Vec3f([0.0; 3]),
                value::write_float3_array(&normals, tokens)
            )
            .with_metadata(interpolation, Value::Token(face_varying))
        ),
        (
            "primvars:exedra:region",
            attribute(
                "uint",
                true,
                Value::UInt(0),
                value::write_uint_array(&regions, tokens)
            )
            .with_metadata(interpolation, Value::Token(uniform))
        ),
        (
            "extent",
            attribute(
                "float3",
                true,
                Value::Vec3f([0.0; 3]),
                value::write_float3_array(&[min, max], tokens)
            )
        ),
        (
            "subdivisionScheme",
            attribute(
                "token",
                false,
                Value::Token(tokens.intern("none")),
                Value::Token(tokens.intern("none"))
            )
            .uniform()
        ),
        (
            "orientation",
            attribute(
                "token",
                false,
                Value::Token(tokens.intern("rightHanded")),
                Value::Token(tokens.intern("rightHanded"))
            )
            .uniform()
        ),
    ];
    if all_uvs && !uvs.is_empty() {
        payload_bytes += uvs.len() * 8;
        properties.push((
            "primvars:st",
            attribute(
                "texCoord2f",
                true,
                Value::Vec2f([0.0; 2]),
                value::write_float2_array(&uvs, tokens),
            )
            .with_metadata(interpolation, Value::Token(face_varying)),
        ));
    }
    Ok(Polygon {
        properties,
        points,
        payload_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_mesh::{ExtractParams, MeshBuilder, op};

    #[test]
    fn polygon_attributes_follow_shared_extraction_policy() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder.add_face(&[0, 1, 2]).unwrap();
        let mut mesh = builder.build().unwrap().mesh;
        let corner = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .next()
            .unwrap();
        let mut edit = mesh.edit();
        op::set_corner_normal_override(&mut edit, corner, Some([1.0, 0.0, 0.0])).unwrap();
        op::set_corner_uv(&mut edit, corner, [7.0, 9.0]).unwrap();
        let _: () = edit.finish();

        for normals in [
            NormalsSource::Derived,
            NormalsSource::CustomOnly,
            NormalsSource::CustomOrDerived,
        ] {
            let policy = CompilePolicy {
                normals,
                uvs: UvSource::CustomOrBoxProjected { scale: 2.0 },
                ..CompilePolicy::default()
            };
            let mut tokens = TokenInterner::default();
            let polygon = Polygon::new(&mesh, &policy, &mut tokens).unwrap();
            let property = |name| {
                polygon
                    .properties
                    .iter()
                    .find(|(key, _)| *key == name)
                    .unwrap()
                    .1
                    .default
                    .as_ref()
                    .unwrap()
            };
            let points = value::read_float3_array(property("points"), &tokens).unwrap();
            let indices = value::read_int_array(property("faceVertexIndices"), &tokens).unwrap();
            let native_normals = value::read_float3_array(property("normals"), &tokens).unwrap();
            let native_uvs = value::read_float2_array(property("primvars:st"), &tokens).unwrap();
            let (tri, _) = mesh.to_trimesh(&ExtractParams {
                normals: policy.normals,
                uvs: policy.uvs,
                ..ExtractParams::default()
            });
            for (corner, index) in indices.iter().enumerate() {
                let position = points[usize::try_from(*index).unwrap()];
                let vertex = tri
                    .positions
                    .iter()
                    .position(|point| *point == position)
                    .unwrap();
                assert_eq!(native_normals[corner], tri.normals[vertex]);
                assert_eq!(native_uvs[corner], tri.uvs[vertex]);
            }
            assert!(
                native_uvs.contains(&[7.0, 9.0]),
                "authored UVs survive projection"
            );
        }

        let polygon = Polygon::new(
            &mesh,
            &CompilePolicy::default(),
            &mut TokenInterner::default(),
        )
        .unwrap();
        assert!(
            !polygon
                .properties
                .iter()
                .any(|(name, _)| *name == "primvars:st"),
            "partial authored coverage must not publish a shortened corner stream"
        );
    }
}
