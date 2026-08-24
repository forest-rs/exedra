// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The three-witness cross-check over one scenario case.
//!
//! A case (from [`crate::scenario`]) is an expression tree over operands,
//! evaluated three ways:
//!
//! 1. **Referee** (ground truth): min/max composition of per-operand
//!    union-of-convex-pieces pseudo-SDFs, in f64. Its sign is exact. Its
//!    magnitude bounds how far every leaf value can be perturbed without
//!    flipping the composed sign (min/max are 1-Lipschitz per argument and
//!    each leaf pseudo-SDF is 1-Lipschitz in space), so `|referee| > band`
//!    soundly excludes boundary-ambiguous points — never unsoundly
//!    includes them.
//! 2. **Mesh witness**: the exedra boolean pipeline folded over the tree
//!    (chained classes feed intermediate outputs back in as operands),
//!    then exact ray-parity membership on the result.
//! 3. **Field witness**: the `exedra_isosurface` CSG combinators composed
//!    over the placed analytic fields, sign sampled per point.
//!
//! The referee describes exactly the polyhedral solid the mesh pipeline
//! consumes (up to the documented f32 narrowing covered by the mesh band),
//! so any mesh disagreement outside the band is a mesh pipeline finding.
//! The field solid differs from the referee by at most the per-operand
//! Hausdorff deviation; any field sign flip outside the wider field band
//! is an isosurface finding. Bands scale linearly with the case's
//! coordinate scale.

use std::collections::{BTreeMap, BTreeSet};

use exedra::boolean::{
    BooleanDiagnostics, BooleanError, BooleanFailureKind, BooleanOp, BooleanOutput, BooleanScratch,
    boolean_mesh,
};
use exedra::{FaceTriangulation, Mesh, VertexId};
use exedra_isosurface::ScalarField;
use exedra_isosurface::analytic::{Difference, Intersection, Union};

use crate::membership::{Parity, point_in_triangles};
use crate::operands::{Operand, mesh_triangles_f64};
use crate::rng::SplitMix64;
use crate::scenario::{Case, Node, ScenarioClass, build_case};

/// Mesh-band at unit scale: covers f32 vertex narrowing, stitched
/// intersection-vertex rounding, and Newell/analytic plane slop.
pub(crate) const MESH_BAND: f64 = 1.0e-3;

/// Extra field slop at unit scale on top of per-operand Hausdorff
/// deviation: f32 SDF arithmetic and the f32 narrowing of sample points.
pub(crate) const FIELD_SLOP: f64 = 1.0e-3;

/// Why the mesh witness skipped a case.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum SkipReason {
    /// A coplanar candidate pair was diagnosed (typed deferral).
    CoplanarAmbiguity,
    /// A splitting configuration was deferred (typed deferral).
    SplitDeferred,
    /// Classification refused the result for another diagnosed reason.
    OtherSuspect,
    /// Otherwise-manifold operands form an edge-only non-manifold contact.
    NonManifoldContact,
    /// The pipeline reported an internal build failure.
    BuildFailure,
    /// The pipeline reported an invariant violation (typed).
    InvariantViolation,
}

impl SkipReason {
    /// Stable report key.
    #[must_use]
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::CoplanarAmbiguity => "coplanar_ambiguity",
            Self::SplitDeferred => "split_deferred",
            Self::OtherSuspect => "other_suspect",
            Self::NonManifoldContact => "non_manifold_contact",
            Self::BuildFailure => "build_failure",
            Self::InvariantViolation => "invariant_violation",
        }
    }
}

/// One cross-check disagreement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Finding {
    /// Which witness disagreed with the referee.
    pub(crate) witness: &'static str,
    /// Case seed (reproduce with `--case-seed <seed> --class <key>`).
    pub(crate) case_seed: u64,
    /// The sample point.
    pub(crate) point: [f64; 3],
    /// Referee membership.
    pub(crate) referee_inside: bool,
    /// Referee margin (value-space perturbation bound).
    pub(crate) margin: f64,
    /// Witness value: parity name or field value.
    pub(crate) witness_value: String,
    /// Case description (class, submode, operands, ops).
    pub(crate) describe: String,
}

