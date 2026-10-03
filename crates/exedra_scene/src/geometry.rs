// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{collections::BTreeMap, rc::Rc, vec::Vec};
use exedra_constructive::{evaluate::Evaluation, ir::Recipe};
use exedra_mesh::{CornerId, Mesh, NormalParams, attr};
use layerstack::{PropertySpec, PropertyType, TokenInterner, Value};
use layerstack_schemas::value;

use crate::SceneErrorKind;

#[derive(Debug)]
pub(crate) struct Geometry {
    pub(crate) recipe: Recipe,
    pub(crate) evaluation: Evaluation,
    pub(crate) properties: Vec<(&'static str, PropertySpec)>,
    pub(crate) payload_bytes: usize,
    pub(crate) points: usize,
}

impl Geometry {
    pub(crate) fn new(
        recipe: Recipe,
        evaluation: Evaluation,
        tokens: &mut TokenInterner,
    ) -> Result<Self, SceneErrorKind> {
        let [body] = evaluation.bodies.as_slice() else {
            return Err(SceneErrorKind::Refused(Rc::new(evaluation.report.clone())));
        };
        let Encoded {
            properties,
            points,
            payload_bytes,
        } = polygon_properties(&body.body.mesh, tokens)?;
        Ok(Self {
            recipe,
            evaluation,
            properties,
            points,
            payload_bytes,
        })
    }
}

struct Encoded {
    properties: Vec<(&'static str, PropertySpec)>,
    points: usize,
    payload_bytes: usize,
}

fn polygon_properties(mesh: &Mesh, tokens: &mut TokenInterner) -> Result<Encoded, SceneErrorKind> {
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
        let start = indices.len();
        for edge in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(edge).expect("evaluated face edge");
            indices.push(lookup[&vertex]);
            let normal = normal_overrides
                .and_then(|layer| layer.get(edge.as_id()).copied())
                .or_else(|| derived.get(CornerId::from(edge)))
                .unwrap_or([0.0; 3]);
            if !normal.iter().all(|v| v.is_finite()) {
                return Err(error);
            }
            normals.push(normal);
            let uv = uv_layer.and_then(|layer| layer.get(edge.as_id()).copied());
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
    Ok(Encoded {
        properties,
        points,
        payload_bytes,
    })
}
