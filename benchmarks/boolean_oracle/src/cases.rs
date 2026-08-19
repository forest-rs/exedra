// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Random CSG cases and the three-witness cross-check.
//!
//! Each case folds 2..=4 random convex operands through random boolean
//! operations and evaluates the composition three ways:
//!
//! 1. **Referee** (ground truth): min/max composition of per-operand convex
//!    half-space pseudo-SDFs, in f64. Its sign is exact. Its magnitude is a
//!    lower bound on the distance to the composed boundary (min/max/negate
//!    preserve leaf signs, and each leaf pseudo-SDF under-reports distance),
//!    so `|referee| > band` guarantees the point is at least `band` away
//!    from the boundary — sound exclusion, never unsound inclusion.
//! 2. **Mesh witness**: the exedra boolean pipeline folded over the operand
//!    meshes, then exact ray-parity membership on the stitched result.
//! 3. **Field witness**: the `exedra_isosurface` CSG combinators folded over
//!    the placed analytic fields, sign sampled per point.
//!
//! The referee describes exactly the polyhedral solid the mesh pipeline
//! consumes, so any mesh disagreement outside the (small) band is a mesh
//! pipeline finding. The field solid differs from the referee by at most the
//! per-operand Hausdorff deviation, so any field sign flip outside the
//! (wider) field band is an isosurface finding.

use exedra::FaceTriangulation;
use exedra::boolean::{
    BooleanDiagnostics, BooleanError, BooleanFailureKind, BooleanOp, BooleanScratch, boolean_mesh,
};
use exedra_isosurface::ScalarField;
use exedra_isosurface::analytic::{Difference, Intersection, Union};

use crate::membership::{Parity, point_in_triangles};
use crate::operands::{Operand, mesh_triangles_f64, random_operand};
use crate::rng::SplitMix64;

/// Mesh-band: covers f32 vertex narrowing, stitched intersection-vertex
/// rounding, and Newell-plane slop on transformed quads.
pub(crate) const MESH_BAND: f64 = 1.0e-3;

/// Extra field slop on top of per-operand Hausdorff deviation: f32 SDF
/// arithmetic and the f32 narrowing of sample points.
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
    /// The pipeline reported an internal build failure.
    BuildFailure,
}

impl SkipReason {
    /// Stable report key.
    #[must_use]
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::CoplanarAmbiguity => "coplanar_ambiguity",
            Self::SplitDeferred => "split_deferred",
            Self::OtherSuspect => "other_suspect",
            Self::BuildFailure => "build_failure",
        }
    }
}

/// One cross-check disagreement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Finding {
    /// Which witness disagreed with the referee.
    pub(crate) witness: &'static str,
    /// Case seed (reproduce with `--seed <seed> --cases 1`).
    pub(crate) case_seed: u64,
    /// The sample point.
    pub(crate) point: [f64; 3],
    /// Referee membership.
    pub(crate) referee_inside: bool,
    /// Referee margin (distance lower bound).
    pub(crate) margin: f64,
    /// Witness value: parity name or field value.
    pub(crate) witness_value: String,
    /// Case description (operands + ops).
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
    /// Disagreements found in this case.
    pub(crate) findings: Vec<Finding>,
}

