// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use exedra::VertexId;

use crate::patch::region::{FaceEdgeRef, SelectedFaceRegion};

#[derive(Clone, Debug)]
pub(crate) struct BoundaryLoop {
    pub(crate) edges: Vec<FaceEdgeRef>,
}

impl BoundaryLoop {
    #[cfg(test)]
    pub(crate) fn vertices(&self) -> Vec<VertexId> {
        self.edges.iter().map(|edge| edge.from).collect()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum BoundaryLoopError {
    AmbiguousBoundaryVertex { vertex: VertexId, candidates: usize },
    OpenBoundaryChain { start: VertexId, end: VertexId },
}

pub(crate) fn extract_boundary_loops(
    region: &SelectedFaceRegion,
) -> Result<Vec<BoundaryLoop>, BoundaryLoopError> {
    let mut outgoing = BTreeMap::<VertexId, Vec<usize>>::new();
    for (index, edge) in region.boundary_edges.iter().enumerate() {
        outgoing.entry(edge.from).or_default().push(index);
    }
    for candidates in outgoing.values_mut() {
        candidates.sort_by_key(|&index| {
            let edge = region.boundary_edges[index];
            (edge.face.index(), edge.edge_index, edge.edge.index())
        });
    }

    let mut visited = vec![false; region.boundary_edges.len()];
    let mut loops = Vec::<BoundaryLoop>::new();
    for seed_index in 0..region.boundary_edges.len() {
        if visited[seed_index] {
            continue;
        }
        let seed = region.boundary_edges[seed_index];
        let mut current = seed;
        let mut current_index = seed_index;
        let mut loop_edges = Vec::<FaceEdgeRef>::new();

        loop {
            visited[current_index] = true;
            loop_edges.push(current);

            if current.to == seed.from {
                break;
            }

            let candidates = outgoing
                .get(&current.to)
                .map(|indices| {
                    indices
                        .iter()
                        .copied()
                        .filter(|&index| !visited[index])
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match candidates.as_slice() {
                [next_index] => {
                    current_index = *next_index;
                    current = region.boundary_edges[*next_index];
                }
                [] => {
                    return Err(BoundaryLoopError::OpenBoundaryChain {
                        start: seed.from,
                        end: current.to,
                    });
                }
                many => {
                    return Err(BoundaryLoopError::AmbiguousBoundaryVertex {
                        vertex: current.to,
                        candidates: many.len(),
                    });
                }
            }
        }

        loops.push(BoundaryLoop { edges: loop_edges });
    }

    loops.sort_by_key(|boundary_loop| {
        let seed = boundary_loop
            .edges
            .first()
            .expect("boundary loops always contain at least one edge");
        (seed.face.index(), seed.edge_index, seed.edge.index())
    });
    Ok(loops)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra::{BuildParams, Mesh};

    use super::extract_boundary_loops;
    use crate::OpContext;
    use crate::patch::region::selected_face_region;

    #[test]
    fn single_face_region_extracts_one_loop() {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let ctx = OpContext::default();
        let region = selected_face_region(&mesh, &[face], false, &ctx).expect("region should load");

        let loops = extract_boundary_loops(&region).expect("loop extraction should succeed");
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].edges.len(), 4);
        assert_eq!(loops[0].vertices().len(), 4);
    }

    #[test]
    fn adjacent_faces_extract_one_shared_boundary_loop() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let ctx = OpContext::default();
        let region = selected_face_region(&mesh, &faces, false, &ctx).expect("region should load");

        let loops = extract_boundary_loops(&region).expect("loop extraction should succeed");
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].edges.len(), 4);
    }

    #[test]
    fn disjoint_faces_extract_multiple_loops_in_stable_order() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 1.0, 0.0],
                [3.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [4, 5, 6]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let ctx = OpContext::default();
        let region = selected_face_region(&mesh, &faces, false, &ctx).expect("region should load");

        let loops = extract_boundary_loops(&region).expect("loop extraction should succeed");
        assert_eq!(loops.len(), 2);
        assert_eq!(loops[0].edges.len(), 3);
        assert_eq!(loops[1].edges.len(), 3);
        assert!(
            loops[0].edges[0].face.index() < loops[1].edges[0].face.index(),
            "loop ordering should follow stable seed ordering"
        );
    }
}
