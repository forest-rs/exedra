// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Boolean stitch: assembling the result mesh from classified patches.
//!
//! [`boolean_mesh`] runs the whole pipeline — broad phase, narrow phase,
//! coplanar contact detection, intersection graph, splitting,
//! classification, stitching — and returns one watertight result mesh
//! with full provenance. Patch selection per operation:
//!
//! - **Union**: outside patches of both meshes.
//! - **Intersection**: inside patches of both meshes.
//! - **Difference (A − B)**: A's outside patches plus B's inside patches
//!   with their face loops reversed (their outward normals point out of B,
//!   which is *into* the difference solid).
//!
//! Coplanar face-on-face contact regions ([`PatchSide::Boundary`]) follow
//! solid-boundary semantics. With **opposed** outward normals the
//! operands' interiors lie on opposite sides of the contact plane; with
//! **same** normals both interiors lie on the same side. Testing which
//! sides of the region belong to the result gives the selection table
//! (derivation in the ticket notes for `exe-45bt`):
//!
//! | op               | opposed normals | same normals |
//! |------------------|-----------------|--------------|
//! | Union            | drop both       | keep A copy  |
//! | Intersection     | drop both       | keep A copy  |
//! | Difference (A−B) | keep A copy     | drop both    |
//!
//! B's copy is never kept: whenever the region bounds the result, its
//! orientation matches A's copy (Difference would flip B's copy onto A's
//! anyway), so keeping the A copy is the deterministic choice. Union of
//! two solids touching along a shared wall therefore removes the wall
//! entirely; a flush-face difference opens the shared wall into the cut.
//!
//! Welding along the cut curves is identity-based, never positional: a
//! graph vertex materialized on both meshes becomes exactly one output
//! vertex, and every other vertex maps per source mesh. The weld-seam
//! crease policy tags every cut edge with full sharpness and a seam mark —
//! the seam is a real feature line of the new solid.
//! Graph points in one connected seam that narrow to one stored `f32` point
//! are representational aliases, however: they map to the lowest-index
//! component survivor. This is topology-constrained identity recovery, not
//! global coordinate welding; unrelated equal-position seams and shells
//! remain distinct.
//!
//! Zero-area operand contacts are regularized only when the half-edge model
//! can still represent the result. An isolated shared point retains one
//! shell-local vertex per operand, yielding two valid closed shells. A shared
//! edge would put four boundary faces on one edge, so it is refused as
//! [`BooleanError::NonManifoldContact`] instead of leaking a mesh-builder
//! invariant error.
//!
//! Suspect patches are typed poison: the operation fails with
//! [`BooleanError::SuspectPatches`] rather than emitting geometry that
//! might be wrong. An empty result (for example the intersection of
//! disjoint solids) is *not* an error — it is the true answer.

use alloc::vec::Vec;

use exedra_math::narrow;
use hashbrown::HashMap;

use super::classify::{PatchClassification, PatchSide, classify_patches};
use super::coplanar::{CoplanarContact, collect_coplanar_contacts};
use super::diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
use super::graph::{IntersectionGraph, MeshAnchor, build_intersection_graph};
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
    /// Splitting or classification left unresolved regions; the result would
    /// be unreliable. The diagnostics accumulator holds the details.
    SuspectPatches {
        /// Conservative count of unresolved regions.
        count: u64,
    },
    /// The operands meet along a zero-area edge whose selected volumetric
    /// boundary would be non-manifold. Both inputs may be valid manifold
    /// meshes; it is their contact that cannot be represented by this
    /// half-edge result.
    NonManifoldContact,
    /// The output mesh could not be rebuilt (an internal invariant
    /// violation, not an input problem).
    Build(BuildError),
    /// A pipeline stage diagnosed an internal invariant violation
    /// ([`super::BooleanFailureKind::InternalInvariantViolation`]); the
    /// assembled result would be unreliable, so no geometry is returned.
    InvariantViolation {
        /// Invariant-violation diagnostics recorded during this run.
        count: u64,
    },
}

impl core::fmt::Display for BooleanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SuspectPatches { count } => {
                write!(f, "{count} unresolved regions; boolean result withheld")
            }
            Self::NonManifoldContact => {
                f.write_str("operand edge contact would produce a non-manifold result")
            }
            Self::Build(e) => write!(f, "stitch rebuild failed: {e:?}"),
            Self::InvariantViolation { count } => {
                write!(
                    f,
                    "{count} internal invariant violations; boolean result withheld"
                )
            }
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
/// [`BooleanError::NonManifoldContact`] when otherwise-manifold operands meet
/// only along an edge that would have four incident result faces;
/// [`BooleanError::InvariantViolation`] when any stage diagnosed an
/// internal invariant violation (always a pipeline bug, never an input
/// problem — the result is withheld rather than returned wrong);
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
    let invariant_baseline = diagnostics.count_of(BooleanFailureKind::InternalInvariantViolation);
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
    // Coplanar contacts capture pre-split face polygons; this must run
    // before splitting mutates the meshes.
    let mut contacts = Vec::new();
    collect_coplanar_contacts(
        &split_a,
        &split_b,
        &pairs,
        strategy,
        scratch,
        &mut contacts,
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
        &contacts,
        strategy,
        scratch,
        diagnostics,
    );
    // Invariant-violation diagnostics are always bugs, never input
    // problems: a stage that recorded one has left its mesh in a state the
    // pipeline no longer vouches for, so the result is withheld even when
    // classification looks clean.
    let invariant_now = diagnostics.count_of(BooleanFailureKind::InternalInvariantViolation);
    if invariant_now > invariant_baseline {
        return Err(BooleanError::InvariantViolation {
            count: (invariant_now - invariant_baseline) as u64,
        });
    }
    let unresolved = unresolved_region_count(&outcome_a, &outcome_b, &classification);
    if unresolved > 0 {
        return Err(BooleanError::SuspectPatches { count: unresolved });
    }

    let mut stats = BooleanStats {
        segments: segments.len() as u64,
        patches: classification.patches.len() as u64,
        ..BooleanStats::default()
    };

    match stitch(
        op,
        mesh_a,
        mesh_b,
        &split_a,
        &split_b,
        &graph,
        &outcome_a,
        &outcome_b,
        &contacts,
        &classification,
        &mut stats,
    ) {
        Ok((mesh, face_provenance)) => Ok(BooleanOutput {
            mesh,
            face_provenance,
            classification,
            stats,
        }),
        Err(StitchError::NonManifoldContact) => {
            diagnostics.push(BooleanDiagnostic {
                kind: BooleanFailureKind::NonManifoldContact,
                a: None,
                b: None,
                detail: "selected shells share an edge with four incident boundary faces",
            });
            Err(BooleanError::NonManifoldContact)
        }
        Err(StitchError::Build(error)) => Err(BooleanError::Build(error)),
    }
}

/// Counts unresolved regions conservatively without double-counting the same
/// failed cut as both a deferred face and its downstream suspect patch.
///
/// Classification normally poisons every patch adjacent to a deferred split,
/// but that reconstruction is intentionally not the safety boundary: if a
/// graph vertex materializes elsewhere, classification may not be able to
/// identify the original unsplit face. Either stage reporting uncertainty is
/// therefore sufficient to withhold geometry.
fn unresolved_region_count(
    outcome_a: &MeshSplitOutcome,
    outcome_b: &MeshSplitOutcome,
    classification: &PatchClassification,
) -> u64 {
    let deferred_faces = outcome_a
        .stats
        .deferred_faces
        .saturating_add(outcome_b.stats.deferred_faces);
    deferred_faces.max(classification.stats.suspect_patches)
}

/// Whether a classified patch is kept, and whether its faces flip.
///
/// `None` drops the patch. Boundary (coplanar contact) patches follow the
/// module-level selection table: only the A copy is ever kept, and never
/// flipped (its outward normal already matches the result).
fn selection(op: BooleanOp, mesh: MeshSide, side: PatchSide) -> Option<bool> {
    match (side, op, mesh) {
        (PatchSide::Outside, BooleanOp::Union, _)
        | (PatchSide::Outside, BooleanOp::Difference, MeshSide::A)
        | (PatchSide::Inside, BooleanOp::Intersection, _) => Some(false),
        (PatchSide::Inside, BooleanOp::Difference, MeshSide::B) => Some(true),
        (PatchSide::Boundary { opposed }, op, MeshSide::A) => {
            let keep = match op {
                BooleanOp::Union | BooleanOp::Intersection => !opposed,
                BooleanOp::Difference => opposed,
            };
            keep.then_some(false)
        }
        _ => None,
    }
}