/// Aggregated result of one case.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CaseOutcome {
    /// Points checked against the mesh witness.
    pub(crate) mesh_points: u64,
    /// Points checked against the field witness.
    pub(crate) field_points: u64,
    /// Points excluded from the mesh check by the band.
    pub(crate) mesh_band_points: u64,
    /// Points excluded from the field check by the band.
    pub(crate) field_band_points: u64,
    /// Points where ray parity exhausted every direction.
    pub(crate) exhausted_points: u64,
    /// Mesh-witness skip, when the pipeline typed-deferred or failed.
    pub(crate) skip: Option<SkipReason>,
    /// Placement/result sub-mode ("-" outside adversarial/empty-total).
    pub(crate) submode: &'static str,
    /// True when the mesh witness returned a zero-face (empty) result.
    pub(crate) empty_result: bool,
    /// Structural errors reported by the kernel's deep validator.
    pub(crate) mesh_validation_errors: u64,
    /// Boolean outputs whose face provenance or kept-face count disagrees
    /// with the emitted mesh.
    pub(crate) mesh_bookkeeping_errors: u64,
    /// Additional vertices that duplicate a stored position within one
    /// connected marked-seam component.
    pub(crate) seam_identity_conflicts: u64,
    /// Disagreements found in this case.
    pub(crate) findings: Vec<Finding>,
}

#[derive(Default)]
struct MeshChecks {
    validation_errors: u64,
    bookkeeping_errors: u64,
    seam_identity_conflicts: u64,
}

/// Applies structural checks before a chained Boolean output becomes the
/// operand of its parent expression.
///
/// Checking only the final tree result can hide a bad intermediate if a later
/// operation removes the affected faces. Provenance order and `kept_faces`
/// are checked here because face-cycle decomposition can turn one selected
/// source face into several emitted faces.
fn inspect_boolean_output(output: &BooleanOutput, checks: &mut MeshChecks) {
    checks.validation_errors = checks
        .validation_errors
        .saturating_add(u64::try_from(output.mesh.validate_deep().len()).unwrap_or(u64::MAX));
    checks.seam_identity_conflicts = checks
        .seam_identity_conflicts
        .saturating_add(seam_identity_conflicts(&output.mesh));

    let face_count = output.mesh.faces().count();
    let provenance_matches = output.face_provenance.len() == face_count
        && output
            .face_provenance
            .iter()
            .map(|&(face, _, _)| face)
            .eq(output.mesh.faces());
    if !provenance_matches {
        checks.bookkeeping_errors = checks.bookkeeping_errors.saturating_add(1);
    }
    if output.stats.kept_faces != u64::try_from(face_count).unwrap_or(u64::MAX) {
        checks.bookkeeping_errors = checks.bookkeeping_errors.saturating_add(1);
    }
}

/// Evaluates a tree through the boolean pipeline. Chained classes make
/// intermediate outputs operands of later stages by construction.
fn eval_mesh_tree(
    node: &Node,
    operands: &[Operand],
    scratch: &mut BooleanScratch,
    diagnostics: &mut BooleanDiagnostics,
    checks: &mut MeshChecks,
) -> Result<Mesh, SkipReason> {
    match node {
        Node::Leaf(index) => Ok(operands[*index].mesh.clone()),
        Node::Op(op, left, right) => {
            let left = eval_mesh_tree(left, operands, scratch, diagnostics, checks)?;
            let right = eval_mesh_tree(right, operands, scratch, diagnostics, checks)?;
            diagnostics.clear();
            let output = boolean_mesh(
                &left,
                &right,
                *op,
                FaceTriangulation::Robust,
                scratch,
                diagnostics,
            )
            .map_err(|error| classify_skip(&error, diagnostics))?;
            inspect_boolean_output(&output, checks);
            Ok(output.mesh)
        }
    }
}

