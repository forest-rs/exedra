// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checked joining of coincident component representatives on transition patches.

use alloc::vec::Vec;
use exedra_math::{norm, sub};
use exedra_mesh::{BuildError, Mesh, MeshBuilder, VertexId, attr, op};
use hashbrown::{HashMap, HashSet};

use super::control::{ExtractionResource, ExtractionWitness, RunContext};
use super::{
    DualContourError, QuadDiagonal, VertexEntry, emit_transition_with_sharpness,
    select_quad_diagonal, triangle_is_nondegenerate,
};

pub(super) fn collapse_coincident_edges(
    mesh: &mut Mesh,
    tolerance: f32,
    has_provenance: bool,
    run: &RunContext,
) -> Result<(usize, f64), DualContourError> {
    let mut sources = HashMap::<_, Vec<[f32; 3]>>::new();
    let candidates = mesh
        .faces()
        .filter(|&face| {
            let points = mesh
                .face_loop(face)
                .map(|corner| {
                    *mesh
                        .vertex_position(mesh.to_vertex(corner).unwrap())
                        .unwrap()
                })
                .collect::<Vec<_>>();
            !polygon_is_nondegenerate(&points)
        })
        .collect::<Vec<_>>();
    let mut count = 0;
    loop {
        let candidate = candidates.iter().copied().find_map(|face| {
            let corners = mesh.face_loop(face).collect::<Vec<_>>();
            corners
                .into_iter()
                .filter_map(|edge| {
                    let a = mesh.from_vertex(edge).unwrap();
                    let b = mesh.to_vertex(edge).unwrap();
                    let (keep, remove) = if a < b { (a, b) } else { (b, a) };
                    let position = *mesh.vertex_position(keep).unwrap();
                    let distance = merge_distance(position, *mesh.vertex_position(remove).unwrap());
                    (distance <= f64::from(tolerance)
                        && sources.get(&remove).is_none_or(|positions| {
                            positions
                                .iter()
                                .all(|&p| merge_distance(position, p) <= f64::from(tolerance))
                        }))
                    .then_some((edge, keep, remove, distance))
                })
                .min_by(|a, b| a.3.total_cmp(&b.3).then_with(|| a.0.cmp(&b.0)))
        });
        let Some((edge, keep, remove, _)) = candidate else {
            break;
        };
        let removed_position = *mesh.vertex_position(remove).unwrap();
        let witness = ExtractionWitness::MeshPatch {
            positions: alloc::vec![*mesh.vertex_position(keep).unwrap(), removed_position],
            source: has_provenance
                .then(|| {
                    mesh.face(edge).and_then(|face| {
                        mesh.attrs()
                            .dense(attr::FACE_REGION)
                            .and_then(|layer| layer.get(face.as_id()).copied())
                    })
                })
                .flatten(),
        };
        run.grow(
            ExtractionResource::VertexJoins,
            run.work.get().vertex_joins,
            1,
        )
        .map_err(|error| error.with_witness(witness.clone()))?;
        check_vertex_link(mesh, keep).map_err(|error| error.with_witness(witness.clone()))?;
        check_vertex_link(mesh, remove).map_err(|error| error.with_witness(witness.clone()))?;
        let mut edit = mesh.edit();
        let result = op::collapse_edge(&mut edit, edge);
        #[expect(
            unused_must_use,
            reason = "the direct edit sink has no retained change output"
        )]
        {
            edit.finish();
        }
        result.map_err(|error| DualContourError::collapse(error).with_witness(witness.clone()))?;
        check_vertex_link(mesh, keep).map_err(|error| error.with_witness(witness))?;
        let removed = sources.remove(&remove).unwrap_or_default();
        let kept_sources = sources.entry(keep).or_default();
        kept_sources.push(removed_position);
        kept_sources.extend(removed);
        count += 1;
        let mut stats = run.stats.get();
        stats.coincident_edge_collapses = count;
        stats.vertices = mesh.vertices().count();
        stats.faces = mesh.faces().count();
        stats.max_vertex_merge_displacement = sources
            .iter()
            .flat_map(|(vertex, originals)| {
                let position = *mesh.vertex_position(*vertex).unwrap();
                originals.iter().map(move |&p| merge_distance(position, p))
            })
            .fold(0.0_f64, f64::max);
        run.stats.set(stats);
    }
    for (triangle, face) in mesh.faces().enumerate() {
        let points = mesh
            .face_loop(face)
            .map(|corner| {
                *mesh
                    .vertex_position(mesh.to_vertex(corner).unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        if !polygon_is_nondegenerate(&points) {
            return Err(
                DualContourError::build(BuildError::DegenerateTriangle { triangle }).with_witness(
                    ExtractionWitness::MeshPatch {
                        positions: points,
                        source: has_provenance
                            .then(|| {
                                mesh.attrs()
                                    .dense(attr::FACE_REGION)
                                    .and_then(|layer| layer.get(face.as_id()).copied())
                            })
                            .flatten(),
                    },
                ),
            );
        }
    }
    let max_displacement = sources
        .iter()
        .flat_map(|(vertex, originals)| {
            let position = *mesh.vertex_position(*vertex).unwrap();
            originals.iter().map(move |&p| merge_distance(position, p))
        })
        .fold(0.0_f64, f64::max);
    Ok((count, max_displacement))
}

fn check_vertex_link(mesh: &Mesh, vertex: VertexId) -> Result<(), DualContourError> {
    let degree = mesh.vertex_star(vertex).count();
    let Some(start) = mesh.vertex_out(vertex) else {
        return Err(DualContourError::disconnected(vertex.index()));
    };
    let mut edge = start;
    for step in 0..degree {
        edge = mesh
            .next(mesh.twin(edge).expect("built twin"))
            .expect("built successor");
        if mesh.from_vertex(edge) != Some(vertex) || (edge == start) != (step + 1 == degree) {
            return Err(DualContourError::disconnected(vertex.index()));
        }
    }
    Ok(())
}

pub(super) fn triangulate_joined_mesh(
    mesh: &Mesh,
    has_provenance: bool,
    run: &RunContext,
) -> Result<Mesh, DualContourError> {
    run.reserve(ExtractionResource::Vertices, 0, mesh.vertices().count())?;
    let mut builder = MeshBuilder::new();
    let indices = mesh
        .vertices()
        .map(|vertex| {
            (
                vertex,
                builder.push_vertex(*mesh.vertex_position(vertex).unwrap()),
            )
        })
        .collect::<HashMap<_, _>>();
    let ordered = |a, b| if a < b { (a, b) } else { (b, a) };
    let mut edges = mesh
        .half_edges()
        .map(|edge| {
            ordered(
                indices[&mesh.from_vertex(edge).unwrap()],
                indices[&mesh.to_vertex(edge).unwrap()],
            )
        })
        .collect::<HashSet<_>>();
    let mut face_count = 0;
    for face in mesh.faces() {
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        let entries = corners
            .iter()
            .map(|&corner| {
                let vertex = mesh.to_vertex(corner).unwrap();
                VertexEntry {
                    builder_index: indices[&vertex],
                    position: *mesh.vertex_position(vertex).unwrap(),
                    sharpness: 0.0,
                }
            })
            .collect::<Vec<_>>();
        // Face-loop corners point to a vertex; the following half-edge leaves it.
        let sharpness = corners
            .iter()
            .cycle()
            .skip(1)
            .take(corners.len())
            .map(|&corner| mesh.edge_sharpness(corner).unwrap_or(0.0))
            .collect::<Vec<_>>();
        let region = mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        let diagonal = if let [a, b, c, d] = entries.as_slice() {
            let zero_two = ordered(a.builder_index, c.builder_index);
            let one_three = ordered(b.builder_index, d.builder_index);
            let candidates =
                match select_quad_diagonal([a.position, b.position, c.position, d.position]) {
                    Some(QuadDiagonal::OneThree) => [QuadDiagonal::OneThree, QuadDiagonal::ZeroTwo],
                    _ => [QuadDiagonal::ZeroTwo, QuadDiagonal::OneThree],
                };
            let chosen = candidates.into_iter().find(|diagonal| match diagonal {
                QuadDiagonal::ZeroTwo => {
                    !edges.contains(&zero_two)
                        && triangle_is_nondegenerate([a.position, b.position, c.position])
                        && triangle_is_nondegenerate([a.position, c.position, d.position])
                }
                QuadDiagonal::OneThree => {
                    !edges.contains(&one_three)
                        && triangle_is_nondegenerate([a.position, b.position, d.position])
                        && triangle_is_nondegenerate([b.position, c.position, d.position])
                }
            });
            if let Some(chosen) = chosen {
                edges.insert(match chosen {
                    QuadDiagonal::ZeroTwo => zero_two,
                    QuadDiagonal::OneThree => one_three,
                });
            }
            chosen
        } else {
            None
        };
        let witness = || ExtractionWitness::MeshPatch {
            positions: entries.iter().map(|entry| entry.position).collect(),
            source: has_provenance.then_some(region),
        };
        run.grow(ExtractionResource::Faces, face_count, entries.len() - 2)
            .map_err(|error| error.with_witness(witness()))?;
        emit_transition_with_sharpness(
            &mut builder,
            &entries,
            &sharpness,
            region,
            &mut face_count,
            diagonal,
        )
        .map_err(|error| error.with_witness(witness()))?;
    }
    Ok(builder.build().map_err(DualContourError::build)?.mesh)
}

pub(super) fn merge_distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    // Promote before subtraction so the tolerance remains meaningful for both
    // tiny fields and translated fields.
    norm(sub(a.map(f64::from), b.map(f64::from)))
}

fn polygon_is_nondegenerate(points: &[[f32; 3]]) -> bool {
    match points {
        [a, b, c] => triangle_is_nondegenerate([*a, *b, *c]),
        [a, b, c, d] => select_quad_diagonal([*a, *b, *c, *d]).is_some(),
        _ => false,
    }
}

pub(super) fn transition_is_nondegenerate(face: &[VertexEntry]) -> bool {
    match face {
        [a, b, c] => triangle_is_nondegenerate([a.position, b.position, c.position]),
        [a, b, c, d] => {
            select_quad_diagonal([a.position, b.position, c.position, d.position]).is_some()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{DualContourError, MeshBuilder, RunContext, collapse_coincident_edges};
    use crate::ExtractionLimits;
    use exedra_mesh::{BuildError, op::CollapseEdgeError};

    #[test]
    fn coincident_edge_does_not_destroy_a_closed_shell() {
        let mut builder = MeshBuilder::new();
        for p in [[0.0; 3], [0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            builder.push_vertex(p);
        }
        for face in [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]] {
            builder.add_face(&face).unwrap();
        }
        let mut mesh = builder.build().unwrap().mesh;
        assert!(matches!(
            collapse_coincident_edges(
                &mut mesh,
                0.0,
                false,
                &RunContext::new(ExtractionLimits::default())
            ),
            Err(DualContourError {
                kind: crate::DualContourErrorKind::Collapse(
                    CollapseEdgeError::DegenerateShell { .. }
                ),
                ..
            })
        ));
        assert_eq!(mesh.faces().count(), 4);
        assert!(mesh.boundary_loops().unwrap().is_empty());
    }

    #[test]
    fn join_chains_cannot_walk_beyond_the_original_tolerance() {
        let mut builder = MeshBuilder::new();
        for p in [
            [2e-7, 0.0, 0.0],
            [1e-7, 0.0, 0.0],
            [0.0; 3],
            [0.0, 1.0, 0.0],
        ] {
            builder.push_vertex(p);
        }
        builder.add_face(&[2, 1, 3]).unwrap();
        builder.add_face(&[1, 0, 3]).unwrap();
        let mut mesh = builder.build().unwrap().mesh;
        // The first join moves x=0 to x=1e-7. A second hop to x=2e-7
        // would pass an edge-length-only check but exceed the original bound.
        assert!(matches!(
            collapse_coincident_edges(
                &mut mesh,
                1.5e-7,
                false,
                &RunContext::new(ExtractionLimits::default())
            ),
            Err(DualContourError {
                kind: crate::DualContourErrorKind::Build(BuildError::DegenerateTriangle { .. }),
                ..
            })
        ));
        assert_eq!(mesh.faces().count(), 1);
        assert!(
            mesh.vertices()
                .any(|v| mesh.vertex_position(v) == Some(&[1e-7, 0.0, 0.0]))
        );
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn joined_region_and_fused_sharpness_survive_retriangulation() {
        use super::triangulate_joined_mesh;
        use exedra_mesh::{FaceBuildAttrs, attr};
        let mut builder = MeshBuilder::new();
        for p in [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            builder.push_vertex(p);
        }
        builder
            .add_face_with_attrs(
                &[0, 1, 4],
                &FaceBuildAttrs {
                    region: Some(11),
                    edge_sharpness: Some(&[0.0, 4.0, 8.0]),
                    ..FaceBuildAttrs::default()
                },
            )
            .unwrap();
        builder
            .add_face_with_attrs(
                &[1, 2, 3, 4],
                &FaceBuildAttrs {
                    region: Some(22),
                    edge_sharpness: Some(&[1.0, 2.0, 3.0, 4.0]),
                    ..FaceBuildAttrs::default()
                },
            )
            .unwrap();
        let mut mesh = builder.build().unwrap().mesh;
        let run = RunContext::new(ExtractionLimits::default());
        assert_eq!(
            collapse_coincident_edges(&mut mesh, 0.0, true, &run).unwrap(),
            (1, 0.0)
        );
        let mesh = triangulate_joined_mesh(&mesh, true, &run).unwrap();
        assert!(mesh.validate_deep().is_empty());
        assert_eq!(mesh.faces().count(), 2);
        assert!(mesh.faces().all(|f| {
            mesh.attrs()
                .dense(attr::FACE_REGION)
                .unwrap()
                .get(f.as_id())
                == Some(&22)
        }));
        for h in mesh.half_edges() {
            let a = *mesh.vertex_position(mesh.from_vertex(h).unwrap()).unwrap();
            let b = *mesh.vertex_position(mesh.to_vertex(h).unwrap()).unwrap();
            let expected = if a[1] == 0.0 && b[1] == 0.0 {
                1.0
            } else if a[0] == 1.0 && b[0] == 1.0 {
                2.0
            } else if a[1] == 1.0 && b[1] == 1.0 {
                3.0
            } else if a[0] == 0.0 && b[0] == 0.0 {
                8.0
            } else {
                0.0
            };
            assert_eq!(
                mesh.edge_sharpness(h).unwrap_or(0.0),
                expected,
                "a={a:?}, b={b:?}"
            );
        }
    }
}