type Provenance = Vec<(FaceId, MeshSide, FaceId)>;

enum StitchError {
    /// An exact edge-only operand contact, classified from source topology.
    NonManifoldContact,
    /// Any rebuild failure not proven to be an operand-contact limitation.
    Build(BuildError),
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal stage threading fixed pipeline context"
)]
fn stitch(
    op: BooleanOp,
    source_mesh_a: &Mesh,
    source_mesh_b: &Mesh,
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    graph: &IntersectionGraph,
    outcome_a: &MeshSplitOutcome,
    outcome_b: &MeshSplitOutcome,
    contacts: &[CoplanarContact],
    classification: &PatchClassification,
    stats: &mut BooleanStats,
) -> Result<(Mesh, Provenance), StitchError> {
    let mut builder = MeshBuilder::new();

    // --- Identity-based vertex maps, seam vertices pre-welded. One connected
    // graph component can contain distinct f64 constructions that narrow to
    // one stored f32 point; those representational aliases share one builder
    // identity. Equal positions in unrelated graph components remain distinct.
    let mut map_a: HashMap<VertexId, u32> = HashMap::new();
    let mut map_b: HashMap<VertexId, u32> = HashMap::new();
    let mut seam_builder_indices: Vec<Option<u32>> = alloc::vec![None; graph.vertices.len()];
    let representatives = canonical_seam_representatives(graph);
    let mut representative_builder_indices: Vec<Option<u32>> =
        alloc::vec![None; graph.vertices.len()];
    let mut representative_positions: Vec<Option<[f32; 3]>> =
        alloc::vec![None; graph.vertices.len()];
    let mut representative_materializations = alloc::vec![0_u32; graph.vertices.len()];
    let mut representative_has_a = alloc::vec![false; graph.vertices.len()];
    let mut representative_has_b = alloc::vec![false; graph.vertices.len()];
    for (index, &representative) in representatives.iter().enumerate() {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "graph vertex indices originate as u32"
        )]
        let graph_index = index as u32;
        let isolated_touch = graph.touch_points.binary_search(&graph_index).is_ok()
            && !graph
                .edges
                .iter()
                .any(|edge| edge.vertices.contains(&graph_index));
        if isolated_touch {
            // A point-only contact is not a seam. Welding it would join two
            // closed shells at one topology vertex and manufacture a
            // non-manifold vertex; the per-side maps below deliberately keep
            // one coincident identity in each regularized shell. Touches at
            // the endpoint of a real graph edge still belong to that edge.
            continue;
        }
        let representative = representative as usize;
        let mut materialized = false;
        if let Some(vertex) = outcome_a.graph_vertices[index]
            && let Some(&position) = mesh_a.vertex_position(vertex)
        {
            record_representative_position(&mut representative_positions[representative], position);
            representative_has_a[representative] = true;
            materialized = true;
        }
        if let Some(vertex) = outcome_b.graph_vertices[index]
            && let Some(&position) = mesh_b.vertex_position(vertex)
        {
            record_representative_position(&mut representative_positions[representative], position);
            representative_has_b[representative] = true;
            materialized = true;
        }
        if materialized {
            representative_materializations[representative] =
                representative_materializations[representative].saturating_add(1);
        }
    }
    for representative in 0..representative_positions.len() {
        let shared_across_operands =
            representative_has_a[representative] && representative_has_b[representative];
        let has_aliases = representative_materializations[representative] > 1;
        if (shared_across_operands || has_aliases)
            && let Some(position) = representative_positions[representative]
        {
            representative_builder_indices[representative] = Some(builder.push_vertex(position));
            if shared_across_operands {
                stats.welded_vertices += 1;
            }
        }
    }
    // Every materialized vertex in an alias group maps to its survivor. A and
    // B need not materialize the same graph record: shared stored identity is
    // a property of the entire topology-scoped group.
    for (index, slot) in seam_builder_indices.iter_mut().enumerate() {
        let representative = representatives[index] as usize;
        let Some(builder_index) = representative_builder_indices[representative] else {
            continue;
        };
        let expected_position = representative_positions[representative];
        if let Some(vertex) = outcome_a.graph_vertices[index] {
            debug_assert_eq!(
                mesh_a.vertex_position(vertex).copied(),
                expected_position,
                "collapsed seam component stores one narrowed position"
            );
            map_a.insert(vertex, builder_index);
        }
        if let Some(vertex) = outcome_b.graph_vertices[index] {
            debug_assert_eq!(
                mesh_b.vertex_position(vertex).copied(),
                expected_position,
                "collapsed seam component stores one narrowed position"
            );
            map_b.insert(vertex, builder_index);
        }
        *slot = Some(builder_index);
    }

    // --- Kept faces in deterministic order (classification is already
    // mesh A first, patches in lowest-face order, faces ascending).
    // Captured boundary attributes re-apply after the build.
    let mut sources: Vec<(MeshSide, FaceId)> = Vec::new();
    let mut captured_attrs: Vec<(u32, u32, Option<f32>, Option<bool>)> = Vec::new();
    for patch in &classification.patches {
        let Some(flip) = selection(op, patch.mesh, patch.side) else {
            continue;
        };
        let (mesh, map) = match patch.mesh {
            MeshSide::A => (mesh_a, &mut map_a),
            MeshSide::B => (mesh_b, &mut map_b),
        };
        for &face in &patch.faces {
            let captured_attrs_start = captured_attrs.len();
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
            // Canonical seam identities can turn a formerly simple source
            // loop into a closed walk which visits the survivor more than
            // once. Split that walk at repeated survivors: each emitted loop
            // remains a simple face, while two-edge and point loops have no
            // representable area and disappear. No new diagonal is invented;
            // the cycles partition the source boundary edges.
            let mut face_loops = simple_cycles(loop_indices);
            if face_loops.is_empty() {
                // The source face narrowed entirely onto a seam point or
                // segment. Its captured attributes have no surviving edge.
                captured_attrs.truncate(captured_attrs_start);
                continue;
            }
            let region = mesh
                .attrs()
                .dense(attr::FACE_REGION)
                .and_then(|layer| layer.get(face.as_id()).copied());
            for loop_indices in &mut face_loops {
                if flip {
                    loop_indices.reverse();
                }
                builder
                    .add_face_with_attrs(
                        loop_indices,
                        &FaceBuildAttrs {
                            region,
                            ..FaceBuildAttrs::default()
                        },
                    )
                    .map_err(StitchError::Build)?;
                // A pinched source face may become several simple output
                // faces. Repeating its source here preserves one provenance
                // row per emitted face and deterministic encounter order.
                sources.push((patch.mesh, face));
                stats.kept_faces += 1;
            }
        }
    }

    let result = builder.build().map_err(|error| {
        classify_stitch_build_error(
            error,
            op,
            source_mesh_a,
            source_mesh_b,
            mesh_a,
            mesh_b,
            graph,
            contacts,
            classification,
            &map_a,
            &map_b,
            &seam_builder_indices,
        )
    })?;
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

fn record_representative_position(slot: &mut Option<[f32; 3]>, position: [f32; 3]) {
    if let Some(existing) = slot {
        debug_assert_eq!(
            stored_position_key(*existing),
            stored_position_key(position),
            "one seam-identity group must store one canonical position key"
        );
    } else {
        *slot = Some(position);
    }
}