/// Referee value over the tree.
pub(crate) fn referee_tree(node: &Node, operands: &[Operand], point: [f64; 3]) -> f64 {
    match node {
        Node::Leaf(index) => operands[*index].referee(point),
        Node::Op(op, left, right) => {
            let l = referee_tree(left, operands, point);
            let r = referee_tree(right, operands, point);
            match op {
                BooleanOp::Union => l.min(r),
                BooleanOp::Intersection => l.max(r),
                BooleanOp::Difference => l.max(-r),
            }
        }
    }
}

/// Field witness composed over the tree (leaves borrow operand fields).
fn field_tree<'a>(node: &Node, operands: &'a [Operand]) -> Box<dyn ScalarField + 'a> {
    match node {
        Node::Leaf(index) => Box::new(&*operands[*index].field),
        Node::Op(op, left, right) => {
            let l = field_tree(left, operands);
            let r = field_tree(right, operands);
            match op {
                BooleanOp::Union => Box::new(Union::new(l, r)),
                BooleanOp::Intersection => Box::new(Intersection::new(l, r)),
                BooleanOp::Difference => Box::new(Difference::new(l, r)),
            }
        }
    }
}

/// Runs one seeded case of one class end to end.
#[must_use]
pub(crate) fn run_case(class: ScenarioClass, case_seed: u64, points_per_case: u64) -> CaseOutcome {
    let case = build_case(class, case_seed);
    let mut rng = SplitMix64::new(case_seed ^ 0xA076_1D64_78BD_642F);

    let mut outcome = CaseOutcome {
        submode: case.submode,
        ..CaseOutcome::default()
    };

    // --- Mesh witness. ---
    let mut scratch = BooleanScratch::default();
    let mut diagnostics = BooleanDiagnostics::new(64);
    let mut mesh_checks = MeshChecks::default();
    let mesh_triangles = match eval_mesh_tree(
        &case.tree,
        &case.operands,
        &mut scratch,
        &mut diagnostics,
        &mut mesh_checks,
    ) {
        Ok(mesh) => {
            outcome.empty_result = mesh.faces().count() == 0;
            Some(mesh_triangles_f64(&mesh))
        }
        Err(reason) => {
            outcome.skip = Some(reason);
            None
        }
    };
    outcome.mesh_validation_errors = mesh_checks.validation_errors;
    outcome.mesh_bookkeeping_errors = mesh_checks.bookkeeping_errors;
    outcome.seam_identity_conflicts = mesh_checks.seam_identity_conflicts;

    // --- Field witness. ---
    let field = field_tree(&case.tree, &case.operands);
    let field_band = case
        .operands
        .iter()
        .map(|operand| operand.field_deviation)
        .fold(0.0_f64, f64::max)
        + FIELD_SLOP * case.band_scale;
    let mesh_band = MESH_BAND * case.band_scale;

    // --- Sample and compare. ---
    let bounds = case_bounds(&case.operands);
    let offset = 0.08 * case.band_scale;
    for _ in 0..points_per_case {
        let point = sample_point(&mut rng, &case, &bounds, offset);
        let referee = referee_tree(&case.tree, &case.operands, point);
        let referee_inside = referee < 0.0;
        let margin = referee.abs();

        // Mesh comparison.
        if let Some(triangles) = &mesh_triangles {
            if margin <= mesh_band {
                outcome.mesh_band_points += 1;
            } else {
                match point_in_triangles(point, triangles) {
                    Parity::Exhausted => outcome.exhausted_points += 1,
                    parity => {
                        outcome.mesh_points += 1;
                        let inside = parity == Parity::Inside;
                        if inside != referee_inside {
                            outcome.findings.push(Finding {
                                witness: "mesh",
                                case_seed,
                                point,
                                referee_inside,
                                margin,
                                witness_value: format!("{parity:?}"),
                                describe: case.describe.clone(),
                            });
                        }
                    }
                }
            }
        }

        // Field comparison.
        if margin <= field_band {
            outcome.field_band_points += 1;
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "field sampling narrows once; the band covers it"
            )]
            let point_f32 = [point[0] as f32, point[1] as f32, point[2] as f32];
            let mut value = [0.0_f32];
            field.eval_points(&[point_f32], &mut value);
            outcome.field_points += 1;
            let inside = value[0] < 0.0;
            if inside != referee_inside {
                outcome.findings.push(Finding {
                    witness: "field",
                    case_seed,
                    point,
                    referee_inside,
                    margin,
                    witness_value: format!("{}", value[0]),
                    describe: case.describe.clone(),
                });
            }
        }
    }
    outcome
}

