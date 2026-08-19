// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Workflow-level boolean orchestration: preview vs commit staging over the
//! Exedra boolean pipeline.
//!
//! - **Preview** answers "where would this boolean act, and does it look
//!   sane?" cheaply: AABB overlap gate, broad-phase candidate discovery,
//!   and the intersection curves as exportable polyline artifacts — no
//!   splitting, no stitching.
//! - **Commit** runs the full robust pipeline (`exedra::boolean::boolean_mesh`)
//!   and applies workflow policy: typed failure on suspect patches
//!   (surfaced as Cambium diagnostics), and policy-driven tiny-component
//!   cleanup with every removal reported, never silent.
//!
//! Both stages record lifecycle timings and reuse one `BooleanScratch`.

use alloc::vec::Vec;

use exedra::boolean::{
    BooleanBvh, BooleanDiagnostics, BooleanError, BooleanOp, BooleanOutput, BooleanScratch,
    SeamCleanupPolicy, SeamCleanupStats, build_intersection_graph, cleanup_seams, narrow_phase,
};
use exedra::{FaceTriangulation, Mesh};

use crate::context::Clock;
use crate::diag::{DiagCode, DiagLevel, Diagnostic};
use crate::report::Timings;

/// Workflow policy for boolean commits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BooleanRunPolicy {
    /// Triangulation strategy for the pipeline.
    pub strategy: FaceTriangulation,
    /// Connected result components with fewer faces than this are removed
    /// during cleanup (0 disables cleanup). Every removal is reported.
    pub min_component_faces: u32,
    /// Opt-in seam sliver cleanup applied after the pipeline (and after
    /// tiny-component removal); `None` leaves the output untouched.
    pub seam_cleanup: Option<SeamCleanupPolicy>,
}

impl Default for BooleanRunPolicy {
    fn default() -> Self {
        Self {
            strategy: FaceTriangulation::Fan,
            min_component_faces: 0,
            seam_cleanup: None,
        }
    }
}

/// Cheap preview of a boolean's action region.
#[derive(Debug)]
pub struct BooleanPreview {
    /// True when the operand bounds overlap at all; when false everything
    /// below is trivially empty and the boolean is a no-op (or, for
    /// Difference, operand A unchanged).
    pub bounds_overlap: bool,
    /// Broad-phase candidate pair count.
    pub candidate_pairs: usize,
    /// Intersection curves as world-space polylines (open chains and
    /// closed loops), exportable as viewer artifacts.
    pub curves: Vec<Vec<[f64; 3]>>,
    /// Mapped diagnostics (coplanar deferrals and friends).
    pub diagnostics: Vec<Diagnostic>,
    /// Lifecycle timings (`boolean.preview` bucket).
    pub timings: Timings,
}

/// A committed boolean result plus workflow reporting.
#[derive(Debug)]
pub struct BooleanCommit {
    /// The pipeline output (mesh, provenance, classification, stats).
    pub output: BooleanOutput,
    /// Faces removed by tiny-component cleanup (empty when disabled).
    pub removed_component_faces: u32,
    /// Seam sliver cleanup counters (`None` when disabled).
    pub seam_cleanup: Option<SeamCleanupStats>,
    /// Mapped diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Lifecycle timings (`boolean.commit` / `boolean.cleanup` buckets).
    pub timings: Timings,
}

/// Why a commit failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum BooleanCommitError {
    /// The pipeline refused (suspect patches, internal invariants); the
    /// mapped diagnostics carry the details.
    Pipeline {
        /// The underlying pipeline error.
        error: BooleanError,
        /// Mapped diagnostics accumulated up to the failure.
        diagnostics: Vec<Diagnostic>,
    },
}

impl core::fmt::Display for BooleanCommitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Pipeline { error, .. } => write!(f, "boolean pipeline failed: {error:?}"),
        }
    }
}

impl core::error::Error for BooleanCommitError {}

fn map_diagnostics(source: &BooleanDiagnostics) -> Vec<Diagnostic> {
    source
        .entries()
        .iter()
        .map(|d| {
            Diagnostic::new(
                DiagLevel::Warn,
                DiagCode::UnsupportedOperation,
                alloc::format!("{d}"),
            )
        })
        .collect()
}

