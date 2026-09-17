// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

pub(super) fn prepare(
    source: &Mesh,
    plane: Plane3,
    policy: &SectionPolicy,
) -> Result<Prepared, SectionError> {
    let (normal, distance) = plane.normalized().ok_or(SectionError::InvalidPolicy)?;
    if !policy.distance_tolerance.is_finite()
        || policy.distance_tolerance <= 0.0
        || policy.max_triangles == 0
        || policy.max_section_vertices == 0
        || policy.max_pair_checks == 0
    {
        return Err(SectionError::InvalidPolicy);
    }
    let mesh = source;
    if mesh.faces().next().is_none()
        || !mesh.validate_deep().is_empty()
        || !mesh
            .boundary_loops()
            .map_err(|_| SectionError::InvalidMesh)?
            .is_empty()
    {
        return Err(SectionError::InvalidMesh);
    }
    let axis = if normal[0].abs() <= normal[1].abs() && normal[0].abs() <= normal[2].abs() {
        [1.0, 0.0, 0.0]
    } else if normal[1].abs() <= normal[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let u = normalize(cross(axis, normal)).ok_or(SectionError::NumericLimit)?;
    let v = cross(normal, u);
    let origin = scale(normal, distance);
    let frame = Placement3::from_axes(u, v, normal, origin);
    let mut points = Vec::new();
    let mut indices = BTreeMap::new();
    let mut sides = Vec::new();
    for vertex in mesh.vertices() {
        let p = mesh
            .vertex_position(vertex)
            .ok_or(SectionError::InvalidMesh)?
            .map(f64::from);
        let side = dot(normal, p) - distance;
        if !side.is_finite() {
            return Err(SectionError::NumericLimit);
        }
        if side.abs() <= policy.distance_tolerance {
            return Err(SectionError::AmbiguousContact);
        }
        indices.insert(
            vertex.index(),
            u32::try_from(points.len()).map_err(|_| SectionError::BudgetExceeded)?,
        );
        points.push(Point {
            position: p,
            key: PointKey::Original(vertex.index()),
            provenance: CutVertexSource::Original(vertex),
            sharpness: mesh.vertex_sharpness(vertex),
        });
        sides.push(side);
    }
    let mut prepared = Prepared {
        points,
        negative: Vec::new(),
        positive: Vec::new(),
        boundaries: Vec::new(),
        groups: Vec::new(),
        section: PlaneSection {
            frame,
            regions: Vec::new(),
            stats: SectionStats::default(),
        },
    };
    let mut cuts = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut incoming = BTreeSet::new();
    let mut triangles = Vec::new();
    let regions = mesh.attrs().dense(exedra_mesh::attr::FACE_REGION);
    let uvs = mesh.attrs().sparse(exedra_mesh::attr::CORNER_UV);
    let normals = mesh
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE);
    for face in mesh.faces() {
        let count = mesh.face_loop(face).count().saturating_sub(2);
        if count as u64 + u64::from(prepared.section.stats.input_triangles)
            > u64::from(policy.max_triangles)
        {
            return Err(SectionError::BudgetExceeded);
        }
        if mesh.face_triangles_into(face, FaceTriangulation::Robust, &mut triangles)
            || triangles.is_empty()
        {
            return Err(SectionError::Triangulation);
        }
        triangulate::restore_boundary(mesh, face, &mut triangles)?;

        let region = regions
            .and_then(|r| r.get(face.into()).copied())
            .unwrap_or(0);
        for triangle in &triangles {
            prepared.section.stats.input_triangles += 1;
            let corners = triangle.map(|corner| Corner {
                point: indices[&mesh.to_vertex(corner).expect("validated corner").index()],
                uv: uvs
                    .and_then(|layer| layer.get(corner.into()).copied())
                    .map(|p| p.map(f64::from)),
                normal: normals.and_then(|layer| layer.get(corner.into()).copied()),
            });
            let straddles = corners.iter().any(|c| sides[c.point as usize] < 0.0)
                && corners.iter().any(|c| sides[c.point as usize] > 0.0);
            if straddles {
                prepared.section.stats.split_triangles += 1;
            }
            for positive in [false, true] {
                let clipped = clip(
                    &corners,
                    &sides,
                    &mut prepared.points,
                    &mut cuts,
                    positive,
                    policy,
                )?;
                if clipped.len() < 3 {
                    continue;
                }
                if straddles && !positive {
                    for i in 0..clipped.len() {
                        let a = clipped[i].point;
                        let b = clipped[(i + 1) % clipped.len()].point;
                        if matches!(prepared.points[a as usize].key, PointKey::Cut(..))
                            && matches!(prepared.points[b as usize].key, PointKey::Cut(..))
                        {
                            // Cap orientation reverses the negative side's boundary edge.
                            if edges.insert(b, (a, face)).is_some() || !incoming.insert(a) {
                                return Err(SectionError::InvalidSection);
                            }
                        }
                    }
                }
                let polygon = Polygon {
                    corners: clipped,
                    provenance: CutFaceSource::Original(face),
                    region,
                };
                if positive {
                    prepared.positive.push(polygon);
                } else {
                    prepared.negative.push(polygon);
                }
            }
        }
    }
    prepared.section.stats.section_vertices =
        u32::try_from(cuts.len()).map_err(|_| SectionError::BudgetExceeded)?;
    for &id in cuts.values() {
        let p = prepared.points[id as usize].position;
        let actual = narrow(p).map(f64::from);
        let deviation = (dot(normal, actual) - distance).abs();
        if !actual.iter().all(|x| x.is_finite())
            || !deviation.is_finite()
            || deviation > policy.distance_tolerance
        {
            return Err(SectionError::NumericLimit);
        }
        prepared.section.stats.max_plane_deviation =
            prepared.section.stats.max_plane_deviation.max(deviation);
    }
    while let Some((start, (next, face))) = edges.pop_first() {
        let mut ids = alloc::vec![start];
        let mut faces = alloc::vec![face];
        let mut current = next;
        while current != start {
            ids.push(current);
            let (next, face) = edges.remove(&current).ok_or(SectionError::InvalidSection)?;
            faces.push(face);
            current = next;
        }
        if ids.len() < 3 {
            return Err(SectionError::InvalidSection);
        }
        let xy: Vec<_> = ids
            .iter()
            .map(|&id| {
                let p = sub(prepared.points[id as usize].position, origin);
                [dot(p, u), dot(p, v)]
            })
            .collect();
        let area = signed_area(&xy);
        if !area.is_finite() || area == 0.0 {
            return Err(SectionError::InvalidSection);
        }
        prepared.boundaries.push(Boundary {
            ids,
            faces,
            xy,
            area,
        });
    }
    let mut pair_work = 0;
    let original = group_boundaries(&prepared.boundaries, policy, &mut pair_work)?;
    let realized: Vec<_> = prepared
        .boundaries
        .iter()
        .map(|boundary| {
            let xy: Vec<_> = boundary
                .ids
                .iter()
                .map(|&id| {
                    let p = sub(
                        narrow(prepared.points[id as usize].position).map(f64::from),
                        origin,
                    );
                    [dot(p, u), dot(p, v)]
                })
                .collect();
            Boundary {
                ids: Vec::new(),
                faces: Vec::new(),
                area: signed_area(&xy),
                xy,
            }
        })
        .collect();
    let realized_groups = group_boundaries(&realized, policy, &mut pair_work).map_err(|error| {
        if error == SectionError::BudgetExceeded {
            error
        } else {
            SectionError::NumericLimit
        }
    })?;
    if realized_groups != original {
        return Err(SectionError::NumericLimit);
    }
    prepared.groups = original.groups;
    prepared.section.stats.section_loops =
        u32::try_from(prepared.boundaries.len()).map_err(|_| SectionError::BudgetExceeded)?;
    let public_loop = |i: usize| SectionLoop {
        points: prepared.boundaries[i].xy.clone(),
        edge_faces: prepared.boundaries[i].faces.clone(),
    };
    prepared.section.regions = prepared
        .groups
        .iter()
        .map(|(outer, holes)| SectionRegion {
            outer: public_loop(*outer),
            holes: holes.iter().map(|&i| public_loop(i)).collect(),
        })
        .collect();
    Ok(prepared)
}

