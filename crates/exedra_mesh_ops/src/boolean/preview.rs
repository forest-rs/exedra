// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Surface-intersection preview using the Boolean pipeline's existing stages.

use super::{
    BooleanBvh, BooleanDiagnostics, BooleanScratch, build_intersection_graph, narrow_phase,
};
use alloc::vec::Vec;
use exedra_mesh::{FaceTriangulation, Mesh};

/// Surface intersection evidence without splitting, classification or stitching.
#[derive(Debug)]
pub struct IntersectionPreview {
    /// Number of broad-phase triangle pairs considered by the narrow phase.
    pub candidate_pairs: usize,
    /// Extracted intersection polylines in mesh coordinates.
    /// Diagnostics may identify unsupported or deferred contacts.
    pub curves: Vec<Vec<[f64; 3]>>,
}

/// Traces candidate surface intersections without editing either operand.
///
/// An empty result does not establish that a Boolean is a no-op: containment,
/// disjoint union and deferred coplanar contacts require the full operation.
/// This uses the same triangulation, scratch and diagnostics as the staged
/// Boolean pipeline; it is not a checked-solid validity query.
#[must_use]
pub fn preview_intersections(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    strategy: FaceTriangulation,
    scratch: &mut BooleanScratch,
    diagnostics: &mut BooleanDiagnostics,
) -> IntersectionPreview {
    // Discover overlapping triangle bounds before tracing surface segments.
    let bvh_a = BooleanBvh::build(mesh_a, strategy, scratch);
    let bvh_b = BooleanBvh::build(mesh_b, strategy, scratch);
    let mut pairs = Vec::new();
    let stats = bvh_a.query_overlaps(&bvh_b, scratch, &mut pairs);
    let bounds_overlap = stats.candidate_pairs > 0;

    let mut curves = Vec::new();
    if bounds_overlap {
        let mut segments = Vec::new();
        let _ = narrow_phase(
            mesh_a,
            mesh_b,
            &pairs,
            strategy,
            scratch,
            &mut segments,
            diagnostics,
        );
        let graph =
            build_intersection_graph(mesh_a, mesh_b, &segments, strategy, scratch, diagnostics);
        for polyline in &graph.polylines {
            curves.push(
                polyline
                    .vertices
                    .iter()
                    .map(|&v| graph.vertices[v as usize].position)
                    .collect(),
            );
        }
    }

    IntersectionPreview {
        candidate_pairs: pairs.len(),
        curves,
    }
}
