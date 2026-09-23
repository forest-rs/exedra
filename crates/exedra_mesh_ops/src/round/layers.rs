// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Caller-defined layer transfer for the staged rounding rewrite.
//!
//! Every rewritten face owns its first source face (as for regions and UV
//! charts). Face values come from that owner. A corner at a surviving vertex
//! takes the owner's corner there (or the first source face that has one); a
//! corner or vertex at a new point takes the owner's corners or vertices
//! weighted by the point's barycentric coordinates in the owner's robust
//! triangulation (see [`crate::layers`]).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use exedra_math::promote;
use exedra_mesh::attributes::CapturedAttributes;
use exedra_mesh::{HalfEdgeId, Mesh, VertexId};

use super::{NewFace, Tok};
use crate::layers::{Chart, has_caller_layers};

/// Captured caller-defined values for every planned face and new point.
pub(super) struct Captured {
    /// Per planned face: face values and per-entry corner values.
    pub(super) faces: Vec<(CapturedAttributes, Vec<CapturedAttributes>)>,
    /// Per new point: vertex values, from the first planned face using it.
    pub(super) points: Vec<Option<CapturedAttributes>>,
}

/// Returns `None` when the mesh has no caller-defined layer, so the ordinary
/// path does no work.
pub(super) fn capture(mesh: &Mesh, points: &[[f64; 3]], faces: &[NewFace]) -> Option<Captured> {
    if !has_caller_layers(mesh) {
        return None;
    }
    let mut charts = BTreeMap::new();
    let mut point_values = alloc::vec![None; points.len()];
    let mut face_values = Vec::with_capacity(faces.len());
    for face in faces {
        let owner = face.source.faces()[0];
        let chart = charts
            .entry(owner)
            .or_insert_with(|| Chart::new(mesh, owner));
        let corners = face
            .entries
            .iter()
            .map(|entry| match *entry {
                Tok::Old(vertex) => {
                    let corner = face.source.faces().iter().find_map(|&source| {
                        mesh.face_loop(source)
                            .find(|&corner| mesh.to_vertex(corner) == Some(vertex))
                    });
                    match corner {
                        Some(corner) => mesh.capture_attributes(&[(corner, 1.0)]),
                        None => mesh.capture_attributes::<HalfEdgeId>(&[]),
                    }
                }
                Tok::New(point) => {
                    // Sample where the vertex will actually land: the new
                    // point is stored as f32, so narrow it before weighting.
                    let weights =
                        chart.weights(promote(exedra_math::narrow(points[point as usize])));
                    let slot = &mut point_values[point as usize];
                    if slot.is_none() {
                        let vertices: Vec<(VertexId, f32)> = weights
                            .iter()
                            .map(|&(corner, weight)| {
                                (mesh.to_vertex(corner).expect("live source corner"), weight)
                            })
                            .collect();
                        *slot = Some(mesh.capture_attributes(&vertices));
                    }
                    mesh.capture_attributes(&weights)
                }
            })
            .collect();
        face_values.push((mesh.capture_attributes(&[(owner, 1.0)]), corners));
    }
    Some(Captured {
        faces: face_values,
        points: point_values,
    })
}
