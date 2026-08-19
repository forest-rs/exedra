// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Boolean stitch: assembling the result mesh from classified patches.
//!
//! [`boolean_mesh`] runs the whole pipeline — broad phase, narrow phase,
//! intersection graph, splitting, classification, stitching — and returns
//! one watertight result mesh with full provenance. Patch selection per
//! operation:
//!
//! - **Union**: outside patches of both meshes.
//! - **Intersection**: inside patches of both meshes.
//! - **Difference (A − B)**: A's outside patches plus B's inside patches
//!   with their face loops reversed (their outward normals point out of B,
//!   which is *into* the difference solid).
//!
//! Welding along the cut curves is identity-based, never positional: a
//! graph vertex materialized on both meshes becomes exactly one output
//! vertex, and every other vertex maps per source mesh. The weld-seam
//! crease policy tags every cut edge with full sharpness and a seam mark —
//! the seam is a real feature line of the new solid.
//!
//! Suspect patches are typed poison: the operation fails with
//! [`BooleanError::SuspectPatches`] rather than emitting geometry that
//! might be wrong. An empty result (for example the intersection of
//! disjoint solids) is *not* an error — it is the true answer.

use alloc::vec::Vec;

use hashbrown::HashMap;

use super::classify::{PatchClassification, PatchSide, classify_patches};
use super::diag::BooleanDiagnostics;
use super::graph::{IntersectionGraph, build_intersection_graph};
use super::narrow::narrow_phase;
use super::split::{MeshSide, MeshSplitOutcome, split_mesh_along_graph};
use super::{BooleanBvh, BooleanScratch};
use crate::{
    BuildError, FaceBuildAttrs, FaceId, FaceTriangulation, Mesh, MeshBuilder, VertexId, attr,
    op::{set_edge_seam, set_edge_sharpness},
};

/// The boolean operation to perform.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BooleanOp {
    /// Everything in either solid.
    Union,
    /// Everything in both solids.
    Intersection,
    /// Everything in A but not in B.
    Difference,
}

/// Deterministic stitch counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct BooleanStats {
    /// Narrow-phase segments produced.
    pub segments: u64,
    /// Patches classified across both meshes.
    pub patches: u64,
    /// Faces kept into the output.
    pub kept_faces: u64,
    /// Output vertices welded from both meshes' cut curves.
    pub welded_vertices: u64,
    /// Seam edges tagged in the output.
    pub seam_edges: u64,
}

/// The stitched boolean result.
#[derive(Clone, Debug)]
pub struct BooleanOutput {
    /// The result mesh (validated by the pipeline's tests; callers may
    /// re-validate).
    pub mesh: Mesh,
    /// `(output face, operand side, original pre-split face)` for every
    /// output face, in face-creation order — composed through the split
    /// stage's origin mapping, so constructive source maps can attribute
    /// boolean faces to operand features.
    pub face_provenance: Vec<(FaceId, MeshSide, FaceId)>,
    /// The patch classification the selection was made from.
    pub classification: PatchClassification,
    /// Stitch counters.
    pub stats: BooleanStats,
}

/// Typed boolean failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BooleanError {
    /// Classification produced suspect patches; the result would be
    /// unreliable. The diagnostics accumulator holds the details.
    SuspectPatches {
        /// Number of suspect patches.
        count: u64,
    },
    /// The output mesh could not be rebuilt (an internal invariant
    /// violation, not an input problem).
    Build(BuildError),
}

impl core::fmt::Display for BooleanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SuspectPatches { count } => {
                write!(f, "{count} suspect patches; boolean result withheld")
            }
            Self::Build(e) => write!(f, "stitch rebuild failed: {e:?}"),
        }
    }
}

impl core::error::Error for BooleanError {}