/// Lowest-index representative for every exact stored position within one
/// connected intersection-graph component.
///
/// Graph provenance intentionally keeps distinct `f64` constructions when
/// their carriers differ. Stitching cannot preserve that distinction within
/// one seam after both points narrow to the same mesh position: adjacent
/// aliases create zero-length edges, while non-adjacent aliases pinch the seam
/// ring. Scoping the coordinate key by graph connectivity retains the
/// provenance fence against unrelated coincident seams and shells.
fn canonical_seam_representatives(graph: &IntersectionGraph) -> Vec<u32> {
    let mut components: Vec<u32> = (0..graph.vertices.len())
        .map(|index| u32::try_from(index).expect("intersection graph vertex count fits u32"))
        .collect();
    for edge in &graph.edges {
        let [a, b] = edge.vertices;
        let root_a = representative(&components, a);
        let root_b = representative(&components, b);
        if root_a != root_b {
            let (survivor, alias) = (root_a.min(root_b), root_a.max(root_b));
            components[alias as usize] = survivor;
        }
    }
    for index in 0..components.len() {
        components[index] = representative(
            &components,
            u32::try_from(index).expect("intersection graph vertex count fits u32"),
        );
    }

    let mut parents: Vec<u32> = (0..graph.vertices.len())
        .map(|index| u32::try_from(index).expect("intersection graph vertex count fits u32"))
        .collect();
    let mut survivors = HashMap::<(u32, [u32; 3]), u32>::new();
    for (index, vertex) in graph.vertices.iter().enumerate() {
        let index = u32::try_from(index).expect("intersection graph vertex count fits u32");
        let key = (
            components[index as usize],
            stored_position_key(narrow(vertex.position)),
        );
        if let Some(&survivor) = survivors.get(&key) {
            parents[index as usize] = survivor;
        } else {
            survivors.insert(key, index);
        }
    }
    parents
}

/// Hashable numerical identity for a stored mesh position.
///
/// Signed zero has two IEEE bit patterns but only one geometric value. Other
/// coordinates retain their exact stored bits; tolerance welding would cross
/// the topology/provenance fence established by the intersection graph.
fn stored_position_key(position: [f32; 3]) -> [u32; 3] {
    position.map(|coordinate| {
        if coordinate == 0.0 {
            0.0_f32.to_bits()
        } else {
            coordinate.to_bits()
        }
    })
}

fn representative(parents: &[u32], mut index: u32) -> u32 {
    while parents[index as usize] != index {
        index = parents[index as usize];
    }
    index
}

/// Decomposes a closed vertex walk into simple cycles after aliases merge.
///
/// Each repeated vertex closes the suffix since its previous visit. Keeping
/// that shared vertex on the remaining prefix preserves all non-degenerate
/// source edges without manufacturing a diagonal. Cycles shorter than three
/// vertices are exact point/segment collapses and cannot be mesh faces.
fn simple_cycles(mut walk: Vec<u32>) -> Vec<Vec<u32>> {
    walk.dedup();
    if walk.len() > 1 && walk.first() == walk.last() {
        walk.pop();
    }
    if walk.len() < 3 {
        return Vec::new();
    }

    let first = walk[0];
    let mut path = alloc::vec![first];
    let mut offsets = HashMap::new();
    offsets.insert(first, 0_usize);
    let mut cycles = Vec::new();
    for vertex in walk.into_iter().skip(1).chain(core::iter::once(first)) {
        if let Some(&start) = offsets.get(&vertex) {
            let cycle = path[start..].to_vec();
            for removed in path.drain(start + 1..) {
                offsets.remove(&removed);
            }
            if cycle.len() >= 3 {
                cycles.push(cycle);
            }
        } else {
            offsets.insert(vertex, path.len());
            path.push(vertex);
        }
    }
    debug_assert_eq!(
        path,
        [first],
        "closing the walk must consume every extracted cycle"
    );
    cycles
}

/// Separates a geometric edge-contact refusal from a genuine stitch bug.
///
/// `MeshBuilder` only knows that four selected faces reached one output edge;
/// it cannot know why. Calling every such error an operand limitation would
/// hide classification and stitching defects. We therefore recognize the
/// narrow edge-contact case only when all of the following source facts agree:
///
/// - the failing builder edge is one intersection-graph edge;
/// - that graph edge is a real pre-existing mesh edge on both operands;
/// - exactly two selected faces from each operand use it; and
/// - it does not bound a positive-area coplanar contact already handled by
///   the contact-selection table.
///
/// Anything else remains [`BooleanError::Build`] via [`StitchError::Build`].
#[expect(
    clippy::too_many_arguments,
    reason = "classification needs the stitch artifacts that prove source ownership"
)]
fn classify_stitch_build_error(
    error: BuildError,
    op: BooleanOp,
    source_mesh_a: &Mesh,
    source_mesh_b: &Mesh,
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    graph: &IntersectionGraph,
    contacts: &[CoplanarContact],
    classification: &PatchClassification,
    map_a: &HashMap<VertexId, u32>,
    map_b: &HashMap<VertexId, u32>,
    seam_builder_indices: &[Option<u32>],
) -> StitchError {
    let BuildError::NonManifoldEdge { a, b, count } = error else {
        return StitchError::Build(error);
    };
    if op != BooleanOp::Union {
        return StitchError::Build(error);
    }
    let output_edge = [a.min(b), a.max(b)];
    let Some(graph_edge) = graph.edges.iter().find(|edge| {
        let [p, q] = edge.vertices.map(|index| index as usize);
        seam_builder_indices
            .get(p)
            .copied()
            .flatten()
            .zip(seam_builder_indices.get(q).copied().flatten())
            .is_some_and(|(u, v)| [u.min(v), u.max(v)] == output_edge)
    }) else {
        return StitchError::Build(error);
    };
    let [p, q] = graph_edge.vertices.map(|index| index as usize);
    let Some((start, end)) = graph.vertices.get(p).zip(graph.vertices.get(q)) else {
        return StitchError::Build(error);
    };
    if !anchors_are_one_mesh_edge(source_mesh_a, start.anchor_a, end.anchor_a)
        || !anchors_are_one_mesh_edge(source_mesh_b, start.anchor_b, end.anchor_b)
        || contact_bounds_graph_edge(contacts, start, end)
    {
        return StitchError::Build(error);
    }

    let (faces_a, faces_b) = selected_edge_face_counts(
        output_edge,
        mesh_a,
        mesh_b,
        op,
        classification,
        map_a,
        map_b,
    );
    if count == 4 && faces_a == 2 && faces_b == 2 {
        StitchError::NonManifoldContact
    } else {
        StitchError::Build(error)
    }
}

/// Returns whether two graph anchors identify the same real mesh edge.
/// `EdgeSpan` can also denote a triangulation diagonal, hence the final
/// topology lookup rather than trusting provenance shape alone.
fn anchors_are_one_mesh_edge(mesh: &Mesh, start: MeshAnchor, end: MeshAnchor) -> bool {
    let carrier = match (start, end) {
        (MeshAnchor::Vertex(a), MeshAnchor::Vertex(b)) if a != b => Some((a, b)),
        (MeshAnchor::Vertex(vertex), MeshAnchor::EdgeSpan(a, b))
        | (MeshAnchor::EdgeSpan(a, b), MeshAnchor::Vertex(vertex))
            if vertex == a || vertex == b =>
        {
            Some((a, b))
        }
        (MeshAnchor::EdgeSpan(a, b), MeshAnchor::EdgeSpan(c, d))
            if sorted_vertex_pair(a, b) == sorted_vertex_pair(c, d) =>
        {
            Some((a, b))
        }
        _ => None,
    };
    carrier.is_some_and(|(a, b)| find_half_edge(mesh, a, b).is_some())
}

fn sorted_vertex_pair(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a.index() <= b.index() {
        (a, b)
    } else {
        (b, a)
    }
}

/// A positive-area coplanar region adjacent to this graph edge means contact
/// selection, rather than edge-only regularization, owns the configuration.
fn contact_bounds_graph_edge(
    contacts: &[CoplanarContact],
    start: &super::graph::GraphVertex,
    end: &super::graph::GraphVertex,
) -> bool {
    contacts.iter().any(|contact| {
        start.faces_a.contains(&contact.face_a)
            && end.faces_a.contains(&contact.face_a)
            && start.faces_b.contains(&contact.face_b)
            && end.faces_b.contains(&contact.face_b)
    })
}