fn clip(
    corners: &[Corner; 3],
    sides: &[f64],
    points: &mut Vec<Point>,
    cuts: &mut BTreeMap<(u32, u32), u32>,
    positive: bool,
    policy: &SectionPolicy,
) -> Result<Vec<Corner>, SectionError> {
    let mut output = Vec::with_capacity(4);
    for i in 0..3 {
        let a = corners[i];
        let b = corners[(i + 1) % 3];
        let da = sides[a.point as usize];
        let db = sides[b.point as usize];
        if (da > 0.0) == positive {
            output.push(a);
        }
        if (da > 0.0) == (db > 0.0) {
            continue;
        }
        let key = (a.point.min(b.point), a.point.max(b.point));
        let (point, t) = exedra_math::intersect_plane_edge(
            (a.point, points[a.point as usize].position, da),
            (b.point, points[b.point as usize].position, db),
        )
        .ok_or(SectionError::NumericLimit)?;
        let id = if let Some(&id) = cuts.get(&key) {
            id
        } else {
            if cuts.len() as u64 >= u64::from(policy.max_section_vertices) {
                return Err(SectionError::BudgetExceeded);
            }
            let id = u32::try_from(points.len()).map_err(|_| SectionError::BudgetExceeded)?;
            let PointKey::Original(a_id) = points[key.0 as usize].key else {
                unreachable!()
            };
            let PointKey::Original(b_id) = points[key.1 as usize].key else {
                unreachable!()
            };
            points.push(Point {
                position: point,
                key: PointKey::Cut(a_id.min(b_id), a_id.max(b_id)),
                provenance: CutVertexSource::Intersection {
                    vertices: [
                        original_vertex(points[a.point as usize]),
                        original_vertex(points[b.point as usize]),
                    ],
                    parameter: t,
                },
                sharpness: None,
            });
            cuts.insert(key, id);
            id
        };
        let uv =
            a.uv.zip(b.uv)
                .map(|(a, b)| core::array::from_fn(|i| a[i] + t * (b[i] - a[i])));
        let normal = a.normal.zip(b.normal).and_then(|(a, b)| {
            normalize(core::array::from_fn(|i| {
                f64::from(a[i]) + t * (f64::from(b[i]) - f64::from(a[i]))
            }))
            .map(narrow)
        });
        output.push(Corner {
            point: id,
            uv,
            normal,
        });
    }
    Ok(output)
}

