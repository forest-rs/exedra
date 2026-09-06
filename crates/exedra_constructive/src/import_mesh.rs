// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Placement of imported and instanced meshes, including orientation repair
//! under constructive reflections.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use exedra_mesh::{HalfEdgeId, Mesh, VertexId};

use crate::ir::Placement3;
use crate::tessellate::TessellateError;

#[derive(Copy, Clone, Debug, Default)]
struct EdgeAttrs {
    seam: Option<bool>,
    sharpness: Option<f32>,
}

#[derive(Copy, Clone, Debug, Default)]
struct CornerAttrs {
    uv: Option<[f32; 2]>,
    normal: Option<[f32; 3]>,
}

#[derive(Clone, Debug)]
struct FaceAttrs {
    edges: Vec<EdgeAttrs>,
    corners: Vec<CornerAttrs>,
}

/// Places a mesh under an affine placement.
///
/// This is the one transformation contract shared by imported-mesh
/// placement and instancing: positions transform in `f64` and narrow once
/// to `f32`, authored corner normal overrides transport as covectors
/// (inverse transpose, renormalized), and every other attribute follows its
/// owning element unchanged. Reflecting placements additionally rebuild the
/// mesh with reversed face loops so outward orientation survives.
///
/// # Errors
///
/// [`TessellateError::NonFiniteGeometry`] when a transformed position or
/// normal is not representable, or when an authored normal cannot be
/// transported because the placement's linear part is singular.
pub(crate) fn transform(source: &Mesh, placement: &Placement3) -> Result<Mesh, TessellateError> {
    if crate::tessellate::det3(placement) < 0.0 {
        transform_reflecting(source, placement)
    } else {
        transform_proper(source, placement)
    }
}