/// Runs the full boolean pipeline on two meshes.
///
/// Deterministic for fixed inputs: every stage orders its work canonically
/// and every geometric decision is exact. Trouble is typed — diagnostics
/// accumulate in `diagnostics`, suspect classifications fail the operation,
/// and nothing is silently approximated.
///
/// # Errors
///
/// [`BooleanError::SuspectPatches`] when classification could not soundly
/// decide every patch (deferred splits, exhausted rays, coplanar overlap);
/// [`BooleanError::Build`] when output assembly violates an invariant.
pub fn boolean_mesh(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    op: BooleanOp,
    strategy: FaceTriangulation,
    scratch: &mut BooleanScratch,
    diagnostics: &mut BooleanDiagnostics,
) -> Result<BooleanOutput, BooleanError> {
    // Splitting mutates: the pipeline works on private clones.
    let mut split_a = mesh_a.clone();
    let mut split_b = mesh_b.clone();

    let bvh_a = BooleanBvh::build(&split_a, strategy, scratch);
    let bvh_b = BooleanBvh::build(&split_b, strategy, scratch);
    let mut pairs = Vec::new();
    bvh_a.query_overlaps(&bvh_b, scratch, &mut pairs);

    let mut segments = Vec::new();
    narrow_phase(
        &split_a,
        &split_b,
        &pairs,
        strategy,
        scratch,
        &mut segments,
        diagnostics,
    );
    let graph = build_intersection_graph(
        &split_a,
        &split_b,
        &segments,
        strategy,
        scratch,
        diagnostics,
    );
    let outcome_a = split_mesh_along_graph(&mut split_a, &graph, MeshSide::A, diagnostics);
    let outcome_b = split_mesh_along_graph(&mut split_b, &graph, MeshSide::B, diagnostics);
    let classification = classify_patches(
        &split_a,
        &split_b,
        &graph,
        &outcome_a,
        &outcome_b,
        strategy,
        scratch,
        diagnostics,
    );
    if classification.any_suspect() {
        return Err(BooleanError::SuspectPatches {
            count: classification.stats.suspect_patches,
        });
    }

    let mut stats = BooleanStats {
        segments: segments.len() as u64,
        patches: classification.patches.len() as u64,
        ..BooleanStats::default()
    };

    stitch(
        op,
        &split_a,
        &split_b,
        &graph,
        &outcome_a,
        &outcome_b,
        &classification,
        &mut stats,
    )
    .map(|(mesh, face_provenance)| BooleanOutput {
        mesh,
        face_provenance,
        classification,
        stats,
    })
    .map_err(BooleanError::Build)
}

/// Which side a patch must be on to be kept, and whether its faces flip.
fn selection(op: BooleanOp, mesh: MeshSide) -> (PatchSide, bool) {
    match (op, mesh) {
        (BooleanOp::Union, _) => (PatchSide::Outside, false),
        (BooleanOp::Intersection, _) => (PatchSide::Inside, false),
        (BooleanOp::Difference, MeshSide::A) => (PatchSide::Outside, false),
        (BooleanOp::Difference, MeshSide::B) => (PatchSide::Inside, true),
    }
}

type Provenance = Vec<(FaceId, MeshSide, FaceId)>;