/// Previews a boolean: bounds gate, broad phase, and intersection curves.
///
/// Never fails: an empty preview is an answer ("these operands do not
/// interact").
#[must_use]
pub fn preview_boolean(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    policy: &BooleanRunPolicy,
    scratch: &mut BooleanScratch,
) -> BooleanPreview {
    let clock = Clock::new(Timings::DEFAULT_MAX_BUCKETS);
    let bucket = clock.bucket("boolean.preview");

    let mut diagnostics = BooleanDiagnostics::default();

    // AABB gate: build both BVHs and query overlaps (the broad phase's
    // root bounds double as the operand bounds).
    let bvh_a = BooleanBvh::build(mesh_a, policy.strategy, scratch);
    let bvh_b = BooleanBvh::build(mesh_b, policy.strategy, scratch);
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
            policy.strategy,
            scratch,
            &mut segments,
            &mut diagnostics,
        );
        let graph = build_intersection_graph(
            mesh_a,
            mesh_b,
            &segments,
            policy.strategy,
            scratch,
            &mut diagnostics,
        );
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

    // Close the bucket before snapshotting: it records on drop.
    drop(bucket);
    BooleanPreview {
        bounds_overlap,
        candidate_pairs: pairs.len(),
        curves,
        diagnostics: map_diagnostics(&diagnostics),
        timings: clock.timings(),
    }
}

/// Commits a boolean: the full robust pipeline plus workflow cleanup.
///
/// # Errors
///
/// Fails typed when the pipeline refuses (suspect patches); the error
/// carries the mapped diagnostics.
pub fn commit_boolean(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    op: BooleanOp,
    policy: &BooleanRunPolicy,
    scratch: &mut BooleanScratch,
) -> Result<BooleanCommit, BooleanCommitError> {
    let clock = Clock::new(Timings::DEFAULT_MAX_BUCKETS);
    let mut diagnostics = BooleanDiagnostics::default();

    let mut output = {
        let _bucket = clock.bucket("boolean.commit");
        exedra::boolean::boolean_mesh(
            mesh_a,
            mesh_b,
            op,
            policy.strategy,
            scratch,
            &mut diagnostics,
        )
        .map_err(|error| BooleanCommitError::Pipeline {
            error,
            diagnostics: map_diagnostics(&diagnostics),
        })?
    };

    let mut mapped = map_diagnostics(&diagnostics);
    let mut removed = 0_u32;
    if policy.min_component_faces > 0 {
        let _bucket = clock.bucket("boolean.cleanup");
        removed = remove_tiny_components(&mut output.mesh, policy.min_component_faces);
        if removed > 0 {
            mapped.push(Diagnostic::new(
                DiagLevel::Note,
                DiagCode::UnsupportedOperation,
                alloc::format!(
                    "boolean cleanup removed {removed} faces in components smaller than {}",
                    policy.min_component_faces
                ),
            ));
            // Cleanup edits the mesh; provenance rows for removed faces
            // now point at dead ids, which downstream must tolerate.
            output
                .face_provenance
                .retain(|(face, _, _)| output_face_alive(&output.mesh, *face));
        }
    }

    let mut seam_stats = None;
    if let Some(cleanup_policy) = &policy.seam_cleanup {
        let _bucket = clock.bucket("boolean.seam_cleanup");
        let stats = cleanup_seams(&mut output.mesh, cleanup_policy);
        if stats.collapses + stats.flips > 0 {
            mapped.push(Diagnostic::new(
                DiagLevel::Note,
                DiagCode::UnsupportedOperation,
                alloc::format!(
                    "seam cleanup applied {} collapses and {} flips",
                    stats.collapses,
                    stats.flips
                ),
            ));
            // Collapses remove faces; drop provenance rows for dead ids.
            output
                .face_provenance
                .retain(|(face, _, _)| output_face_alive(&output.mesh, *face));
        }
        seam_stats = Some(stats);
    }

    Ok(BooleanCommit {
        output,
        removed_component_faces: removed,
        seam_cleanup: seam_stats,
        diagnostics: mapped,
        timings: clock.timings(),
    })
}

fn output_face_alive(mesh: &Mesh, face: exedra::FaceId) -> bool {
    mesh.faces().any(|f| f == face)
}