/// Applies a non-reflecting placement in place: topology and every attribute
/// keep their ids, positions and authored normals move.
fn transform_proper(source: &Mesh, placement: &Placement3) -> Result<Mesh, TessellateError> {
    let positions = placed_positions(source, placement)?;
    let normals = placed_normals(source, placement)?;
    let mut mesh = source.clone();
    {
        let mut edit = mesh.edit();
        for (vertex, position) in positions {
            exedra_mesh::op::set_vertex_position(&mut edit, vertex, position)
                .expect("a cloned live vertex stays live");
        }
        for (corner, normal) in normals {
            exedra_mesh::op::set_corner_normal_override(&mut edit, corner, Some(normal))
                .expect("a cloned live corner stays live");
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }
    Ok(mesh)
}

/// Every live vertex with its placed, narrowed position.
///
/// Narrowing is the representation boundary the placement contract owns:
/// an overflow to infinity is refused, and so is a placement that folds two
/// distinct endpoints of a face edge onto one `f32` position, because the
/// source's nondegenerate edge would silently become a zero-length one.
fn placed_positions(
    source: &Mesh,
    placement: &Placement3,
) -> Result<Vec<(VertexId, [f32; 3])>, TessellateError> {
    let mut positions = Vec::with_capacity(source.vertices().count());
    let mut placed = BTreeMap::<u32, [f32; 3]>::new();
    for vertex in source.vertices() {
        let local = source
            .vertex_position(vertex)
            .copied()
            .expect("a live vertex has a position")
            .map(f64::from);
        let position = exedra_math::narrow(apply_placement(placement, local));
        if !exedra_math::finite(position) {
            return Err(TessellateError::NonFiniteGeometry);
        }
        positions.push((vertex, position));
        placed.insert(vertex.index(), position);
    }
    for edge in source.faces().flat_map(|face| source.face_loop(face)) {
        let from = source
            .from_vertex(edge)
            .expect("a validated face edge has an origin");
        let to = source
            .to_vertex(edge)
            .expect("a validated face edge has a destination");
        let distinct_source = source.vertex_position(from) != source.vertex_position(to);
        if distinct_source && placed[&from.index()] == placed[&to.index()] {
            return Err(TessellateError::CollapsedGeometry);
        }
    }
    Ok(positions)
}

/// Every authored corner normal override, transported under `placement`.
fn placed_normals(
    source: &Mesh,
    placement: &Placement3,
) -> Result<Vec<(HalfEdgeId, [f32; 3])>, TessellateError> {
    let Some(layer) = source
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
    else {
        return Ok(Vec::new());
    };
    // Inverse-transpose transport is needed only for authored normal
    // overrides. A representable placement with no such overrides should
    // not be refused merely because its inverse overflows at an extreme
    // scale.
    let inverse = inverse_linear(placement);
    let mut normals = Vec::new();
    for corner in source.faces().flat_map(|face| source.face_loop(face)) {
        if let Some(normal) = layer.get(corner.into()) {
            let inverse = inverse.ok_or(TessellateError::NonFiniteGeometry)?;
            normals.push((corner, transform_normal(*normal, inverse)?));
        }
    }
    Ok(normals)
}

/// Applies a known reflecting placement and repairs the resulting orientation.
///
/// Rebuilding is necessary because an affine transform with negative
/// determinant reverses every face. Face loops are reversed while attributes
/// follow their semantic owners: region values follow faces, seam/sharpness
/// values follow undirected edges, and UV/normal values follow face corners.
fn transform_reflecting(source: &Mesh, placement: &Placement3) -> Result<Mesh, TessellateError> {
    debug_assert!(
        crate::tessellate::det3(placement) < 0.0,
        "orientation repair is only needed for reflecting placements"
    );

    // See `placed_normals` for why a missing inverse is tolerated here.
    let inverse = inverse_linear(placement);
    let regions = source
        .attrs()
        .dense(exedra_mesh::attr::FACE_REGION)
        .expect("every mesh has the built-in face-region layer");
    let seams = source.attrs().sparse(exedra_mesh::attr::EDGE_SEAM);
    let edge_sharpness = source.attrs().sparse(exedra_mesh::attr::EDGE_SHARPNESS);
    let uvs = source.attrs().sparse(exedra_mesh::attr::CORNER_UV);
    let normals = source
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE);

    let source_edges = collect_edge_attrs(source, seams, edge_sharpness);
    let mut builder = exedra_mesh::MeshBuilder::new();
    let mut vertex_indices = BTreeMap::<u32, u32>::new();
    let mut vertex_sharpness = Vec::with_capacity(source.vertices().count());
    for (vertex, position) in placed_positions(source, placement)? {
        let output = builder.push_vertex(position);
        vertex_indices.insert(vertex.index(), output);
        vertex_sharpness.push(source.vertex_sharpness(vertex));
    }

    let mut face_attrs = Vec::with_capacity(source.faces().count());
    for face in source.faces() {
        let source_loop = source.face_loop(face).collect::<Vec<_>>();
        let mut source_vertices = source_loop
            .iter()
            .map(|edge| {
                source
                    .from_vertex(*edge)
                    .expect("a validated face edge has an origin")
                    .index()
            })
            .collect::<Vec<_>>();
        source_vertices.reverse();

        let output_loop = source_vertices
            .iter()
            .map(|vertex| vertex_indices[vertex])
            .collect::<Vec<_>>();
        let corner_by_vertex = collect_corner_attrs(source, &source_loop, uvs, normals, inverse)?;
        let mut output_edges = Vec::with_capacity(source_vertices.len());
        let mut output_corners = Vec::with_capacity(source_vertices.len());
        for index in 0..source_vertices.len() {
            let from = source_vertices[index];
            let to = source_vertices[(index + 1) % source_vertices.len()];
            output_edges.push(
                source_edges
                    .get(&ordered_edge(from, to))
                    .copied()
                    .expect("a rebuilt edge comes from one validated source edge"),
            );
            // Corner attributes live on a half-edge's destination vertex.
            // Looking them up by face-local vertex keeps them attached to the
            // same geometric corner after reversing the directed face loop.
            output_corners.push(
                corner_by_vertex
                    .get(&to)
                    .copied()
                    .expect("a rebuilt corner comes from one source corner"),
            );
        }

        builder.add_face_with_attrs(
            &output_loop,
            &exedra_mesh::FaceBuildAttrs {
                region: regions.get(face.into()).copied(),
                edge_seams: None,
                edge_sharpness: None,
            },
        )?;
        face_attrs.push(FaceAttrs {
            edges: output_edges,
            corners: output_corners,
        });
    }

    let mut built = builder.build()?;
    let has_authored_attrs = vertex_sharpness.iter().any(Option::is_some)
        || face_attrs.iter().any(|face| {
            face.edges
                .iter()
                .any(|edge| edge.seam.is_some() || edge.sharpness.is_some())
                || face
                    .corners
                    .iter()
                    .any(|corner| corner.uv.is_some() || corner.normal.is_some())
        });
    if has_authored_attrs {
        let mut edit = built.mesh.edit();
        for (vertex, sharpness) in built.vertex_ids.iter().zip(vertex_sharpness) {
            if let Some(sharpness) = sharpness {
                exedra_mesh::op::set_vertex_sharpness(&mut edit, *vertex, sharpness)
                    .expect("builder provenance names one live vertex");
            }
        }
        for (output_edges, attrs) in built.face_edge_ids.iter().zip(&face_attrs) {
            for ((edge, edge_attrs), corner_attrs) in
                output_edges.iter().zip(&attrs.edges).zip(&attrs.corners)
            {
                if let Some(seam) = edge_attrs.seam {
                    exedra_mesh::op::set_edge_seam(&mut edit, *edge, seam)
                        .expect("builder provenance names one live edge");
                }
                if let Some(sharpness) = edge_attrs.sharpness {
                    exedra_mesh::op::set_edge_sharpness(&mut edit, *edge, sharpness)
                        .expect("builder provenance names one live edge");
                }
                if let Some(uv) = corner_attrs.uv {
                    exedra_mesh::op::set_corner_uv(&mut edit, *edge, uv)
                        .expect("builder provenance names one live corner");
                }
                if let Some(normal) = corner_attrs.normal {
                    exedra_mesh::op::set_corner_normal_override(&mut edit, *edge, Some(normal))
                        .expect("builder provenance names one live corner");
                }
            }
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }

    Ok(built.mesh)
}

fn collect_edge_attrs(
    source: &Mesh,
    seams: Option<&exedra_mesh::attributes::SparseLayer<bool>>,
    sharpness: Option<&exedra_mesh::attributes::SparseLayer<f32>>,
) -> BTreeMap<(u32, u32), EdgeAttrs> {
    let mut attributes = BTreeMap::new();
    for face in source.faces() {
        for edge in source.face_loop(face) {
            let from = source
                .from_vertex(edge)
                .expect("a validated face edge has an origin")
                .index();
            let to = source
                .to_vertex(edge)
                .expect("a validated face edge has a destination")
                .index();
            let canonical = source
                .canonical_edge(edge)
                .expect("a validated face edge has a twin");
            attributes
                .entry(ordered_edge(from, to))
                .or_insert(EdgeAttrs {
                    seam: seams.and_then(|layer| layer.get(canonical.into()).copied()),
                    sharpness: sharpness.and_then(|layer| layer.get(canonical.into()).copied()),
                });
        }
    }
    attributes
}

fn collect_corner_attrs(
    source: &Mesh,
    face_loop: &[HalfEdgeId],
    uvs: Option<&exedra_mesh::attributes::SparseLayer<[f32; 2]>>,
    normals: Option<&exedra_mesh::attributes::SparseLayer<[f32; 3]>>,
    inverse: Option<[[f64; 3]; 3]>,
) -> Result<BTreeMap<u32, CornerAttrs>, TessellateError> {
    let mut attributes = BTreeMap::new();
    for edge in face_loop {
        let vertex = source
            .to_vertex(*edge)
            .expect("a validated face edge has a destination");
        let normal = normals
            .and_then(|layer| layer.get((*edge).into()).copied())
            .map(|normal| {
                inverse
                    .ok_or(TessellateError::NonFiniteGeometry)
                    .and_then(|inverse| transform_normal(normal, inverse))
            })
            .transpose()?;
        attributes.insert(
            vertex.index(),
            CornerAttrs {
                uv: uvs.and_then(|layer| layer.get((*edge).into()).copied()),
                normal,
            },
        );
    }
    Ok(attributes)
}

fn transform_normal(normal: [f32; 3], inverse: [[f64; 3]; 3]) -> Result<[f32; 3], TessellateError> {
    let normal = normal.map(f64::from);
    // Normals are covectors: inverse-transpose is required under non-uniform
    // scale or shear. Reversing reflected loops then makes this transformed
    // outward normal agree with the repaired geometric winding.
    let transformed = [
        inverse[0][0] * normal[0] + inverse[1][0] * normal[1] + inverse[2][0] * normal[2],
        inverse[0][1] * normal[0] + inverse[1][1] * normal[1] + inverse[2][1] * normal[2],
        inverse[0][2] * normal[0] + inverse[1][2] * normal[1] + inverse[2][2] * normal[2],
    ];
    exedra_math::normalize(transformed)
        .map(exedra_math::narrow)
        .ok_or(TessellateError::NonFiniteGeometry)
}

fn inverse_linear(placement: &Placement3) -> Option<[[f64; 3]; 3]> {
    let a = [
        [
            placement.rows[0][0],
            placement.rows[0][1],
            placement.rows[0][2],
        ],
        [
            placement.rows[1][0],
            placement.rows[1][1],
            placement.rows[1][2],
        ],
        [
            placement.rows[2][0],
            placement.rows[2][1],
            placement.rows[2][2],
        ],
    ];
    let determinant = exedra_math::det3(a);
    if !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    let inverse = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) / determinant,
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) / determinant,
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) / determinant,
        ],
        [
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) / determinant,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) / determinant,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) / determinant,
        ],
        [
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) / determinant,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) / determinant,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) / determinant,
        ],
    ];
    inverse
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(inverse)
}

fn apply_placement(placement: &Placement3, point: [f64; 3]) -> [f64; 3] {
    let r = &placement.rows;
    [
        r[0][0] * point[0] + r[0][1] * point[1] + r[0][2] * point[2] + r[0][3],
        r[1][0] * point[0] + r[1][1] * point[1] + r[1][2] * point[2] + r[1][3],
        r[2][0] * point[0] + r[2][1] * point[1] + r[2][2] * point[2] + r[2][3],
    ]
}

fn ordered_edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}