#[expect(
    clippy::too_many_arguments,
    reason = "internal stage threading fixed pipeline context"
)]
fn stitch(
    op: BooleanOp,
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    graph: &IntersectionGraph,
    outcome_a: &MeshSplitOutcome,
    outcome_b: &MeshSplitOutcome,
    classification: &PatchClassification,
    stats: &mut BooleanStats,
) -> Result<(Mesh, Provenance), BuildError> {
    let mut builder = MeshBuilder::new();

    // --- Identity-based vertex maps, seam vertices pre-welded: one output
    // vertex per graph vertex materialized on both meshes.
    let mut map_a: HashMap<VertexId, u32> = HashMap::new();
    let mut map_b: HashMap<VertexId, u32> = HashMap::new();
    let mut seam_builder_indices: Vec<Option<u32>> = alloc::vec![None; graph.vertices.len()];
    for (index, slot) in seam_builder_indices.iter_mut().enumerate() {
        if let (Some(va), Some(vb)) = (
            outcome_a.graph_vertices[index],
            outcome_b.graph_vertices[index],
        ) {
            let Some(position) = mesh_a.vertex_position(va) else {
                continue;
            };
            debug_assert_eq!(
                mesh_b.vertex_position(vb),
                Some(position),
                "both sides materialize the identical narrowed position"
            );
            let builder_index = builder.push_vertex(*position);
            map_a.insert(va, builder_index);
            map_b.insert(vb, builder_index);
            *slot = Some(builder_index);
            stats.welded_vertices += 1;
        }
    }

    // --- Kept faces in deterministic order (classification is already
    // mesh A first, patches in lowest-face order, faces ascending).
    // Captured boundary attributes re-apply after the build.
    let mut sources: Vec<(MeshSide, FaceId)> = Vec::new();
    let mut captured_attrs: Vec<(u32, u32, Option<f32>, Option<bool>)> = Vec::new();
    for patch in &classification.patches {
        let (keep_side, flip) = selection(op, patch.mesh);
        if patch.side != keep_side {
            continue;
        }
        let (mesh, map) = match patch.mesh {
            MeshSide::A => (mesh_a, &mut map_a),
            MeshSide::B => (mesh_b, &mut map_b),
        };
        for &face in &patch.faces {
            let mut loop_indices: Vec<u32> = Vec::new();
            for half_edge in mesh.face_loop(face) {
                let Some(vertex) = mesh.to_vertex(half_edge) else {
                    continue;
                };
                let index = *map.entry(vertex).or_insert_with(|| {
                    let position = mesh
                        .vertex_position(vertex)
                        .copied()
                        .unwrap_or([0.0, 0.0, 0.0]);
                    builder.push_vertex(position)
                });
                loop_indices.push(index);
            }
            // Capture boundary sharpness/seams for re-application (loop
            // edge k runs corner k -> corner k+1).
            let count = loop_indices.len();
            for (k, half_edge) in mesh.face_loop(face).enumerate() {
                let sharpness = mesh.edge_sharpness(half_edge).filter(|s| *s != 0.0);
                let seam = mesh.edge_seam(half_edge).filter(|s| *s);
                if sharpness.is_some() || seam.is_some() {
                    // face_loop yields the edge INTO each corner in loop
                    // order aligned with `to_vertex`; the undirected pair
                    // is (previous corner, this corner).
                    let to = loop_indices[k];
                    let from = loop_indices[(k + count - 1) % count];
                    captured_attrs.push((from, to, sharpness, seam));
                }
            }
            if flip {
                loop_indices.reverse();
            }
            let region = mesh
                .attrs()
                .dense(attr::FACE_REGION)
                .and_then(|layer| layer.get(face.as_id()).copied());
            builder.add_face_with_attrs(
                &loop_indices,
                &FaceBuildAttrs {
                    region,
                    ..FaceBuildAttrs::default()
                },
            )?;
            sources.push((patch.mesh, face));
            stats.kept_faces += 1;
        }
    }

    let result = builder.build()?;
    let mut mesh = result.mesh;

    // --- Compose provenance through the split stage's origin mapping.
    let origins_a: HashMap<FaceId, FaceId> = outcome_a.face_origins.iter().copied().collect();
    let origins_b: HashMap<FaceId, FaceId> = outcome_b.face_origins.iter().copied().collect();
    let face_provenance: Provenance = sources
        .iter()
        .zip(&result.face_ids)
        .map(|(&(side, face), &output)| {
            let original = match side {
                MeshSide::A => origins_a.get(&face).copied().unwrap_or(face),
                MeshSide::B => origins_b.get(&face).copied().unwrap_or(face),
            };
            (output, side, original)
        })
        .collect();

    // --- Weld-seam crease policy plus captured attribute re-application.
    let vertex_of = |index: u32| result.vertex_ids.get(index as usize).copied();
    {
        let mut session = mesh.edit();
        for polyline in &graph.polylines {
            let count = polyline.vertices.len();
            let edges = if polyline.closed {
                count
            } else {
                count.saturating_sub(1)
            };
            for i in 0..edges {
                let p = polyline.vertices[i] as usize;
                let q = polyline.vertices[(i + 1) % count] as usize;
                let pair = seam_builder_indices[p]
                    .zip(seam_builder_indices[q])
                    .and_then(|(bp, bq)| vertex_of(bp).zip(vertex_of(bq)));
                let Some((u, v)) = pair else {
                    continue;
                };
                if let Some(half_edge) = find_half_edge(session.mesh(), u, v) {
                    let _ = set_edge_sharpness(&mut session, half_edge, 1.0);
                    let _ = set_edge_seam(&mut session, half_edge, true);
                    stats.seam_edges += 1;
                }
            }
        }
        for (from, to, sharpness, seam) in captured_attrs {
            let pair = vertex_of(from).zip(vertex_of(to));
            let Some((u, v)) = pair else {
                continue;
            };
            if let Some(half_edge) = find_half_edge(session.mesh(), u, v) {
                if let Some(sharpness) = sharpness {
                    let _ = set_edge_sharpness(&mut session, half_edge, sharpness);
                }
                if let Some(seam) = seam {
                    let _ = set_edge_seam(&mut session, half_edge, seam);
                }
            }
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
    }

    Ok((mesh, face_provenance))
}

/// Finds a half-edge between `from` and `to` in either direction. Linear
/// scan (v1; stitching is a construction stage).
fn find_half_edge(mesh: &Mesh, from: VertexId, to: VertexId) -> Option<crate::HalfEdgeId> {
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            let ends = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge));
            if ends == (Some(from), Some(to)) || ends == (Some(to), Some(from)) {
                return Some(half_edge);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::set_face_region;

    fn cube(origin: [f32; 3]) -> Mesh {
        let o = origin;
        let positions = [
            [o[0], o[1], o[2]],
            [o[0] + 1.0, o[1], o[2]],
            [o[0] + 1.0, o[1] + 1.0, o[2]],
            [o[0], o[1] + 1.0, o[2]],
            [o[0], o[1], o[2] + 1.0],
            [o[0] + 1.0, o[1], o[2] + 1.0],
            [o[0] + 1.0, o[1] + 1.0, o[2] + 1.0],
            [o[0], o[1] + 1.0, o[2] + 1.0],
        ];
        let faces: [[u32; 4]; 6] = [
            [3, 2, 1, 0],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(&face).expect("valid cube face");
        }
        builder.build().expect("valid cube").mesh
    }

    fn signed_volume(mesh: &Mesh) -> f64 {
        let mut volume = 0.0;
        for face in mesh.faces() {
            let corners: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                .collect();
            for i in 1..corners.len().saturating_sub(1) {
                let (a, b, c) = (corners[0], corners[i], corners[i + 1]);
                volume += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]);
            }
        }
        volume / 6.0
    }

    fn run(op: BooleanOp) -> (BooleanOutput, BooleanDiagnostics) {
        let mut mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, 0.5, 0.5]);
        // Distinct regions on A so survival is observable.
        {
            let mut session = mesh_a.edit();
            let faces: Vec<FaceId> = session.mesh().faces().collect();
            for (index, face) in faces.into_iter().enumerate() {
                let region = u32::try_from(index).expect("small") + 10;
                set_face_region(&mut session, face, region).expect("live face");
            }
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
        }
        let mut scratch = BooleanScratch::new();
        let mut diagnostics = BooleanDiagnostics::default();
        let output = boolean_mesh(
            &mesh_a,
            &mesh_b,
            op,
            FaceTriangulation::Fan,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("two-cube boolean succeeds");
        (output, diagnostics)
    }

    #[test]
    fn union_is_watertight_with_correct_volume() {
        let (output, diagnostics) = run(BooleanOp::Union);
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // V = 1 + 1 - 0.125.
        let volume = signed_volume(&output.mesh);
        assert!((volume - 1.875).abs() < 1e-6, "union volume {volume}");
        assert!(output.stats.seam_edges >= 6, "hexagonal seam tagged");
    }

    #[test]
    fn intersection_is_watertight_with_correct_volume() {
        let (output, _) = run(BooleanOp::Intersection);
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        let volume = signed_volume(&output.mesh);
        assert!(
            (volume - 0.125).abs() < 1e-6,
            "intersection volume {volume}"
        );
    }

    #[test]
    fn difference_flips_b_and_has_correct_volume() {
        let (output, _) = run(BooleanOp::Difference);
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // V = 1 - 0.125; positive volume proves B's kept faces flipped.
        let volume = signed_volume(&output.mesh);
        assert!((volume - 0.875).abs() < 1e-6, "difference volume {volume}");
    }

    #[test]
    fn provenance_covers_every_output_face_and_regions_survive() {
        let (output, _) = run(BooleanOp::Union);
        assert_eq!(
            output.face_provenance.len(),
            output.mesh.faces().count(),
            "every output face is attributed"
        );
        // A-side output faces carry their original faces' regions (>= 10).
        let regions = output
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("region layer");
        let mut a_faces = 0;
        for &(face, side, original) in &output.face_provenance {
            if side == MeshSide::A {
                let region = regions.get(face.as_id()).copied().expect("region value");
                assert_eq!(
                    region,
                    10 + original.index(),
                    "region must trace to the original face"
                );
                a_faces += 1;
            }
        }
        assert!(a_faces > 0);
        // Originals are pre-split faces: indices within the original six.
        for &(_, _, original) in &output.face_provenance {
            assert!(original.index() < 6, "original {original:?} is pre-split");
        }
    }

    #[test]
    fn seam_edges_are_tagged_sharp_and_seam() {
        let (output, _) = run(BooleanOp::Union);
        let mesh = &output.mesh;
        let mut tagged = 0;
        for face in mesh.faces() {
            for half_edge in mesh.face_loop(face) {
                if mesh.edge_sharpness(half_edge).unwrap_or(0.0) > 0.5
                    && mesh.edge_seam(half_edge) == Some(true)
                {
                    tagged += 1;
                }
            }
        }
        // Each seam edge is visited from both adjacent faces.
        assert!(tagged >= 12, "seam edges tagged sharp+seam, got {tagged}");
    }

    #[test]
    fn boolean_is_deterministic() {
        let (first, _) = run(BooleanOp::Difference);
        let (second, _) = run(BooleanOp::Difference);
        assert_eq!(first.face_provenance, second.face_provenance);
        assert_eq!(first.stats, second.stats);
        let snapshot = |mesh: &Mesh| {
            let faces: Vec<Vec<u32>> = mesh
                .faces()
                .map(|face| {
                    mesh.face_loop(face)
                        .filter_map(|he| mesh.to_vertex(he))
                        .map(|v| v.index())
                        .collect()
                })
                .collect();
            let positions: Vec<[u32; 3]> = mesh
                .vertices()
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
                .collect();
            (faces, positions)
        };
        assert_eq!(snapshot(&first.mesh), snapshot(&second.mesh));
    }

    #[test]
    fn disjoint_intersection_is_empty_not_an_error() {
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([5.0, 5.0, 5.0]);
        let mut scratch = BooleanScratch::new();
        let mut diagnostics = BooleanDiagnostics::default();
        let output = boolean_mesh(
            &mesh_a,
            &mesh_b,
            BooleanOp::Intersection,
            FaceTriangulation::Fan,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("disjoint intersection is a valid empty result");
        assert_eq!(output.mesh.faces().count(), 0);

        let union = boolean_mesh(
            &mesh_a,
            &mesh_b,
            BooleanOp::Union,
            FaceTriangulation::Fan,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("disjoint union keeps both shells");
        assert_eq!(union.mesh.faces().count(), 12);
        assert!(union.mesh.validate_deep().is_empty());
        let volume = signed_volume(&union.mesh);
        assert!((volume - 2.0).abs() < 1e-6);
    }
}