/// Counts exact-position identity aliases within connected marked seams.
///
/// Equal positions in different shells are valid (for example a regularized
/// point contact), so this walks only edges carrying the Boolean seam mark and
/// scopes the position table to one connected seam component at a time.
fn seam_identity_conflicts(mesh: &Mesh) -> u64 {
    let mut adjacency = BTreeMap::<VertexId, Vec<VertexId>>::new();
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            if mesh.edge_seam(half_edge) != Some(true) {
                continue;
            }
            let Some((from, to)) = mesh.from_vertex(half_edge).zip(mesh.to_vertex(half_edge))
            else {
                continue;
            };
            adjacency.entry(from).or_default().push(to);
            adjacency.entry(to).or_default().push(from);
        }
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
        neighbors.dedup();
    }

    let mut visited = BTreeSet::new();
    let mut conflicts = 0_u64;
    for &seed in adjacency.keys() {
        if !visited.insert(seed) {
            continue;
        }
        let mut stack = vec![seed];
        let mut positions = BTreeMap::<[u32; 3], VertexId>::new();
        while let Some(vertex) = stack.pop() {
            if let Some(position) = mesh.vertex_position(vertex) {
                // Signed zero is one geometric mesh coordinate even though
                // IEEE-754 exposes two bit patterns for it.
                let key = position.map(|coordinate| {
                    if coordinate == 0.0 {
                        0.0_f32.to_bits()
                    } else {
                        coordinate.to_bits()
                    }
                });
                if positions.insert(key, vertex).is_some() {
                    conflicts += 1;
                }
            }
            if let Some(neighbors) = adjacency.get(&vertex) {
                for &neighbor in neighbors.iter().rev() {
                    if visited.insert(neighbor) {
                        stack.push(neighbor);
                    }
                }
            }
        }
    }
    conflicts
}

fn classify_skip(error: &BooleanError, diagnostics: &BooleanDiagnostics) -> SkipReason {
    match error {
        BooleanError::NonManifoldContact => SkipReason::NonManifoldContact,
        BooleanError::Build(_) => SkipReason::BuildFailure,
        BooleanError::InvariantViolation { .. } => SkipReason::InvariantViolation,
        BooleanError::SuspectPatches { .. } => {
            if diagnostics.count_of(BooleanFailureKind::SplitDeferred) > 0 {
                SkipReason::SplitDeferred
            } else if diagnostics.count_of(BooleanFailureKind::CoplanarAmbiguity) > 0 {
                SkipReason::CoplanarAmbiguity
            } else {
                SkipReason::OtherSuspect
            }
        }
        _ => SkipReason::OtherSuspect,
    }
}

pub(crate) struct Bounds {
    pub(crate) min: [f64; 3],
    pub(crate) max: [f64; 3],
}

pub(crate) fn case_bounds(operands: &[Operand]) -> Bounds {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut extent = 0.0_f64;
    for operand in operands {
        for vertex in operand.mesh.vertices() {
            if let Some(p) = operand.mesh.vertex_position(vertex) {
                for axis in 0..3 {
                    min[axis] = min[axis].min(f64::from(p[axis]));
                    max[axis] = max[axis].max(f64::from(p[axis]));
                }
            }
        }
    }
    for axis in 0..3 {
        extent = extent.max(max[axis] - min[axis]);
    }
    // Inflate so outside-near-the-solid points are common, proportionally
    // to the case extent (scale-independent).
    let pad = 0.12 * extent.max(1.0e-12);
    for axis in 0..3 {
        min[axis] -= pad;
        max[axis] += pad;
    }
    Bounds { min, max }
}