/// Runs one seeded case end to end.
#[must_use]
pub(crate) fn run_case(case_seed: u64, points_per_case: u64) -> CaseOutcome {
    let mut rng = SplitMix64::new(case_seed);
    let operand_count = 2 + rng.index(3);
    let operands: Vec<Operand> = (0..operand_count)
        .map(|_| random_operand(&mut rng))
        .collect();
    let ops: Vec<BooleanOp> = (1..operand_count)
        .map(|_| {
            [
                BooleanOp::Union,
                BooleanOp::Intersection,
                BooleanOp::Difference,
            ][rng.index(3)]
        })
        .collect();
    let describe = describe_case(&operands, &ops);

    let mut outcome = CaseOutcome::default();

    // --- Mesh witness: fold the boolean pipeline over the operands. ---
    let mut scratch = BooleanScratch::default();
    let mut diagnostics = BooleanDiagnostics::new(64);
    let mut mesh = operands[0].mesh.clone();
    for (operand, op) in operands[1..].iter().zip(&ops) {
        diagnostics.clear();
        match boolean_mesh(
            &mesh,
            &operand.mesh,
            *op,
            FaceTriangulation::Robust,
            &mut scratch,
            &mut diagnostics,
        ) {
            Ok(output) => mesh = output.mesh,
            Err(error) => {
                outcome.skip = Some(classify_skip(&error, &diagnostics));
                break;
            }
        }
    }
    let mesh_triangles = if outcome.skip.is_none() {
        Some(mesh_triangles_f64(&mesh))
    } else {
        None
    };

    // --- Field witness: fold the isosurface combinators over borrowed
    // operand fields (each operand appears exactly once in the tree). ---
    let mut field: Box<dyn ScalarField + '_> = Box::new(&*operands[0].field);
    for (operand, op) in operands[1..].iter().zip(&ops) {
        let right: &dyn ScalarField = &*operand.field;
        field = match op {
            BooleanOp::Union => Box::new(Union::new(field, right)),
            BooleanOp::Intersection => Box::new(Intersection::new(field, right)),
            BooleanOp::Difference => Box::new(Difference::new(field, right)),
        };
    }

    // Field band: the composed field deviates from the referee by at most
    // the largest per-operand deviation (min/max composition is 1-Lipschitz
    // in each operand).
    let field_band = operands
        .iter()
        .map(|operand| operand.field_deviation)
        .fold(0.0_f64, f64::max)
        + FIELD_SLOP;

    // --- Sample and compare. ---
    let bounds = case_bounds(&operands);
    for _ in 0..points_per_case {
        let point = sample_point(&mut rng, &operands, &bounds);
        let referee = referee_value(&operands, &ops, point);
        let referee_inside = referee < 0.0;
        let margin = referee.abs();

        // Mesh comparison.
        if let Some(triangles) = &mesh_triangles {
            if margin <= MESH_BAND {
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
                                describe: describe.clone(),
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
                    describe: describe.clone(),
                });
            }
        }
    }
    outcome
}

/// Referee value: fold pseudo-SDFs with min/max/negate.
fn referee_value(operands: &[Operand], ops: &[BooleanOp], point: [f64; 3]) -> f64 {
    let mut value = operands[0].referee(point);
    for (operand, op) in operands[1..].iter().zip(ops) {
        let right = operand.referee(point);
        value = match op {
            BooleanOp::Union => value.min(right),
            BooleanOp::Intersection => value.max(right),
            BooleanOp::Difference => value.max(-right),
        };
    }
    value
}

