// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

pub(super) fn cap_triangles(prepared: &Prepared) -> Result<Vec<[u32; 3]>, SectionError> {
    let mut output = Vec::new();
    for (outer, holes) in &prepared.groups {
        let outer = &prepared.boundaries[*outer];
        let hole_points: Vec<_> = holes
            .iter()
            .map(|&i| prepared.boundaries[i].xy.as_slice())
            .collect();
        let input = exedra_triangulate::PolygonInput {
            outer: &outer.xy,
            holes: &hole_points,
        };
        let params = exedra_triangulate::RefineParams::default()
            .with_max_steiner_points(0)
            .with_boundary_splits(exedra_triangulate::BoundarySplits::Forbidden);
        let cap =
            exedra_triangulate::refine(&input, &params).map_err(|_| SectionError::Triangulation)?;
        let ids: Vec<_> = outer
            .ids
            .iter()
            .chain(holes.iter().flat_map(|&i| &prepared.boundaries[i].ids))
            .copied()
            .collect();
        for triangle in cap.triangles {
            output.push(triangle.map(|i| ids[i as usize]));
        }
    }
    Ok(output)
}

fn original_edge(a: PointKey, b: PointKey) -> Option<(u32, u32)> {
    match (a, b) {
        (PointKey::Original(a), PointKey::Original(b)) => Some((a.min(b), a.max(b))),
        (PointKey::Original(v), PointKey::Cut(a, b))
        | (PointKey::Cut(a, b), PointKey::Original(v))
            if v == a || v == b =>
        {
            Some((a, b))
        }
        _ => None,
    }
}

pub(super) fn emit(
    source: &Mesh,
    prepared: &Prepared,
    polygons: &[Polygon],
    caps: &[[u32; 3]],
    cap_region: u32,
    positive: bool,
) -> Result<Option<CutMesh>, SectionError> {
    if polygons.is_empty() {
        return Ok(None);
    }
    let mut builder = MeshBuilder::new();
    let mut ids = BTreeMap::new();
    let mut vertex_sources = Vec::new();
    let mut sharpness = Vec::new();
    let mut face_sources = Vec::new();
    let mut corner_uvs = Vec::new();
    let mut corner_normals = Vec::new();
    let mut edge_attrs = BTreeMap::new();
    for face in source.faces() {
        for edge in source.face_loop(face) {
            let a = source.from_vertex(edge).expect("validated edge").index();
            let b = source.to_vertex(edge).expect("validated edge").index();
            edge_attrs.insert(
                (a.min(b), a.max(b)),
                (
                    source.edge_seam(edge).unwrap_or(false),
                    source.edge_sharpness(edge).unwrap_or(0.0),
                ),
            );
        }
    }
    let mut rim_edges = BTreeSet::new();
    let mut xy = BTreeMap::new();
    for boundary in &prepared.boundaries {
        for (i, &id) in boundary.ids.iter().enumerate() {
            let next = boundary.ids[(i + 1) % boundary.ids.len()];
            rim_edges.insert((id.min(next), id.max(next)));
            xy.insert(id, boundary.xy[i]);
        }
    }
    let cap_polygons: Vec<_> = caps
        .iter()
        .map(|t| {
            let t = if positive { [t[2], t[1], t[0]] } else { *t };
            Polygon {
                corners: t
                    .map(|id| Corner {
                        point: id,
                        uv: Some(xy[&id]),
                        normal: None,
                    })
                    .to_vec(),
                provenance: CutFaceSource::Cap,
                region: cap_region,
            }
        })
        .collect();
    for polygon in polygons.iter().chain(&cap_polygons) {
        // A clipped triangle is convex, so this fan defines its geometry.
        for i in 1..polygon.corners.len() - 1 {
            let triangle = [
                polygon.corners[0],
                polygon.corners[i],
                polygon.corners[i + 1],
            ];
            let p = triangle.map(|c| prepared.points[c.point as usize].position);
            let rounded = p.map(|p| narrow(p).map(f64::from));
            let expected = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            let actual = cross(sub(rounded[1], rounded[0]), sub(rounded[2], rounded[0]));
            let orientation = dot(expected, actual);
            if !orientation.is_finite() || orientation <= 0.0 {
                return Err(SectionError::NumericLimit);
            }
            let mut out = [0; 3];
            let mut seams = [false; 3];
            let mut creases = [0.0; 3];
            for j in 0..3 {
                let id = triangle[j].point;
                let point = prepared.points[id as usize];
                out[j] = *ids.entry(id).or_insert_with(|| {
                    vertex_sources.push(point.provenance);
                    sharpness.push(point.sharpness);
                    builder.push_vertex(narrow(point.position))
                });
                let next = triangle[(j + 1) % 3].point;
                let attrs = original_edge(point.key, prepared.points[next as usize].key)
                    .and_then(|e| edge_attrs.get(&e))
                    .copied()
                    .unwrap_or((false, 0.0));
                seams[j] = attrs.0;
                creases[j] = if rim_edges.contains(&(id.min(next), id.max(next))) {
                    1.0
                } else {
                    attrs.1
                };
            }
            builder
                .add_face_with_attrs(
                    &out,
                    &FaceBuildAttrs {
                        region: Some(polygon.region),
                        edge_seams: Some(&seams),
                        edge_sharpness: Some(&creases),
                    },
                )
                .map_err(|_| SectionError::BuildFailed)?;
            face_sources.push(polygon.provenance);
            // Builder edge i ends at input corner i+1.
            let destination_corners = [triangle[1], triangle[2], triangle[0]];
            corner_uvs.push(destination_corners.map(|c| {
                c.uv.map(|uv| {
                    let p = narrow([uv[0], uv[1], 0.0]);
                    [p[0], p[1]]
                })
            }));
            corner_normals.push(destination_corners.map(|c| c.normal));
        }
    }
    let mut built = builder.build().map_err(|_| SectionError::BuildFailed)?;
    {
        let mut edit = built.mesh.edit();
        for (id, value) in built.vertex_ids.iter().zip(sharpness) {
            if let Some(value) = value {
                exedra_mesh::op::set_vertex_sharpness(&mut edit, *id, value)
                    .map_err(|_| SectionError::BuildFailed)?;
            }
        }
        for ((edges, uvs), normals) in built
            .face_edge_ids
            .iter()
            .zip(corner_uvs)
            .zip(corner_normals)
        {
            for ((edge, uv), normal) in edges.iter().zip(uvs).zip(normals) {
                if let Some(uv) = uv {
                    if uv.iter().any(|x| !x.is_finite()) {
                        return Err(SectionError::NumericLimit);
                    }
                    exedra_mesh::op::set_corner_uv(&mut edit, *edge, uv)
                        .map_err(|_| SectionError::BuildFailed)?;
                }
                if let Some(normal) = normal {
                    exedra_mesh::op::set_corner_normal_override(&mut edit, *edge, Some(normal))
                        .map_err(|_| SectionError::BuildFailed)?;
                }
            }
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }
    if !built.mesh.validate_deep().is_empty()
        || !built
            .mesh
            .boundary_loops()
            .map_err(|_| SectionError::BuildFailed)?
            .is_empty()
    {
        return Err(SectionError::BuildFailed);
    }
    Ok(Some(CutMesh {
        face_sources: built.face_ids.iter().copied().zip(face_sources).collect(),
        vertex_sources: built
            .vertex_ids
            .iter()
            .copied()
            .zip(vertex_sources)
            .collect(),
        mesh: built.mesh,
    }))
}