pub(super) fn signed_area(points: &[[f64; 2]]) -> f64 {
    let origin = points[0];
    (1..points.len() - 1)
        .map(|i| {
            let a = [points[i][0] - origin[0], points[i][1] - origin[1]];
            let b = [points[i + 1][0] - origin[0], points[i + 1][1] - origin[1]];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        * 0.5
}
fn contains(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        {
            inside = !inside;
        }
    }
    inside
}
#[derive(PartialEq)]
struct BoundaryTopology {
    groups: Vec<(usize, Vec<usize>)>,
    parents: Vec<Option<usize>>,
}

fn group_boundaries(
    rings: &[Boundary],
    policy: &SectionPolicy,
    work: &mut u64,
) -> Result<BoundaryTopology, SectionError> {
    let mut groups = Vec::new();
    for ring in rings {
        if !ring.area.is_finite()
            || ring.area == 0.0
            || ring.xy.iter().enumerate().any(|(i, p)| {
                p == &ring.xy[(i + 1) % ring.xy.len()] || p.iter().any(|x| !x.is_finite())
            })
        {
            return Err(SectionError::InvalidSection);
        }
    }
    for i in 0..rings.len() {
        for j in 0..=i {
            *work = work.saturating_add(
                (rings[i].xy.len() as u64).saturating_mul(rings[j].xy.len() as u64),
            );
            if *work > policy.max_pair_checks {
                return Err(SectionError::BudgetExceeded);
            }
            let conflict = if i == j {
                crate::polygon::ring_self_intersects(&rings[i].xy)
            } else {
                crate::polygon::rings_intersect(&rings[i].xy, &rings[j].xy)
            };
            if conflict {
                return Err(SectionError::InvalidSection);
            }
        }
    }
    let mut parents = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        let mut parent: Option<usize> = None;
        for (j, other) in rings.iter().enumerate() {
            if i == j || other.area.abs() <= ring.area.abs() {
                continue;
            }
            *work = work.saturating_add(other.xy.len() as u64);
            if *work > policy.max_pair_checks {
                return Err(SectionError::BudgetExceeded);
            }
            if contains(&other.xy, ring.xy[0])
                && parent.is_none_or(|p| other.area.abs() < rings[p].area.abs())
            {
                parent = Some(j);
            }
        }
        parents.push(parent);
    }
    for i in 0..rings.len() {
        let mut depth = 0;
        let mut parent = parents[i];
        while let Some(p) = parent {
            depth += 1;
            parent = parents[p];
        }
        if (rings[i].area > 0.0) != (depth % 2 == 0) {
            return Err(SectionError::InvalidSection);
        }
        if depth % 2 == 0 {
            let holes = parents
                .iter()
                .enumerate()
                .filter_map(|(j, p)| (*p == Some(i)).then_some(j))
                .collect();
            groups.push((i, holes));
        }
    }
    Ok(BoundaryTopology { groups, parents })
}

fn original_vertex(point: Point) -> VertexId {
    match point.provenance {
        CutVertexSource::Original(vertex) => vertex,
        CutVertexSource::Intersection { .. } => {
            unreachable!("clipping only intersects original triangle endpoints")
        }
    }
}
