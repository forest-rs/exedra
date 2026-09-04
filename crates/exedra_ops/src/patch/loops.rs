// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

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
    InvalidSelectedPatch,
    AmbiguousBoundaryVertex { vertex: VertexId, candidates: usize },
    OpenBoundaryChain { start: VertexId, end: VertexId },
}

pub(crate) fn extract_boundary_loops(
    mesh: &exedra::Mesh,
    region: &SelectedFaceRegion,
) -> Result<Vec<BoundaryLoop>, BoundaryLoopError> {
    let selected_faces = region
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<Vec<_>>();
    let boundary_loops = mesh
        .selected_face_boundary_loops(&selected_faces)
        .map_err(map_boundary_error)?;

    boundary_loops
        .into_iter()
        .map(|boundary_loop| {
            let edges = boundary_loop
                .into_iter()
                .map(|edge| {
                    region
                        .boundary_edges
                        .iter()
                        .copied()
                        .find(|candidate| candidate.edge == edge)
                        .ok_or(BoundaryLoopError::InvalidSelectedPatch)
                })
                .collect::<Result<Vec<FaceEdgeRef>, _>>()?;
            Ok(BoundaryLoop { edges })
        })
        .collect()
}

fn map_boundary_error(error: exedra::SelectedFaceBoundaryError) -> BoundaryLoopError {
    match error {
        exedra::SelectedFaceBoundaryError::AmbiguousBoundaryVertex { vertex, candidates } => {
            BoundaryLoopError::AmbiguousBoundaryVertex { vertex, candidates }
        }
        exedra::SelectedFaceBoundaryError::OpenBoundaryChain { start, end } => {
            BoundaryLoopError::OpenBoundaryChain { start, end }
        }
        exedra::SelectedFaceBoundaryError::OutsideFaceInSelection
        | exedra::SelectedFaceBoundaryError::StaleFace { .. }
        | exedra::SelectedFaceBoundaryError::InvalidFaceLoop { .. } => {
            BoundaryLoopError::InvalidSelectedPatch
        }
    }
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

        let loops = extract_boundary_loops(&mesh, &region).expect("loop extraction should succeed");
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

        let loops = extract_boundary_loops(&mesh, &region).expect("loop extraction should succeed");
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

        let loops = extract_boundary_loops(&mesh, &region).expect("loop extraction should succeed");
        assert_eq!(loops.len(), 2);
        assert_eq!(loops[0].edges.len(), 3);
        assert_eq!(loops[1].edges.len(), 3);
        assert!(
            loops[0].edges[0].face.index() < loops[1].edges[0].face.index(),
            "loop ordering should follow stable seed ordering"
        );
    }
}