fn classify_skip(error: &BooleanError, diagnostics: &BooleanDiagnostics) -> SkipReason {
    match error {
        BooleanError::Build(_) => SkipReason::BuildFailure,
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

struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

fn case_bounds(operands: &[Operand]) -> Bounds {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
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
    // Inflate so outside-near-the-solid points are common.
    for axis in 0..3 {
        min[axis] -= 0.25;
        max[axis] += 0.25;
    }
    Bounds { min, max }
}

/// Stratified sampling: two thirds uniform in the inflated case bounds,
/// one third near operand surfaces (vertex plus a small offset).
fn sample_point(rng: &mut SplitMix64, operands: &[Operand], bounds: &Bounds) -> [f64; 3] {
    if rng.index(3) < 2 {
        return [
            rng.range_f64(bounds.min[0], bounds.max[0]),
            rng.range_f64(bounds.min[1], bounds.max[1]),
            rng.range_f64(bounds.min[2], bounds.max[2]),
        ];
    }
    let operand = &operands[rng.index(operands.len())];
    let vertices: Vec<_> = operand.mesh.vertices().collect();
    let vertex = vertices[rng.index(vertices.len())];
    let p = operand
        .mesh
        .vertex_position(vertex)
        .copied()
        .unwrap_or([0.0; 3]);
    [
        f64::from(p[0]) + rng.range_f64(-0.08, 0.08),
        f64::from(p[1]) + rng.range_f64(-0.08, 0.08),
        f64::from(p[2]) + rng.range_f64(-0.08, 0.08),
    ]
}

fn describe_case(operands: &[Operand], ops: &[BooleanOp]) -> String {
    let mut text = operands[0].describe.clone();
    for (operand, op) in operands[1..].iter().zip(ops) {
        let symbol = match op {
            BooleanOp::Union => "+",
            BooleanOp::Intersection => "&",
            BooleanOp::Difference => "-",
        };
        text.push_str(&format!(" {symbol} [{}]", operand.describe));
    }
    text
}

#[cfg(test)]
mod triage {
    use super::*;
    use crate::membership::point_in_triangles;
    use crate::operands::mesh_triangles_f64;

    /// Patch-level triage: rebuilds one boolean stage by hand and dumps
    /// every classified patch (side, face count, per-face centroid parity
    /// against the other operand) to spot flood-fill leaks and wrong patch
    /// selections. `ORACLE_SEED` picks the case, `ORACLE_STAGE` the fold
    /// stage (default 0).
    #[test]
    #[ignore = "triage tool, run by hand"]
    fn isolate_patches() {
        use exedra::boolean::{
            BooleanBvh, MeshSide, build_intersection_graph, classify_patches,
            collect_coplanar_contacts, narrow_phase, split_mesh_along_graph,
        };

        let case_seed: u64 = std::env::var("ORACLE_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_613_533_614_605_315_259);
        let stage: usize = std::env::var("ORACLE_STAGE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let mut rng = SplitMix64::new(case_seed);
        let operand_count = 2 + rng.index(3);
        let operands: Vec<Operand> = (0..operand_count)
            .map(|_| random_operand(&mut rng))
            .collect();
        let ops: Vec<BooleanOp> = (1..operand_count)
            .map(|_| {
                [
                    BooleanOp::Union,
                    BooleanOp::Intersection,
                    BooleanOp::Difference,
                ][rng.index(3)]
            })
            .collect();
        eprintln!("ops={ops:?} stage={stage}");
        if std::env::var("ORACLE_DUMP").is_ok() {
            for (i, operand) in operands.iter().enumerate() {
                eprint!("operand[{i}] vertices=[");
                for vertex in operand.mesh.vertices() {
                    if let Some(p) = operand.mesh.vertex_position(vertex) {
                        eprint!("[{:?},{:?},{:?}],", p[0], p[1], p[2]);
                    }
                }
                eprint!("] faces=[");
                for face in operand.mesh.faces() {
                    eprint!("&[");
                    for half_edge in operand.mesh.face_loop(face) {
                        if let Some(v) = operand.mesh.from_vertex(half_edge) {
                            eprint!("{},", v.index());
                        }
                    }
                    eprint!("][..],");
                }
                eprintln!("]");
            }
        }

        // Fold up to the requested stage; mesh_a is the running result.
        let mut scratch = BooleanScratch::default();
        let mut diagnostics = BooleanDiagnostics::new(64);
        let mut mesh_a = operands[0].mesh.clone();
        for (operand, op) in operands[1..=stage.min(ops.len() - 1)]
            .iter()
            .zip(&ops)
            .take(stage)
        {
            diagnostics.clear();
            let output = boolean_mesh(
                &mesh_a,
                &operand.mesh,
                *op,
                FaceTriangulation::Robust,
                &mut scratch,
                &mut diagnostics,
            )
            .expect("pre-stage boolean should succeed");
            mesh_a = output.mesh;
        }
        let mesh_b = operands[stage + 1].mesh.clone();

        // Run the stage's pipeline by hand.
        let strategy = FaceTriangulation::Robust;
        let mut split_a = mesh_a.clone();
        let mut split_b = mesh_b.clone();
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
        for (i, polyline) in graph.polylines.iter().enumerate() {
            eprintln!(
                "  polyline[{i}] closed={} vertices={:?}",
                polyline.closed, polyline.vertices
            );
        }
        let outcome_a = split_mesh_along_graph(&mut split_a, &graph, MeshSide::A, &mut diagnostics);
        let outcome_b = split_mesh_along_graph(&mut split_b, &graph, MeshSide::B, &mut diagnostics);
        eprintln!(
            "split A: {:?}\nsplit B: {:?}",
            outcome_a.stats, outcome_b.stats
        );
        for (i, slot) in outcome_a.graph_vertices.iter().enumerate() {
            eprintln!(
                "  graph vertex {i}: A={:?} B={:?} pos=({:.6},{:.6},{:.6})",
                slot,
                outcome_b.graph_vertices[i],
                graph.vertices[i].position[0],
                graph.vertices[i].position[1],
                graph.vertices[i].position[2],
            );
        }
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
        // Rebuild classify's cut-edge set per side and print every mesh
        // edge that adjoins two faces of the same patch while lying
        // geometrically between differently-classified regions.
        for (label, mesh, outcome) in [("A", &split_a, &outcome_a), ("B", &split_b, &outcome_b)] {
            let mut cut_edges = std::collections::HashSet::new();
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
                    if let (Some(u), Some(v)) =
                        (outcome.graph_vertices[p], outcome.graph_vertices[q])
                    {
                        let (u, v) = if u.index() <= v.index() {
                            (u, v)
                        } else {
                            (v, u)
                        };
                        cut_edges.insert((u.index(), v.index()));
                    }
                }
            }
            let cut_vertex_ids: std::collections::HashSet<u32> = outcome
                .graph_vertices
                .iter()
                .flatten()
                .map(|v| v.index())
                .collect();
            eprintln!("mesh {label}: cut_edges={cut_edges:?}");
            let _ = &cut_vertex_ids;
            // Replicate the classify flood with parent tracking: which edge
            // joined each face to its patch?
            let faces: Vec<_> = mesh.faces().collect();
            let mut assigned = std::collections::HashMap::new();
            for &seed in &faces {
                if assigned.contains_key(&seed) {
                    continue;
                }
                assigned.insert(seed, None);
                let mut stack = vec![seed];
                while let Some(face) = stack.pop() {
                    for half_edge in mesh.face_loop(face) {
                        let (Some(from), Some(to)) =
                            (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
                        else {
                            continue;
                        };
                        let pair = if from.index() <= to.index() {
                            (from.index(), to.index())
                        } else {
                            (to.index(), from.index())
                        };
                        if cut_edges.contains(&pair) {
                            continue;
                        }
                        let Some(neighbor) = mesh.twin(half_edge).and_then(|t| mesh.face(t)) else {
                            continue;
                        };
                        if neighbor == exedra::FaceId::OUTSIDE || assigned.contains_key(&neighbor) {
                            continue;
                        }
                        assigned.insert(neighbor, Some((face, pair)));
                        stack.push(neighbor);
                    }
                }
            }
            // Print the flood parents of the interesting faces on B.
            if label == "B" {
                for face in mesh.faces() {
                    if let Some(Some((parent, pair))) = assigned.get(&face) {
                        eprintln!(
                            "  flood {label}: face {} <- face {} via edge {pair:?}",
                            face.index(),
                            parent.index()
                        );
                    }
                }
            }
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
            // Per-face: centroid nudged inward along face normal is not
            // exact, so instead report the parity of each face's vertex
            // centroid against the other operand (informational only).
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

    /// Stage-isolating reproduction for deep-sweep mesh findings: which fold
    /// stage first disagrees with the referee?
    #[test]
    #[ignore = "triage tool, run by hand"]
    fn isolate_case() {
        let case_seed: u64 = std::env::var("ORACLE_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_259_526_084_160_495_857);
        let point: [f64; 3] = std::env::var("ORACLE_POINT")
            .ok()
            .and_then(|v| {
                let parts: Vec<f64> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                (parts.len() == 3).then(|| [parts[0], parts[1], parts[2]])
            })
            .unwrap_or([0.225_127, 0.149_602, -0.416_627]);

        let mut rng = SplitMix64::new(case_seed);
        let operand_count = 2 + rng.index(3);
        let operands: Vec<Operand> = (0..operand_count)
            .map(|_| random_operand(&mut rng))
            .collect();
        let ops: Vec<BooleanOp> = (1..operand_count)
            .map(|_| {
                [
                    BooleanOp::Union,
                    BooleanOp::Intersection,
                    BooleanOp::Difference,
                ][rng.index(3)]
            })
            .collect();
        eprintln!("ops={ops:?}");
        for (i, operand) in operands.iter().enumerate() {
            let referee = operand.referee(point);
            let parity = point_in_triangles(point, &mesh_triangles_f64(&operand.mesh));
            eprintln!(
                "operand[{i}] {} referee={referee:+.6} parity={parity:?}",
                operand.describe
            );
        }

        let mut scratch = BooleanScratch::default();
        let mut diagnostics = BooleanDiagnostics::new(64);
        let mut mesh = operands[0].mesh.clone();
        let mut referee = operands[0].referee(point);
        for (i, (operand, op)) in operands[1..].iter().zip(&ops).enumerate() {
            diagnostics.clear();
            let output = boolean_mesh(
                &mesh,
                &operand.mesh,
                *op,
                FaceTriangulation::Robust,
                &mut scratch,
                &mut diagnostics,
            )
            .expect("stage boolean should succeed");
            mesh = output.mesh;
            let right = operand.referee(point);
            referee = match op {
                BooleanOp::Union => referee.min(right),
                BooleanOp::Intersection => referee.max(right),
                BooleanOp::Difference => referee.max(-right),
            };
            let parity = point_in_triangles(point, &mesh_triangles_f64(&mesh));
            let deep_errors = mesh.validate_deep().len();
            eprintln!(
                "after stage {i} ({op:?}): referee={referee:+.6} parity={parity:?} faces={} deep_errors={deep_errors} diag_clean={}",
                mesh.faces().count(),
                diagnostics.is_clean(),
            );
            for entry in diagnostics.entries() {
                eprintln!("  diag: {entry:?}");
            }
            // Independent parity cross-check: divergence-theorem volume of
            // the result mesh vs a Monte-Carlo referee volume estimate.
            let triangles = mesh_triangles_f64(&mesh);
            let mut mesh_volume = 0.0_f64;
            for [a, b, c] in &triangles {
                mesh_volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
                    - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0;
            }
            let bounds = case_bounds(&operands);
            let cell = (bounds.max[0] - bounds.min[0])
                * (bounds.max[1] - bounds.min[1])
                * (bounds.max[2] - bounds.min[2]);
            let mut mc = SplitMix64::new(11);
            let samples = 200_000_u64;
            let mut hits = 0_u64;
            for _ in 0..samples {
                let p = [
                    mc.range_f64(bounds.min[0], bounds.max[0]),
                    mc.range_f64(bounds.min[1], bounds.max[1]),
                    mc.range_f64(bounds.min[2], bounds.max[2]),
                ];
                let mut value = operands[0].referee(p);
                for (operand, op) in operands[1..=(i + 1)].iter().zip(&ops) {
                    let r = operand.referee(p);
                    value = match op {
                        BooleanOp::Union => value.min(r),
                        BooleanOp::Intersection => value.max(r),
                        BooleanOp::Difference => value.max(-r),
                    };
                }
                if value < 0.0 {
                    hits += 1;
                }
            }
            #[expect(clippy::cast_precision_loss, reason = "estimate only")]
            let mc_volume = cell * hits as f64 / samples as f64;
            eprintln!("  mesh_volume={mesh_volume:.6} referee_mc_volume={mc_volume:.6}");
            // Scan for disagreeing points at this stage and print a few.
            let mut scan = SplitMix64::new(23);
            let mut printed = 0;
            for _ in 0..40_000 {
                let p = [
                    scan.range_f64(bounds.min[0], bounds.max[0]),
                    scan.range_f64(bounds.min[1], bounds.max[1]),
                    scan.range_f64(bounds.min[2], bounds.max[2]),
                ];
                let mut value = operands[0].referee(p);
                for (operand, op) in operands[1..=(i + 1)].iter().zip(&ops) {
                    let r = operand.referee(p);
                    value = match op {
                        BooleanOp::Union => value.min(r),
                        BooleanOp::Intersection => value.max(r),
                        BooleanOp::Difference => value.max(-r),
                    };
                }
                if value.abs() <= MESH_BAND {
                    continue;
                }
                let parity = point_in_triangles(p, &triangles);
                let inside = parity == Parity::Inside;
                if parity != Parity::Exhausted && inside != (value < 0.0) {
                    eprintln!(
                        "  disagree point=({:.6},{:.6},{:.6}) referee={value:+.6} parity={parity:?}",
                        p[0], p[1], p[2]
                    );
                    for (oi, operand) in operands[..=(i + 1)].iter().enumerate() {
                        let op_parity = point_in_triangles(p, &mesh_triangles_f64(&operand.mesh));
                        eprintln!(
                            "    operand[{oi}] referee={:+.6} parity={op_parity:?}",
                            operand.referee(p)
                        );
                    }
                    printed += 1;
                    if printed >= 5 {
                        break;
                    }
                }
            }
        }
    }
}