/// Removes connected face components smaller than `min_faces`; returns the
/// number of faces removed. Deterministic: components discovered in face
/// slot order.
fn remove_tiny_components(mesh: &mut Mesh, min_faces: u32) -> u32 {
    use exedra::FaceId;
    let faces: Vec<FaceId> = mesh.faces().collect();
    // BTreeSet: deterministic and no extra dependency.
    let mut assigned: alloc::collections::BTreeSet<u32> = alloc::collections::BTreeSet::new();
    let mut doomed: Vec<FaceId> = Vec::new();

    for &seed in &faces {
        if assigned.contains(&seed.index()) {
            continue;
        }
        // Flood-fill one component across shared edges.
        let mut component = alloc::vec![seed];
        assigned.insert(seed.index());
        let mut cursor = 0;
        while cursor < component.len() {
            let face = component[cursor];
            cursor += 1;
            for he in mesh.face_loop(face) {
                if let Some(twin) = mesh.twin(he)
                    && let Some(neighbor) = mesh.face(twin)
                    && neighbor != FaceId::OUTSIDE
                    && !assigned.contains(&neighbor.index())
                {
                    assigned.insert(neighbor.index());
                    component.push(neighbor);
                }
            }
        }
        if (component.len() as u64) < u64::from(min_faces) {
            doomed.extend(component);
        }
    }

    if doomed.is_empty() {
        return 0;
    }
    doomed.sort_unstable();
    let count = u32::try_from(doomed.len()).unwrap_or(u32::MAX);
    let mut session = mesh.edit();
    let _ = exedra::op::delete_faces(&mut session, &doomed, exedra::DeletePolicy::CleanupIsolated);
    #[expect(unused_must_use, reason = "sink output unused")]
    {
        session.finish();
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::builders;
    use exedra_constructive::evaluate::evaluate;
    use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
    use exedra_constructive::tessellate::EvalPolicy;

    fn cube_at(offset: f64) -> Mesh {
        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let n = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::translate(offset, offset, offset),
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let recipe = b.finish(n).expect("valid recipe");
        let mut result = evaluate(&recipe, &EvalPolicy::default()).expect("evaluates");
        result.bodies.remove(0).body.mesh.clone()
    }

    #[test]
    fn preview_reports_overlap_and_curves() {
        let a = cube_at(0.0);
        let b = cube_at(0.5);
        let mut scratch = BooleanScratch::default();
        let preview = preview_boolean(&a, &b, &BooleanRunPolicy::default(), &mut scratch);
        assert!(preview.bounds_overlap);
        assert!(preview.candidate_pairs > 0);
        assert!(!preview.curves.is_empty(), "intersection curves exported");
        let bucket_names: Vec<&str> = preview.timings.iter().map(|t| t.name).collect();
        assert!(bucket_names.contains(&"boolean.preview"));
    }

    #[test]
    fn preview_of_disjoint_operands_is_calm() {
        let a = cube_at(0.0);
        let b = cube_at(10.0);
        let mut scratch = BooleanScratch::default();
        let preview = preview_boolean(&a, &b, &BooleanRunPolicy::default(), &mut scratch);
        assert!(!preview.bounds_overlap);
        assert!(preview.curves.is_empty());
    }

    #[test]
    fn commit_runs_full_pipeline_with_timings() {
        let a = cube_at(0.0);
        let b = cube_at(0.5);
        let mut scratch = BooleanScratch::default();
        let commit = commit_boolean(
            &a,
            &b,
            BooleanOp::Union,
            &BooleanRunPolicy::default(),
            &mut scratch,
        )
        .expect("commits");
        assert!(commit.output.mesh.validate_deep().is_empty());
        assert!(!commit.output.face_provenance.is_empty());
        assert_eq!(commit.removed_component_faces, 0);
        let bucket_names: Vec<&str> = commit.timings.iter().map(|t| t.name).collect();
        assert!(bucket_names.contains(&"boolean.commit"));
    }

    #[test]
    fn cleanup_policy_leaves_healthy_results_alone() {
        // A watertight two-cube union has one large component: a sane
        // threshold removes nothing and reports nothing.
        let a = cube_at(0.0);
        let b = cube_at(0.5);
        let mut scratch = BooleanScratch::default();
        let commit = commit_boolean(
            &a,
            &b,
            BooleanOp::Union,
            &BooleanRunPolicy {
                min_component_faces: 4,
                ..BooleanRunPolicy::default()
            },
            &mut scratch,
        )
        .expect("commits");
        assert_eq!(commit.removed_component_faces, 0);
        assert!(commit.output.mesh.validate_deep().is_empty());
    }

    #[test]
    fn seam_cleanup_opt_in_reports_stats_and_stays_off_by_default() {
        let a = cube_at(0.0);
        let b = cube_at(0.5);
        let mut scratch = BooleanScratch::default();
        let default_commit = commit_boolean(
            &a,
            &b,
            BooleanOp::Union,
            &BooleanRunPolicy::default(),
            &mut scratch,
        )
        .expect("commits");
        assert!(default_commit.seam_cleanup.is_none(), "off by default");

        let commit = commit_boolean(
            &a,
            &b,
            BooleanOp::Union,
            &BooleanRunPolicy {
                seam_cleanup: Some(SeamCleanupPolicy::default()),
                ..BooleanRunPolicy::default()
            },
            &mut scratch,
        )
        .expect("commits");
        let stats = commit.seam_cleanup.expect("stats reported when enabled");
        // The overlapping-cube union has no seam slivers to fix; the pass
        // must run, report, and leave the healthy result alone.
        assert_eq!(stats.collapses + stats.flips, 0, "{stats:?}");
        assert!(commit.output.mesh.validate_deep().is_empty());
        let bucket_names: Vec<&str> = commit.timings.iter().map(|t| t.name).collect();
        assert!(bucket_names.contains(&"boolean.seam_cleanup"));
    }
}