/// Stratified sampling: two thirds uniform in the inflated case bounds,
/// one third near operand surfaces (vertex plus a small scaled offset).
fn sample_point(rng: &mut SplitMix64, case: &Case, bounds: &Bounds, offset: f64) -> [f64; 3] {
    if rng.index(3) < 2 {
        return [
            rng.range_f64(bounds.min[0], bounds.max[0]),
            rng.range_f64(bounds.min[1], bounds.max[1]),
            rng.range_f64(bounds.min[2], bounds.max[2]),
        ];
    }
    let operand = &case.operands[rng.index(case.operands.len())];
    let vertices: Vec<_> = operand.mesh.vertices().collect();
    let vertex = vertices[rng.index(vertices.len())];
    let p = operand
        .mesh
        .vertex_position(vertex)
        .copied()
        .unwrap_or([0.0; 3]);
    [
        f64::from(p[0]) + rng.range_f64(-offset, offset),
        f64::from(p[1]) + rng.range_f64(-offset, offset),
        f64::from(p[2]) + rng.range_f64(-offset, offset),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curved_wall_seed_one_has_canonical_seam_identity() {
        // This chained curved-wall case formerly emitted three zero-length
        // marked edges at stage 1 because distinct graph points narrowed to
        // one f32 position. It must now remain supported and oracle-clean.
        let outcome = run_case(ScenarioClass::CurvedWall, 1, 400);

        assert_eq!(outcome.skip, None);
        assert_eq!(outcome.mesh_validation_errors, 0);
        assert_eq!(outcome.mesh_bookkeeping_errors, 0);
        assert_eq!(outcome.seam_identity_conflicts, 0);
        assert!(outcome.findings.is_empty());
    }

    #[test]
    fn chained_face_partition_keeps_boundary_continuation_unambiguous() {
        // A deep chained sweep found this partition order adding a sub-face
        // while two distinct OUTSIDE continuations met at vertex 16. The
        // splitter must either rebuild the face safely or report a typed
        // deferral; malformed intermediate topology must never reach the
        // stitcher's invariant panic.
        let outcome = run_case(ScenarioClass::Chained, 7_497_488_052_617_644_153, 1);

        assert_eq!(outcome.skip, Some(SkipReason::SplitDeferred));
        assert_eq!(outcome.mesh_validation_errors, 0);
        assert_eq!(outcome.mesh_bookkeeping_errors, 0);
        assert_eq!(outcome.seam_identity_conflicts, 0);
    }

    #[test]
    fn shared_edge_union_has_a_named_geometric_skip() {
        // This fixed adversarial seed is two boxes with a partial shared edge
        // under Union. The oracle must report the public geometric refusal,
        // never regress to its generic internal-build-failure bucket.
        let outcome = run_case(ScenarioClass::Adversarial, 11_008_669_762_952_232_555, 80);

        assert_eq!(outcome.submode, "shared_edge");
        assert_eq!(outcome.skip, Some(SkipReason::NonManifoldContact));
        assert!(outcome.findings.is_empty());
    }

    #[test]
    fn scale_sliver_collapse_is_a_typed_numerical_skip() {
        // This fixed kilo-scale seed contains an intersection seam edge whose
        // distinct f64 endpoints narrow to one f32 point. Collapsing the seam
        // would give a neighboring output edge four faces, so the pipeline
        // must report numerical uncertainty rather than leak BuildError.
        let outcome = run_case(ScenarioClass::Scale, 6_750_632_535_653_330_089, 80);

        assert_eq!(outcome.submode, "kilo");
        assert_eq!(outcome.skip, Some(SkipReason::OtherSuspect));
        assert_eq!(outcome.mesh_validation_errors, 0);
        assert_eq!(outcome.mesh_bookkeeping_errors, 0);
        assert_eq!(outcome.seam_identity_conflicts, 0);
        assert!(outcome.findings.is_empty());
    }
}

#[cfg(test)]
mod triage {
    use super::*;
    use crate::membership::point_in_triangles;
    use crate::operands::mesh_triangles_f64;

    fn env_class() -> ScenarioClass {
        std::env::var("ORACLE_CLASS")
            .ok()
            .and_then(|v| ScenarioClass::parse(&v))
            .unwrap_or(ScenarioClass::ConvexMixed)
    }

    fn env_seed(default: u64) -> u64 {
        std::env::var("ORACLE_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// Collects `(op, left_mesh, right_mesh)` for every Op node in
    /// post-order, evaluating through the pipeline.
    fn op_stages(
        node: &Node,
        operands: &[Operand],
        scratch: &mut BooleanScratch,
        diagnostics: &mut BooleanDiagnostics,
        stages: &mut Vec<(BooleanOp, Mesh, Mesh)>,
    ) -> Mesh {
        match node {
            Node::Leaf(index) => operands[*index].mesh.clone(),
            Node::Op(op, left, right) => {
                let left = op_stages(left, operands, scratch, diagnostics, stages);
                let right = op_stages(right, operands, scratch, diagnostics, stages);
                stages.push((*op, left.clone(), right.clone()));
                diagnostics.clear();
                boolean_mesh(
                    &left,
                    &right,
                    *op,
                    FaceTriangulation::Robust,
                    scratch,
                    diagnostics,
                )
                .map(|output| output.mesh)
                .unwrap_or(left)
            }
        }
    }

    /// Patch-level triage: rebuilds one boolean stage by hand and dumps
    /// every classified patch. `ORACLE_CLASS`/`ORACLE_SEED` pick the case,
    /// `ORACLE_STAGE` the post-order Op-node index (default: last).
    #[test]
    #[ignore = "triage tool, run by hand"]
    fn isolate_patches() {
        use exedra::boolean::{
            BooleanBvh, MeshSide, build_intersection_graph, classify_patches,
            collect_coplanar_contacts, narrow_phase, split_mesh_along_graph,
        };

        let case = build_case(env_class(), env_seed(5_613_533_614_605_315_259));
        eprintln!("case: {}", case.describe);
        let mut scratch = BooleanScratch::default();
        let mut diagnostics = BooleanDiagnostics::new(64);
        let mut stages = Vec::new();
        let _ = op_stages(
            &case.tree,
            &case.operands,
            &mut scratch,
            &mut diagnostics,
            &mut stages,
        );
        let stage: usize = std::env::var("ORACLE_STAGE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(stages.len().saturating_sub(1));
        let (op, mut split_a, mut split_b) = stages[stage].clone();
        eprintln!("stage={stage} op={op:?}");
        if std::env::var("ORACLE_DUMP").is_ok() {
            for (label, mesh) in [("A", &split_a), ("B", &split_b)] {
                eprint!("operand[{label}] vertices=[");
                for vertex in mesh.vertices() {
                    if let Some(p) = mesh.vertex_position(vertex) {
                        eprint!("[{:?},{:?},{:?}],", p[0], p[1], p[2]);
                    }
                }
                eprint!("] faces=[");
                for face in mesh.faces() {
                    eprint!("&[");
                    for half_edge in mesh.face_loop(face) {
                        if let Some(v) = mesh.from_vertex(half_edge) {
                            eprint!("{},", v.index());
                        }
                    }
                    eprint!("][..],");
                }
                eprintln!("]");
            }
        }

        let strategy = FaceTriangulation::Robust;
        diagnostics.clear();
        let bvh_a = BooleanBvh::build(&split_a, strategy, &mut scratch);
        let bvh_b = BooleanBvh::build(&split_b, strategy, &mut scratch);
        let mut pairs = Vec::new();
        bvh_a.query_overlaps(&bvh_b, &mut scratch, &mut pairs);
        let mut segments = Vec::new();
        narrow_phase(
            &split_a,
            &split_b,
            &pairs,
            strategy,
            &mut scratch,
            &mut segments,
            &mut diagnostics,
        );
        let mut contacts = Vec::new();
        collect_coplanar_contacts(
            &split_a,
            &split_b,
            &pairs,
            strategy,
            &mut scratch,
            &mut contacts,
            &mut diagnostics,
        );
        let graph = build_intersection_graph(
            &split_a,
            &split_b,
            &segments,
            strategy,
            &mut scratch,
            &mut diagnostics,
        );
        eprintln!(
            "graph: {} vertices, {} edges, {} polylines ({} closed)",
            graph.vertices.len(),
            graph.edges.len(),
            graph.polylines.len(),
            graph.polylines.iter().filter(|p| p.closed).count(),
        );
        for edge in &graph.edges {
            let [a, b] = edge.vertices.map(|index| index as usize);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "triage inspects the graph-to-mesh f32 boundary"
            )]
            let narrow = |point: [f64; 3]| point.map(|value| value as f32);
            if narrow(graph.vertices[a].position) == narrow(graph.vertices[b].position) {
                eprintln!(
                    "  zero graph edge {}-{} position={:?} anchors=({:?}, {:?})/({:?}, {:?})",
                    a,
                    b,
                    narrow(graph.vertices[a].position),
                    graph.vertices[a].anchor_a,
                    graph.vertices[b].anchor_a,
                    graph.vertices[a].anchor_b,
                    graph.vertices[b].anchor_b,
                );
            }
        }
        let outcome_a = split_mesh_along_graph(&mut split_a, &graph, MeshSide::A, &mut diagnostics);
        let outcome_b = split_mesh_along_graph(&mut split_b, &graph, MeshSide::B, &mut diagnostics);
        eprintln!(
            "split A: {:?}\nsplit B: {:?}",
            outcome_a.stats, outcome_b.stats
        );
        let classification = classify_patches(
            &split_a,
            &split_b,
            &graph,
            &outcome_a,
            &outcome_b,
            &contacts,
            strategy,
            &mut scratch,
            &mut diagnostics,
        );
        eprintln!("diag_clean={}", diagnostics.is_clean());
        for entry in diagnostics.entries() {
            eprintln!("  diag: {entry:?}");
        }
        let tri_a = mesh_triangles_f64(&split_a);
        let tri_b = mesh_triangles_f64(&split_b);
        for (index, patch) in classification.patches.iter().enumerate() {
            let (mesh, other) = match patch.mesh {
                MeshSide::A => (&split_a, &tri_b),
                MeshSide::B => (&split_b, &tri_a),
            };
            eprintln!(
                "patch[{index}] mesh={:?} side={:?} faces={}",
                patch.mesh,
                patch.side,
                patch.faces.len()
            );
            for &face in &patch.faces {
                let mut sum = [0.0_f64; 3];
                let mut count = 0.0;
                for half_edge in mesh.face_loop(face) {
                    if let Some(p) = mesh
                        .to_vertex(half_edge)
                        .and_then(|v| mesh.vertex_position(v))
                    {
                        sum = [
                            sum[0] + f64::from(p[0]),
                            sum[1] + f64::from(p[1]),
                            sum[2] + f64::from(p[2]),
                        ];
                        count += 1.0;
                    }
                }
                let centroid = [sum[0] / count, sum[1] / count, sum[2] / count];
                let parity = point_in_triangles(centroid, other);
                eprintln!(
                    "  face {} centroid=({:.6},{:.6},{:.6}) other-parity={parity:?}",
                    face.index(),
                    centroid[0],
                    centroid[1],
                    centroid[2]
                );
            }
        }
    }

    /// Stage-isolating reproduction: which Op node first disagrees with
    /// the referee? `ORACLE_CLASS`/`ORACLE_SEED` pick the case.
    #[test]
    #[ignore = "triage tool, run by hand"]
    fn isolate_case() {
        let class = env_class();
        let case = build_case(class, env_seed(5_259_526_084_160_495_857));
        eprintln!("case: {}", case.describe);

        // Evaluate every Op node in post-order and cross-check each result
        // against its own sub-tree referee with a fresh scan.
        fn walk(
            node: &Node,
            case: &Case,
            scratch: &mut BooleanScratch,
            diagnostics: &mut BooleanDiagnostics,
            counter: &mut usize,
        ) -> Mesh {
            match node {
                Node::Leaf(index) => case.operands[*index].mesh.clone(),
                Node::Op(op, left, right) => {
                    let left = walk(left, case, scratch, diagnostics, counter);
                    let right = walk(right, case, scratch, diagnostics, counter);
                    diagnostics.clear();
                    let result = boolean_mesh(
                        &left,
                        &right,
                        *op,
                        FaceTriangulation::Robust,
                        scratch,
                        diagnostics,
                    );
                    let stage = *counter;
                    *counter += 1;
                    match result {
                        Err(error) => {
                            eprintln!("stage {stage} ({op:?}): typed failure {error:?}");
                            for entry in diagnostics.entries() {
                                eprintln!("  diag: {entry:?}");
                            }
                            left
                        }
                        Ok(output) => {
                            let mesh = output.mesh;
                            let deep = mesh.validate_deep().len();
                            let zero_length_edges = mesh
                                .faces()
                                .flat_map(|face| mesh.face_loop(face))
                                .filter(|&half_edge| {
                                    mesh.from_vertex(half_edge)
                                        .and_then(|vertex| mesh.vertex_position(vertex))
                                        .zip(
                                            mesh.to_vertex(half_edge)
                                                .and_then(|vertex| mesh.vertex_position(vertex)),
                                        )
                                        .is_some_and(|(from, to)| from == to)
                                })
                                .count()
                                / 2;
                            if zero_length_edges > 0 {
                                for face in mesh.faces() {
                                    for half_edge in mesh.face_loop(face) {
                                        let Some((from_vertex, to_vertex)) = mesh
                                            .from_vertex(half_edge)
                                            .zip(mesh.to_vertex(half_edge))
                                        else {
                                            continue;
                                        };
                                        if from_vertex.index() >= to_vertex.index()
                                            || mesh.vertex_position(from_vertex)
                                                != mesh.vertex_position(to_vertex)
                                        {
                                            continue;
                                        }
                                        eprintln!(
                                            "  zero edge {}-{} position={:?} seam={:?} sharpness={:?}",
                                            from_vertex.index(),
                                            to_vertex.index(),
                                            mesh.vertex_position(from_vertex),
                                            mesh.edge_seam(half_edge),
                                            mesh.edge_sharpness(half_edge),
                                        );
                                    }
                                }
                            }
                            let triangles = mesh_triangles_f64(&mesh);
                            let mut volume = 0.0_f64;
                            for [a, b, c] in &triangles {
                                volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
                                    - a[1] * (b[0] * c[2] - b[2] * c[0])
                                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                                    / 6.0;
                            }
                            eprintln!(
                                "stage {stage} ({op:?}): faces={} deep_errors={deep} zero_length_edges={zero_length_edges} volume={volume:.6} diag_clean={}",
                                mesh.faces().count(),
                                diagnostics.is_clean(),
                            );
                            // Scan this sub-tree's referee against the mesh.
                            let bounds = case_bounds(&case.operands);
                            let mut scan = SplitMix64::new(23);
                            let mut printed = 0;
                            let band = MESH_BAND * case.band_scale;
                            for _ in 0..40_000 {
                                let p = [
                                    scan.range_f64(bounds.min[0], bounds.max[0]),
                                    scan.range_f64(bounds.min[1], bounds.max[1]),
                                    scan.range_f64(bounds.min[2], bounds.max[2]),
                                ];
                                let value = referee_tree(node, &case.operands, p);
                                if value.abs() <= band {
                                    continue;
                                }
                                let parity = point_in_triangles(p, &triangles);
                                let inside = parity == Parity::Inside;
                                if parity != Parity::Exhausted && inside != (value < 0.0) {
                                    eprintln!(
                                        "  disagree point=({:.6},{:.6},{:.6}) referee={value:+.6} parity={parity:?}",
                                        p[0], p[1], p[2]
                                    );
                                    printed += 1;
                                    if printed >= 5 {
                                        break;
                                    }
                                }
                            }
                            mesh
                        }
                    }
                }
            }
        }
        let mut scratch = BooleanScratch::default();
        let mut diagnostics = BooleanDiagnostics::new(64);
        let mut counter = 0;
        let _ = walk(
            &case.tree,
            &case,
            &mut scratch,
            &mut diagnostics,
            &mut counter,
        );
    }
}