/// Counts selected face loops that use one builder edge, separated by source
/// operand. This recomputes only on the exceptional rebuild path, keeping the
/// ordinary stitch allocation-calm.
fn selected_edge_face_counts(
    output_edge: [u32; 2],
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    op: BooleanOp,
    classification: &PatchClassification,
    map_a: &HashMap<VertexId, u32>,
    map_b: &HashMap<VertexId, u32>,
) -> (usize, usize) {
    let mut counts = [0_usize; 2];
    for patch in &classification.patches {
        if selection(op, patch.mesh, patch.side).is_none() {
            continue;
        }
        let (mesh, map, side) = match patch.mesh {
            MeshSide::A => (mesh_a, map_a, 0),
            MeshSide::B => (mesh_b, map_b, 1),
        };
        for &face in &patch.faces {
            let indices: Vec<u32> = mesh
                .face_loop(face)
                .filter_map(|half_edge| mesh.to_vertex(half_edge))
                .filter_map(|vertex| map.get(&vertex).copied())
                .collect();
            if indices.len() < 2 {
                continue;
            }
            let uses_edge = (0..indices.len()).any(|index| {
                let a = indices[index];
                let b = indices[(index + 1) % indices.len()];
                [a.min(b), a.max(b)] == output_edge
            });
            if uses_edge {
                counts[side] += 1;
            }
        }
    }
    (counts[0], counts[1])
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
    use super::super::graph::{GraphEdge, GraphVertex};
    use super::*;
    use crate::op::set_face_region;
    use alloc::vec;

    fn graph_vertex(position: [f64; 3]) -> GraphVertex {
        GraphVertex {
            position,
            anchor_a: MeshAnchor::FaceInterior(FaceId::OUTSIDE),
            anchor_b: MeshAnchor::FaceInterior(FaceId::OUTSIDE),
            faces_a: Vec::new(),
            faces_b: Vec::new(),
        }
    }

    #[test]
    fn seam_identity_merges_non_adjacent_vertices_in_one_component() {
        // Two graph constructions can reach the same stored point through an
        // intervening edge. This also pins f64 narrowing and signed-zero
        // canonicalization: they are one identity because the connected seam
        // would otherwise revisit an indistinguishable stored vertex.
        let graph = IntersectionGraph {
            vertices: vec![
                graph_vertex([1.0, 0.0, 0.0]),
                graph_vertex([2.0, 0.0, 0.0]),
                graph_vertex([1.0 + f64::EPSILON, -0.0, 0.0]),
            ],
            edges: vec![
                GraphEdge {
                    vertices: [0, 1],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [1, 2],
                    crossings: Vec::new(),
                },
            ],
            ..IntersectionGraph::default()
        };

        assert_eq!(canonical_seam_representatives(&graph), vec![0, 1, 0]);
    }

    #[test]
    fn seam_identity_keeps_disconnected_coincident_components_distinct() {
        // Equal coordinates do not imply shared topology. Separate seam
        // components (and, by extension, separate shells) retain independent
        // identities even when their f32 positions match exactly.
        let graph = IntersectionGraph {
            vertices: vec![
                graph_vertex([0.0, 0.0, 0.0]),
                graph_vertex([1.0, 0.0, 0.0]),
                graph_vertex([0.0, 0.0, 0.0]),
                graph_vertex([-1.0, 0.0, 0.0]),
            ],
            edges: vec![
                GraphEdge {
                    vertices: [0, 1],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [2, 3],
                    crossings: Vec::new(),
                },
            ],
            ..IntersectionGraph::default()
        };

        assert_eq!(canonical_seam_representatives(&graph), vec![0, 1, 2, 3]);
    }

    #[test]
    fn alias_pinches_split_into_simple_face_cycles() {
        // Canonicalizing vertex 1 turns one source boundary into two lobes
        // which share only that survivor. Each lobe must become a simple
        // face; emitting the original walk would violate mesh invariants.
        assert_eq!(
            simple_cycles(vec![0, 1, 2, 3, 1, 4, 5]),
            vec![vec![1, 2, 3], vec![0, 1, 4, 5]]
        );
    }

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

    fn zero_area_faces(mesh: &Mesh) -> Vec<(FaceId, Vec<[f32; 3]>)> {
        mesh.faces()
            .filter_map(|face| {
                let points: Vec<[f32; 3]> = mesh
                    .face_loop(face)
                    .filter_map(|half_edge| mesh.to_vertex(half_edge))
                    .filter_map(|vertex| mesh.vertex_position(vertex).copied())
                    .collect();
                let has_area = (1..points.len().saturating_sub(1)).any(|index| {
                    let a = points[0];
                    let b = points[index];
                    let c = points[index + 1];
                    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let cross = [
                        ab[1] * ac[2] - ab[2] * ac[1],
                        ab[2] * ac[0] - ab[0] * ac[2],
                        ab[0] * ac[1] - ab[1] * ac[0],
                    ];
                    cross != [0.0; 3]
                });
                (!has_area).then_some((face, points))
            })
            .collect()
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

    /// Builds an axis-aligned box mesh spanning `min..max` (same face
    /// order as `cube`: -z, +z, -y, +x, +y, -x).
    fn box_mesh(min: [f32; 3], max: [f32; 3]) -> Mesh {
        let positions = [
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
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
            builder.add_face(&face).expect("valid box face");
        }
        builder.build().expect("valid box").mesh
    }

    /// Builds the same geometry with the wall-first face order emitted by
    /// `exedra_constructive`'s rectangle extrusion. Face order changes stable
    /// face IDs and therefore deterministic graph labels, so it is part of
    /// this fixture even though the solid is geometrically identical.
    fn extruded_box_mesh(min: [f32; 3], max: [f32; 3]) -> Mesh {
        let positions = [
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
        ];
        let faces: [[u32; 4]; 6] = [
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
            [3, 2, 1, 0],
            [4, 5, 6, 7],
        ];
        let mut builder = MeshBuilder::new();
        for position in positions {
            builder.push_vertex(position);
        }
        for face in faces {
            builder.add_face(&face).expect("valid box face");
        }
        builder.build().expect("valid box").mesh
    }

    fn run_boolean(
        mesh_a: &Mesh,
        mesh_b: &Mesh,
        op: BooleanOp,
    ) -> Result<(BooleanOutput, BooleanDiagnostics), (BooleanError, BooleanDiagnostics)> {
        let mut scratch = BooleanScratch::new();
        let mut diagnostics = BooleanDiagnostics::default();
        match boolean_mesh(
            mesh_a,
            mesh_b,
            op,
            FaceTriangulation::Fan,
            &mut scratch,
            &mut diagnostics,
        ) {
            Ok(output) => Ok((output, diagnostics)),
            Err(error) => Err((error, diagnostics)),
        }
    }

    #[test]
    fn union_of_touching_boxes_removes_the_shared_wall() {
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let (output, diagnostics) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Union).expect("touching union succeeds");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        let volume = signed_volume(&output.mesh);
        assert!((volume - 2.0).abs() < 1e-6, "union volume {volume}");
        assert_eq!(
            output.mesh.faces().count(),
            10,
            "five faces per box; both wall copies annihilate"
        );
        // The shared wall (A face 3 = +x, B face 5 = -x) is gone.
        assert!(!output.face_provenance.iter().any(|&(_, side, original)| {
            (side == MeshSide::A && original.index() == 3)
                || (side == MeshSide::B && original.index() == 5)
        }));
        // Both contact patches classified as opposed boundary regions.
        for side in [MeshSide::A, MeshSide::B] {
            assert!(
                output
                    .classification
                    .of(side)
                    .any(|p| p.side == PatchSide::Boundary { opposed: true }),
                "{side:?} carries an opposed contact patch"
            );
        }
        // The contact outline is a tagged seam of the merged solid.
        assert!(output.stats.seam_edges >= 4, "{:?}", output.stats);
    }

    #[test]
    fn shared_edge_union_returns_a_typed_geometric_refusal() {
        // Two otherwise-disjoint boxes share exactly one vertical edge. The
        // volumetric union is not a 2-manifold there, so the public error must
        // describe operand contact rather than expose a builder invariant.
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 1.0, 0.0], [2.0, 2.0, 1.0]);
        let (error, diagnostics) = run_boolean(&mesh_a, &mesh_b, BooleanOp::Union)
            .expect_err("shared-edge union is not a manifold solid");

        assert_eq!(error, BooleanError::NonManifoldContact);
        assert_eq!(
            diagnostics.count_of(BooleanFailureKind::NonManifoldContact),
            1,
            "typed refusal retains an inspectable geometric diagnostic"
        );
    }

    #[test]
    fn unproven_non_manifold_rebuild_error_remains_internal() {
        // The contact classifier must not launder an arbitrary builder error
        // into a supported geometric refusal. Without graph/source evidence,
        // the same low-level shape remains an internal stitch failure.
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([0.5, 0.5, 0.5], [1.5, 1.5, 1.5]);
        let build = BuildError::NonManifoldEdge {
            a: 0,
            b: 1,
            count: 4,
        };
        let classified = classify_stitch_build_error(
            build,
            BooleanOp::Union,
            &mesh_a,
            &mesh_b,
            &mesh_a,
            &mesh_b,
            &IntersectionGraph::default(),
            &[],
            &PatchClassification::default(),
            &HashMap::new(),
            &HashMap::new(),
            &[],
        );

        assert!(matches!(classified, StitchError::Build(error) if error == build));
    }

    #[test]
    fn shared_edge_non_union_ops_keep_their_regularized_volumes() {
        // Edge-only contact has no common volume: Intersection is empty and
        // Difference leaves A intact. Only Union needs the non-manifold
        // refusal because only it would retain all four incident faces.
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 1.0, 0.0], [2.0, 2.0, 1.0]);

        let (intersection, diagnostics) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Intersection).expect("empty intersection");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert_eq!(intersection.mesh.faces().count(), 0);

        let (difference, diagnostics) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Difference).expect("unchanged difference");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(difference.mesh.validate_deep().is_empty());
        assert!((signed_volume(&difference.mesh) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn shared_vertex_union_preserves_two_regularized_shells() {
        // A point contact has zero volume and needs no welded topology. Keeping
        // shell-local vertex identities yields two valid closed components
        // instead of creating a non-manifold vertex in one shell complex.
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
        let (output, diagnostics) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Union).expect("point contact regularizes");

        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(output.mesh.validate_deep().is_empty());
        assert_eq!(output.mesh.faces().count(), 12, "both cube shells survive");
        assert_eq!(
            output
                .mesh
                .vertices()
                .filter(|&vertex| output.mesh.vertex_position(vertex) == Some(&[1.0, 1.0, 1.0]))
                .count(),
            2,
            "the coincident point retains one identity per closed shell"
        );
    }

    #[test]
    fn union_with_partial_face_overlap_is_watertight() {
        // B's smaller wall sits flush on a quadrant of A's wall, sharing
        // part of the wall's boundary (within the v1 splitting envelope).
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 0.0, 0.0], [2.0, 0.5, 0.5]);
        let (output, diagnostics) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Union).expect("partial-overlap union");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        let volume = signed_volume(&output.mesh);
        assert!((volume - 1.25).abs() < 1e-6, "union volume {volume}");
    }

    #[test]
    fn union_with_centered_partial_face_overlap_is_watertight() {
        // A's bottom face is centered within B's top face along X while
        // sharing its full Y span. The two interior contact-boundary edges
        // must separate the shared wall from the exterior surface patch.
        let mesh_a = box_mesh([2.4, -0.1, 1.8], [3.6, 0.7, 3.2]);
        let mesh_b = box_mesh([2.2, -0.1, 1.6], [3.8, 0.7, 1.8]);
        let (output, diagnostics) = run_boolean(&mesh_a, &mesh_b, BooleanOp::Union)
            .expect("centered partial-overlap union");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        let volume = signed_volume(&output.mesh);
        assert!((volume - 1.6).abs() < 1e-5, "union volume {volume}");
    }

    #[test]
    fn difference_output_is_valid_input_to_another_boolean() {
        // Pins the first stage of a chained difference. A drilled cap may
        // retain collinear seam labels, but it must not emit zero-area faces
        // that the next narrow phase can only reject as degenerate input.
        let host = extruded_box_mesh([0.0, 0.0, 0.0], [6.0, 0.6, 4.0]);
        let cutter = extruded_box_mesh([2.4, -0.1, 1.8], [3.6, 0.7, 3.2]);
        let (output, diagnostics) =
            run_boolean(&host, &cutter, BooleanOp::Difference).expect("first difference");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let zero_area = zero_area_faces(&output.mesh);
        assert!(
            zero_area.is_empty(),
            "zero-area output faces: {zero_area:?}"
        );

        // The follow-up cutter meets the first cut exactly at z=1.8. This
        // exercises the downstream operation as well as inspecting its input.
        let flush_cutter = extruded_box_mesh([2.2, -0.1, 1.6], [3.8, 0.7, 1.8]);
        let (chained, diagnostics) =
            run_boolean(&output.mesh, &flush_cutter, BooleanOp::Difference).unwrap_or_else(
                |(error, diagnostics)| panic!("{error:?}: {:?}", diagnostics.entries()),
            );
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(chained.mesh.validate_deep().is_empty());
    }

    #[test]
    fn chained_flush_cutters_are_order_independent() {
        // The shallow cutter creates a cap that the taller cutter opens on
        // the next step. Reversing the original regression order must retain
        // the same connected void instead of leaving a dangling graph chain.
        let host = extruded_box_mesh([0.0, 0.0, 0.0], [6.0, 0.6, 4.0]);
        let shallow = extruded_box_mesh([2.2, -0.1, 1.6], [3.8, 0.7, 1.8]);
        let (first, diagnostics) =
            run_boolean(&host, &shallow, BooleanOp::Difference).expect("first difference");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());

        let tall = extruded_box_mesh([2.4, -0.1, 1.8], [3.6, 0.7, 3.2]);
        let (output, diagnostics) = run_boolean(&first.mesh, &tall, BooleanOp::Difference)
            .unwrap_or_else(|(error, diagnostics)| {
                panic!("{error:?}: {:?}", diagnostics.entries())
            });
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(output.mesh.validate_deep().is_empty());
        let volume = signed_volume(&output.mesh);
        assert!((volume - 13.2).abs() < 1e-5, "difference volume {volume}");
    }

    #[test]
    fn union_of_interpenetrating_aligned_boxes_is_watertight() {
        // Pins the n-ary cutter fold: the short box overlaps the bottom of
        // the tall box, so their coplanar side contacts meet transversal cut
        // chains without corrupting the OUTSIDE half-edge cycle.
        let tall = extruded_box_mesh([2.4, -0.1, 1.8], [3.6, 0.7, 3.2]);
        let short = extruded_box_mesh([2.2, -0.1, 1.6], [3.8, 0.7, 2.0]);
        let (output, diagnostics) =
            run_boolean(&tall, &short, BooleanOp::Union).expect("overlapping cutter union");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        let volume = signed_volume(&output.mesh);
        assert!((volume - 1.664).abs() < 1e-5, "union volume {volume}");
    }

    #[test]
    fn difference_drills_four_disconnected_box_cutters() {
        // Four disjoint cutter components create four cap holes in one pass;
        // every hole boundary must survive triangulation and stitch into one
        // watertight host body with the expected removed volume.
        let host = box_mesh([0.0, 0.0, 0.0], [6.0, 0.6, 4.0]);
        let mut cutter = box_mesh([0.3, -0.1, 1.5], [0.9, 0.7, 2.5]);
        for (min_x, max_x) in [(1.5, 2.1), (2.7, 3.3), (3.9, 4.5)] {
            let next = box_mesh([min_x, -0.1, 1.5], [max_x, 0.7, 2.5]);
            cutter = run_boolean(&cutter, &next, BooleanOp::Union)
                .expect("disjoint cutter union")
                .0
                .mesh;
        }

        let (output, diagnostics) =
            run_boolean(&host, &cutter, BooleanOp::Difference).expect("four-cutter difference");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(output.mesh.validate_deep().is_empty());
        let volume = signed_volume(&output.mesh);
        assert!((volume - 12.96).abs() < 1e-5, "difference volume {volume}");
    }

    #[test]
    fn difference_carves_a_face_flush_notch() {
        // The cutter shares three wall planes with the host: a corner
        // notch whose walls open exactly where the flush contact was.
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([0.5, 0.0, 0.0], [1.0, 0.5, 0.5]);
        let (output, diagnostics) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Difference).expect("flush notch difference");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        let volume = signed_volume(&output.mesh);
        assert!((volume - 0.875).abs() < 1e-6, "difference volume {volume}");
        // Same-normal contacts drop for Difference: all three flush wall
        // regions open into the notch.
        assert!(
            output
                .classification
                .of(MeshSide::A)
                .filter(|p| p.side == PatchSide::Boundary { opposed: false })
                .count()
                >= 3
        );
    }

    #[test]
    fn difference_of_externally_touching_boxes_keeps_the_wall() {
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let (output, diagnostics) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Difference).expect("touching difference");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // A is untouched: the opposed contact region stays A's boundary.
        let volume = signed_volume(&output.mesh);
        assert!((volume - 1.0).abs() < 1e-6, "difference volume {volume}");
        assert_eq!(output.mesh.faces().count(), 6);
    }

    #[test]
    fn intersection_of_touching_boxes_is_empty() {
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let (output, _) =
            run_boolean(&mesh_a, &mesh_b, BooleanOp::Intersection).expect("touching intersection");
        assert_eq!(
            output.mesh.faces().count(),
            0,
            "a shared wall bounds no common volume"
        );
    }

    #[test]
    fn touching_booleans_are_deterministic() {
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        for other in [
            box_mesh([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]),
            box_mesh([1.0, 0.0, 0.0], [2.0, 0.5, 0.5]),
        ] {
            let (first, _) = run_boolean(&mesh_a, &other, BooleanOp::Union).expect("union");
            let (second, _) = run_boolean(&mesh_a, &other, BooleanOp::Union).expect("union");
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
    }

    #[test]
    fn interior_flush_contact_is_typed_or_correct_never_silent() {
        // The contact region lies strictly inside A's wall, so carving it
        // needs interior-loop splitting. Until that lands the pipeline
        // must refuse typed (suspect patches); once it lands the result
        // must be the correct watertight union. Silent wrong geometry is
        // the only unacceptable outcome.
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 0.25, 0.25], [2.0, 0.75, 0.75]);
        match run_boolean(&mesh_a, &mesh_b, BooleanOp::Union) {
            Ok((output, _)) => {
                let errors = output.mesh.validate_deep();
                assert!(errors.is_empty(), "{errors:?}");
                let volume = signed_volume(&output.mesh);
                assert!((volume - 1.25).abs() < 1e-6, "union volume {volume}");
            }
            Err((error, diagnostics)) => {
                assert!(
                    matches!(error, BooleanError::SuspectPatches { .. }),
                    "typed refusal expected, got {error:?}"
                );
                assert!(!diagnostics.is_clean(), "refusal carries diagnostics");
            }
        }
    }

    #[test]
    fn deferred_split_with_clean_classification_still_withholds_geometry() {
        // A graph vertex may materialize on another face and hide the original
        // incomplete cut from classification. The split-stage deferral remains
        // independently sufficient to refuse the result.
        let mut outcome_a = MeshSplitOutcome::default();
        outcome_a.stats.deferred_faces = 1;
        let outcome_b = MeshSplitOutcome::default();
        let classification = PatchClassification::default();

        assert_eq!(
            unresolved_region_count(&outcome_a, &outcome_b, &classification),
            1
        );
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

    /// A 4 x 4 x 1 box centered at the origin: the drill-through target.
    fn slab() -> Mesh {
        let positions = [
            [-2.0, -2.0, -0.5],
            [2.0, -2.0, -0.5],
            [2.0, 2.0, -0.5],
            [-2.0, 2.0, -0.5],
            [-2.0, -2.0, 0.5],
            [2.0, -2.0, 0.5],
            [2.0, 2.0, 0.5],
            [-2.0, 2.0, 0.5],
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
            builder.add_face(&face).expect("valid slab face");
        }
        builder.build().expect("valid slab").mesh
    }

    /// A regular 16-gon prism, radius 0.8, z in [-1.5, 1.5]: pierces the
    /// slab completely so each slab cap sees a closed interior loop.
    fn drill_prism() -> Mesh {
        let n = 16_u32;
        let mut builder = MeshBuilder::new();
        for z in [-1.5_f64, 1.5] {
            for i in 0..n {
                let angle = core::f64::consts::TAU * f64::from(i) / f64::from(n);
                let position = [0.8 * angle.cos(), 0.8 * angle.sin(), z];
                #[expect(clippy::cast_possible_truncation, reason = "test geometry narrowing")]
                builder.push_vertex([position[0] as f32, position[1] as f32, position[2] as f32]);
            }
        }
        let bottom: Vec<u32> = (0..n).rev().collect();
        builder.add_face(&bottom).expect("bottom cap");
        let top: Vec<u32> = (n..2 * n).collect();
        builder.add_face(&top).expect("top cap");
        for i in 0..n {
            let j = (i + 1) % n;
            builder.add_face(&[i, j, n + j, n + i]).expect("side wall");
        }
        builder.build().expect("valid prism").mesh
    }

    /// Exact cross-section area of the drill prism (regular n-gon).
    fn drill_area() -> f64 {
        0.5 * 16.0 * 0.8 * 0.8 * (core::f64::consts::TAU / 16.0).sin()
    }

    fn run_drill(op: BooleanOp) -> (BooleanOutput, BooleanDiagnostics) {
        let mut scratch = BooleanScratch::new();
        let mut diagnostics = BooleanDiagnostics::default();
        let output = boolean_mesh(
            &slab(),
            &drill_prism(),
            op,
            FaceTriangulation::Fan,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("drill boolean succeeds");
        (output, diagnostics)
    }

    /// Euler characteristic over the closed surface (V - E + F): 2 for a
    /// sphere-like shell, 0 for a single through-hole shell.
    fn euler_characteristic(mesh: &Mesh) -> i64 {
        let vertices = i64::try_from(mesh.vertices().count()).expect("small");
        let faces = i64::try_from(mesh.faces().count()).expect("small");
        let half_edges: usize = mesh.faces().map(|face| mesh.face_loop(face).count()).sum();
        let edges = i64::try_from(half_edges).expect("small") / 2;
        vertices - edges + faces
    }

    #[test]
    fn drill_difference_is_a_holed_watertight_solid() {
        let (output, diagnostics) = run_drill(BooleanOp::Difference);
        // The former interior-loop deferral is gone for this shape.
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = output.mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        // V = slab - hole cross-section * thickness.
        let volume = signed_volume(&output.mesh);
        let expected = 16.0 - drill_area();
        assert!(
            (volume - expected).abs() < 1e-3,
            "difference volume {volume}, expected {expected}"
        );
        // Genus 1: a genuine hole through the shell, not a dent.
        assert_eq!(euler_characteristic(&output.mesh), 0, "through-hole shell");
        assert!(output.stats.seam_edges > 0, "hole rims tagged as seams");
    }

    #[test]
    fn drill_union_and_intersection_volumes_match_closed_form() {
        let (union, diagnostics) = run_drill(BooleanOp::Union);
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = union.mesh.validate_deep();
        assert!(errors.is_empty(), "union: {errors:?}");
        let volume = signed_volume(&union.mesh);
        let expected = 16.0 + 2.0 * drill_area();
        assert!(
            (volume - expected).abs() < 1e-3,
            "union volume {volume}, expected {expected}"
        );

        let (intersection, diagnostics) = run_drill(BooleanOp::Intersection);
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        let errors = intersection.mesh.validate_deep();
        assert!(errors.is_empty(), "intersection: {errors:?}");
        let volume = signed_volume(&intersection.mesh);
        let expected = drill_area();
        assert!(
            (volume - expected).abs() < 1e-3,
            "intersection volume {volume}, expected {expected}"
        );
        // The intersection is the prism segment inside the slab: a plain
        // sphere-like shell.
        assert_eq!(euler_characteristic(&intersection.mesh), 2);
    }

    #[test]
    fn drill_provenance_covers_every_output_face() {
        let (output, _) = run_drill(BooleanOp::Difference);
        assert_eq!(
            output.face_provenance.len(),
            output.mesh.faces().count(),
            "every output face is attributed"
        );
        // Originals are pre-split faces of the operands: slab has 6,
        // prism has 18.
        for &(_, side, original) in &output.face_provenance {
            let limit = match side {
                MeshSide::A => 6,
                MeshSide::B => 18,
            };
            assert!(
                original.index() < limit,
                "original {original:?} is pre-split"
            );
        }
    }

    #[test]
    fn drill_boolean_is_deterministic() {
        let (first, _) = run_drill(BooleanOp::Difference);
        let (second, _) = run_drill(BooleanOp::Difference);
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

    // --- Oracle regression fixtures (exe-dnny): rotated convex operands
    // whose intersection once misclassified a through-hole disk patch.
    // Every disk vertex lies on the cut curve, so the patch sample falls
    // back to face geometry; before the largest-triangle rule it sampled a
    // sliver hugging the cut whose f32-narrowed centroid landed a hair
    // outside the other solid, dropping the whole disk and leaving the
    // result an open tube (validate_deep-clean, since boundaries are
    // legal). Expected volumes are exact convex half-space clipping
    // results computed independently of the pipeline.

    fn oracle_5613_prism() -> Mesh {
        let positions: [[f32; 3]; 48] = [
            [-0.14699002, -0.05251341, 1.0170524],
            [-0.23497675, -0.06370783, 1.0291966],
            [-0.32293925, -0.076021835, 1.0179973],
            [-0.40488303, -0.088616244, 0.9842177],
            [-0.47522378, -0.10063277, 0.9301599],
            [-0.52916783, -0.1112525, 0.8595078],
            [-0.56303906, -0.11975173, 0.7770762],
            [-0.5745292, -0.12555124, 0.6884827],
            [-0.56285506, -0.1282558, 0.5997648],
            [-0.5288124, -0.12768112, 0.51696855],
            [-0.47472104, -0.123866335, 0.44573623],
            [-0.40426737, -0.117071435, 0.3909223],
            [-0.32225257, -0.107759476, 0.35626224],
            [-0.23426585, -0.09656506, 0.34411806],
            [-0.14630328, -0.084251046, 0.35531738],
            [-0.0643596, -0.07165665, 0.3890969],
            [0.005981167, -0.05964012, 0.44315472],
            [0.059925232, -0.049020387, 0.5138068],
            [0.09379644, -0.040521163, 0.5962384],
            [0.105286516, -0.034721654, 0.68483186],
            [0.09361242, -0.032017082, 0.7735499],
            [0.05956972, -0.03259178, 0.8563462],
            [0.005478426, -0.036406558, 0.92757845],
            [-0.06497515, -0.043201447, 0.98239225],
            [-0.2667236, 0.8419054, 0.9740307],
            [-0.3547103, 0.830711, 0.9861749],
            [-0.44267285, 0.818397, 0.9749756],
            [-0.5246166, 0.8058026, 0.941196],
            [-0.59495735, 0.79378605, 0.8871382],
            [-0.6489014, 0.78316635, 0.81648606],
            [-0.68277264, 0.7746671, 0.7340545],
            [-0.69426274, 0.7688676, 0.645461],
            [-0.68258864, 0.76616305, 0.55674314],
            [-0.648546, 0.7667377, 0.47394687],
            [-0.59445465, 0.7705525, 0.40271452],
            [-0.52400094, 0.7773474, 0.34790063],
            [-0.44198614, 0.78665936, 0.31324056],
            [-0.35399944, 0.79785377, 0.30109635],
            [-0.26603687, 0.8101678, 0.31229568],
            [-0.18409318, 0.8227622, 0.34607518],
            [-0.11375241, 0.8347787, 0.40013304],
            [-0.059808344, 0.8453984, 0.47078514],
            [-0.02593714, 0.8538977, 0.5532167],
            [-0.0144470595, 0.85969716, 0.6418102],
            [-0.026121158, 0.8624018, 0.7305282],
            [-0.060163856, 0.8618271, 0.81332445],
            [-0.11425515, 0.85801226, 0.8845567],
            [-0.18470873, 0.8512174, 0.9393705],
        ];
        let faces: &[&[u32]] = &[
            &[0, 24, 25, 1],
            &[1, 25, 26, 2],
            &[2, 26, 27, 3],
            &[3, 27, 28, 4],
            &[4, 28, 29, 5],
            &[5, 29, 30, 6],
            &[6, 30, 31, 7],
            &[7, 31, 32, 8],
            &[8, 32, 33, 9],
            &[9, 33, 34, 10],
            &[10, 34, 35, 11],
            &[11, 35, 36, 12],
            &[12, 36, 37, 13],
            &[13, 37, 38, 14],
            &[14, 38, 39, 15],
            &[15, 39, 40, 16],
            &[16, 40, 41, 17],
            &[17, 41, 42, 18],
            &[18, 42, 43, 19],
            &[19, 43, 44, 20],
            &[20, 44, 45, 21],
            &[21, 45, 46, 22],
            &[22, 46, 47, 23],
            &[23, 47, 24, 0],
            &[
                47, 46, 45, 44, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27,
                26, 25, 24,
            ],
            &[
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23,
            ],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(face).expect("valid fixture face");
        }
        builder.build().expect("valid fixture").mesh
    }

    fn oracle_5613_box() -> Mesh {
        let positions: [[f32; 3]; 8] = [
            [0.37568805, -0.12331576, 0.08715004],
            [-0.1828028, 1.547334, 0.42592442],
            [-0.48202243, 1.2288326, 1.5033162],
            [0.07646841, -0.44181708, 1.1645417],
            [-0.9381503, -0.7079079, 0.80409336],
            [-1.496641, 0.9627418, 1.1428677],
            [-1.1974214, 1.2812431, 0.06547603],
            [-0.6389306, -0.3894066, -0.27329835],
        ];
        let faces: &[&[u32]] = &[
            &[0, 1, 2, 3],
            &[4, 5, 6, 7],
            &[6, 5, 2, 1],
            &[7, 0, 3, 4],
            &[4, 3, 2, 5],
            &[7, 6, 1, 0],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(face).expect("valid fixture face");
        }
        builder.build().expect("valid fixture").mesh
    }

    fn oracle_8653_prism() -> Mesh {
        let positions: [[f32; 3]; 48] = [
            [0.4037345, 0.17647406, -1.463111],
            [0.5850749, 0.09759264, -1.5151093],
            [0.7627896, -0.003476948, -1.5186942],
            [0.9247677, -0.11984698, -1.4736214],
            [1.0599706, -0.24358705, -1.3829626],
            [1.1591845, -0.36626446, -1.252896],
            [1.2156479, -0.4795189, -1.0922855],
            [1.2255133, -0.57563233, -0.9120764],
            [1.1881082, -0.6480547, -0.72454965],
            [1.1059816, -0.69185066, -0.54248494],
            [0.9847303, -0.7040355, -0.3782895],
            [0.83261764, -0.68377894, -0.2431532],
            [0.6600096, -0.6324613, -0.1462853],
            [0.47866926, -0.55357987, -0.0942871],
            [0.30095443, -0.45251018, -0.09070227],
            [0.13897654, -0.33614028, -0.13577501],
            [0.003773608, -0.21240018, -0.22643384],
            [-0.09544028, -0.0897228, -0.35650036],
            [-0.15190376, 0.02353162, -0.5171108],
            [-0.16176912, 0.11964503, -0.69731987],
            [-0.12436386, 0.19206755, -0.8848469],
            [-0.042237256, 0.23586345, -1.0669117],
            [0.079013884, 0.24804829, -1.231107],
            [0.23112631, 0.22779171, -1.366243],
            [0.66723746, 0.6317019, -1.2347432],
            [0.8485779, 0.5528205, -1.2867415],
            [1.0262926, 0.45175087, -1.2903264],
            [1.1882707, 0.33538085, -1.2452536],
            [1.3234736, 0.21164079, -1.1545948],
            [1.4226874, 0.088963374, -1.0245281],
            [1.4791509, -0.024291044, -0.8639177],
            [1.4890163, -0.1204045, -0.68370855],
            [1.4516112, -0.19282691, -0.49618185],
            [1.3694847, -0.23662284, -0.31411713],
            [1.2482333, -0.24880768, -0.14992167],
            [1.0961206, -0.2285511, -0.014785381],
            [0.92351264, -0.17723346, 0.082082525],
            [0.74217224, -0.098352045, 0.13408072],
            [0.5644574, 0.0027176458, 0.13766554],
            [0.40247953, 0.11908755, 0.092592806],
            [0.26727661, 0.24282764, 0.0019339791],
            [0.16806272, 0.36550504, -0.12813254],
            [0.11159923, 0.47875947, -0.28874302],
            [0.10173388, 0.57487285, -0.46895206],
            [0.13913913, 0.64729536, -0.6564791],
            [0.22126575, 0.6910913, -0.83854383],
            [0.34251687, 0.70327616, -1.0027391],
            [0.49462932, 0.6830195, -1.1378752],
        ];
        let faces: &[&[u32]] = &[
            &[0, 24, 25, 1],
            &[1, 25, 26, 2],
            &[2, 26, 27, 3],
            &[3, 27, 28, 4],
            &[4, 28, 29, 5],
            &[5, 29, 30, 6],
            &[6, 30, 31, 7],
            &[7, 31, 32, 8],
            &[8, 32, 33, 9],
            &[9, 33, 34, 10],
            &[10, 34, 35, 11],
            &[11, 35, 36, 12],
            &[12, 36, 37, 13],
            &[13, 37, 38, 14],
            &[14, 38, 39, 15],
            &[15, 39, 40, 16],
            &[16, 40, 41, 17],
            &[17, 41, 42, 18],
            &[18, 42, 43, 19],
            &[19, 43, 44, 20],
            &[20, 44, 45, 21],
            &[21, 45, 46, 22],
            &[22, 46, 47, 23],
            &[23, 47, 24, 0],
            &[
                47, 46, 45, 44, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27,
                26, 25, 24,
            ],
            &[
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23,
            ],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(face).expect("valid fixture face");
        }
        builder.build().expect("valid fixture").mesh
    }

    fn oracle_8653_box() -> Mesh {
        let positions: [[f32; 3]; 8] = [
            [1.0313563, 0.53808326, -1.2482872],
            [0.56720525, 0.18406793, -1.6002858],
            [-0.55818754, 0.8418953, -0.7779206],
            [-0.09403643, 1.1959106, -0.42592204],
            [-0.04156279, 0.510798, 0.19392337],
            [-0.50571394, 0.15678261, -0.1580752],
            [0.61967885, -0.5010447, -0.9804404],
            [1.08383, -0.14702936, -0.6284418],
        ];
        let faces: &[&[u32]] = &[
            &[0, 1, 2, 3],
            &[4, 5, 6, 7],
            &[6, 5, 2, 1],
            &[7, 0, 3, 4],
            &[4, 3, 2, 5],
            &[7, 6, 1, 0],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(face).expect("valid fixture face");
        }
        builder.build().expect("valid fixture").mesh
    }

    fn assert_closed(mesh: &Mesh) {
        for face in mesh.faces() {
            for half_edge in mesh.face_loop(face) {
                let neighbor = mesh.twin(half_edge).and_then(|twin| mesh.face(twin));
                assert!(
                    neighbor.is_some_and(|f| f != FaceId::OUTSIDE),
                    "boundary half-edge in a result that must be closed"
                );
            }
        }
    }

    #[test]
    fn oracle_regression_5613_prism_box_intersection_is_closed_and_exact() {
        let a = oracle_5613_prism();
        let b = oracle_5613_box();
        let mut scratch = BooleanScratch::default();
        let mut diagnostics = BooleanDiagnostics::new(16);
        let output = boolean_mesh(
            &a,
            &b,
            BooleanOp::Intersection,
            FaceTriangulation::Robust,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("intersection succeeds");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(output.mesh.validate_deep().is_empty());
        assert_closed(&output.mesh);
        let volume = signed_volume(&output.mesh);
        assert!(
            (volume - 0.322_870_600).abs() < 1.0e-4,
            "intersection volume {volume} vs exact 0.3228706"
        );
    }

    #[test]
    fn oracle_regression_8653_prism_box_intersection_is_closed_and_exact() {
        let a = oracle_8653_prism();
        let b = oracle_8653_box();
        let mut scratch = BooleanScratch::default();
        let mut diagnostics = BooleanDiagnostics::new(16);
        let output = boolean_mesh(
            &a,
            &b,
            BooleanOp::Intersection,
            FaceTriangulation::Robust,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("intersection succeeds");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(output.mesh.validate_deep().is_empty());
        assert_closed(&output.mesh);
        let volume = signed_volume(&output.mesh);
        assert!(
            (volume - 0.495_480_360).abs() < 1.0e-4,
            "intersection volume {volume} vs exact 0.4954804"
        );
    }

    /// A high-resolution drill through a block, rotated 45 degrees off the
    /// axes (exe-lz4w): the rotated f32 lattice makes the cut loop a chain
    /// of near-collinear sliver constructions. Before the largest-triangle
    /// patch sample this misclassified a sliver patch, whose wrongly kept
    /// copy stitched a third face onto existing edges
    /// (`BuildError::NonManifoldEdge`).
    #[test]
    fn rotated_drill_difference_is_a_holed_watertight_solid() {
        let angle = core::f64::consts::FRAC_PI_4;
        let (sin, cos) = (angle.sin(), angle.cos());
        let rotate = |p: [f64; 3]| [p[0] * cos - p[1] * sin, p[0] * sin + p[1] * cos, p[2]];
        #[expect(clippy::cast_possible_truncation, reason = "test geometry narrowing")]
        let narrow = |p: [f64; 3]| [p[0] as f32, p[1] as f32, p[2] as f32];

        // Block: 200 x 100 x 80, corner at the origin.
        let mut builder = MeshBuilder::new();
        for p in [
            [0.0, 0.0, 0.0],
            [200.0, 0.0, 0.0],
            [200.0, 100.0, 0.0],
            [0.0, 100.0, 0.0],
            [0.0, 0.0, 80.0],
            [200.0, 0.0, 80.0],
            [200.0, 100.0, 80.0],
            [0.0, 100.0, 80.0],
        ] {
            builder.push_vertex(narrow(rotate(p)));
        }
        for face in [
            [3_u32, 2, 1, 0],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            builder.add_face(&face).expect("block face");
        }
        let block = builder.build().expect("valid block").mesh;

        // Drill: 96-gon prism r=30 at (130, 50), through both caps.
        let n = 96_u32;
        let mut builder = MeshBuilder::new();
        for z in [-20.0_f64, 100.0] {
            for i in 0..n {
                let theta = core::f64::consts::TAU * f64::from(i) / f64::from(n);
                let p = [130.0 + 30.0 * theta.cos(), 50.0 + 30.0 * theta.sin(), z];
                builder.push_vertex(narrow(rotate(p)));
            }
        }
        let bottom: Vec<u32> = (0..n).rev().collect();
        builder.add_face(&bottom).expect("bottom cap");
        let top: Vec<u32> = (n..2 * n).collect();
        builder.add_face(&top).expect("top cap");
        for i in 0..n {
            let j = (i + 1) % n;
            builder.add_face(&[i, j, n + j, n + i]).expect("side wall");
        }
        let drill = builder.build().expect("valid drill").mesh;

        let mut scratch = BooleanScratch::new();
        let mut diagnostics = BooleanDiagnostics::default();
        let output = boolean_mesh(
            &block,
            &drill,
            BooleanOp::Difference,
            FaceTriangulation::Robust,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("rotated drill difference succeeds");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(output.mesh.validate_deep().is_empty());
        assert_closed(&output.mesh);
        assert_eq!(euler_characteristic(&output.mesh), 0, "through-hole shell");
        let volume = signed_volume(&output.mesh);
        let hole = 0.5 * f64::from(n) * 30.0 * 30.0 * (core::f64::consts::TAU / f64::from(n)).sin();
        let expected = 200.0 * 100.0 * 80.0 - hole * 80.0;
        assert!(
            (volume - expected).abs() < 40.0,
            "difference volume {volume}, expected {expected}"
        );
    }
}
