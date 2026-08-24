// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable wind-tunnel scenarios for constructive recipe evaluation:
//! CT-1 (evaluation + source-map lookups at scale), CT-2 (incremental
//! regeneration: a one-parameter edit re-tessellates exactly one body,
//! bit-identical to a full rebuild), and CT-3 (the gallery's direct
//! Boolean-plus-rounding card, timed by phase).
//!
//! Run the quick profile (the default):
//! `cargo run --release -p constructive_wind_tunnel -- --quick`
//!
//! Run the formal stress profile:
//! `cargo run --release -p constructive_wind_tunnel -- --ct1-stress`
//!
//! Isolate the gallery fixture for profiling:
//! `cargo run --release -p constructive_wind_tunnel -- --gallery-stress`
//!
//! Keep the gallery fixture alive for an external sampling profiler:
//! `cargo run --release -p constructive_wind_tunnel -- --gallery-sample`

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra::boolean::{BooleanDiagnostics, BooleanOp, BooleanScratch, BooleanStats, boolean_mesh};
use exedra::round::{RoundPolicy, RoundStats, round_sharp_edges};
use exedra::{ExtractParams, FaceTriangulation, Mesh, MeshBuilder};
use exedra_constructive::builders;
use exedra_constructive::cache::EvalCache;
use exedra_constructive::evaluate::{Evaluation, evaluate, evaluate_with_cache};
use exedra_constructive::ir::{CapMode, CsgOp, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_testkit::trimesh_signature;

fn main() {
    let config = Config::from_args(std::env::args().skip(1));
    if config.gallery_only {
        run_ct3(config.profile);
    } else {
        run_ct1(config.profile);
        run_ct2(config.profile);
        run_ct3(config.profile);
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Config {
    profile: Profile,
    gallery_only: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Profile {
    Quick,
    Stress,
    Sample,
}

impl Profile {
    const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Stress => "stress",
            Self::Sample => "sample",
        }
    }

    /// Number of parameterized recipes per pass.
    const fn recipe_count(self) -> u32 {
        match self {
            Self::Quick => 64,
            Self::Stress | Self::Sample => 1024,
        }
    }

    const fn iterations(self) -> u32 {
        match self {
            Self::Quick => 4,
            Self::Stress | Self::Sample => 3,
        }
    }

    /// Body-node count for the CT-2 incremental scenario.
    const fn ct2_nodes(self) -> u32 {
        match self {
            Self::Quick => 100,
            Self::Stress | Self::Sample => 400,
        }
    }

    /// Iterations for the gallery-shaped Boolean and rounding phases. The
    /// stress count keeps each phase alive long enough for `sample` while the
    /// quick count remains suitable for local before/after measurements.
    const fn gallery_iterations(self) -> u32 {
        match self {
            Self::Quick => 8,
            Self::Stress => 32,
            // About one minute on the gallery fixture: long enough for an
            // external profiler to attach through a separate process and
            // collect steady-state stacks without changing the workload.
            Self::Sample => 4096,
        }
    }
}

impl Config {
    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut config = Self {
            profile: Profile::Quick,
            gallery_only: false,
        };
        for arg in args {
            match arg.as_str() {
                "--quick" => config.profile = Profile::Quick,
                "--ct1-stress" => config.profile = Profile::Stress,
                "--gallery" => config.gallery_only = true,
                "--gallery-stress" => {
                    config.profile = Profile::Stress;
                    config.gallery_only = true;
                }
                "--gallery-sample" => {
                    config.profile = Profile::Sample;
                    config.gallery_only = true;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("unknown argument: {arg}");
                    print_help();
                    std::process::exit(2);
                }
            }
        }
        config
    }
}

fn print_help() {
    eprintln!(
        "usage: constructive_wind_tunnel [--quick | --ct1-stress | --gallery | --gallery-stress | --gallery-sample]"
    );
}

/// Builds the `index`-th parameterized recipe: a rounded-profile extrusion
/// whose dimensions derive from the index (integer-derived, deterministic).
fn build_recipe(index: u32) -> Recipe {
    let width = 200.0 + f64::from(index % 17) * 10.0;
    let height = 120.0 + f64::from(index % 11) * 8.0;
    let radius = 8.0 + f64::from(index % 5) * 3.0;
    let depth = 400.0 + f64::from(index % 7) * 25.0;

    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(
        builders::rounded_rect(width, height, radius).expect("wind-tunnel dimensions are valid"),
    );
    let node = b
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::translate(f64::from(index) * 300.0, 0.0, 0.0),
            height: depth,
            caps: CapMode::Both,
        })
        .expect("wind-tunnel extrude is valid");
    b.finish(node).expect("wind-tunnel recipe is valid")
}

/// One full pass: evaluate every recipe, fold signatures and counters.
fn run_pass(recipes: &[Recipe], policy: &EvalPolicy) -> PassResult {
    let mut signature = 0_u64;
    let mut faces = 0_u64;
    let mut map_bytes = 0_u64;
    let mut lookups = 0_u64;
    for recipe in recipes {
        let result: Evaluation = evaluate(recipe, policy).expect("wind-tunnel recipes evaluate");
        for placed in &result.bodies {
            let (tri, _) = placed.body.mesh.to_trimesh(&ExtractParams::default());
            let rotation = u32::try_from(faces % 63).expect("modulo 63 fits");
            signature ^= trimesh_signature(&tri).rotate_left(rotation);
            faces += placed.body.mesh.faces().count() as u64;
            // Exercise source-map lookups at scale: every face resolves
            // forward, and its feature resolves back to a face set that
            // contains it.
            for face in placed.body.mesh.faces() {
                let feature = placed
                    .body
                    .source_map
                    .face_feature(face)
                    .expect("every face is mapped");
                let reverse = placed.body.source_map.faces_for(feature);
                assert!(
                    reverse.iter().any(|&(_, i)| i == face.index()),
                    "reverse lookup must contain the face"
                );
                lookups += 2;
            }
        }
        map_bytes += result.report.counters.source_map_bytes;
    }
    PassResult {
        signature,
        faces,
        map_bytes,
        lookups,
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct PassResult {
    signature: u64,
    faces: u64,
    map_bytes: u64,
    lookups: u64,
}

fn run_ct1(profile: Profile) {
    let policy = EvalPolicy::default();
    let build_start = Instant::now();
    let recipes: Vec<Recipe> = (0..profile.recipe_count()).map(build_recipe).collect();
    let build_time = build_start.elapsed();

    // Determinism oracle before any timing: two full passes must agree bit
    // for bit (signatures fold every mesh).
    let first = run_pass(&recipes, &policy);
    let second = run_pass(&recipes, &policy);
    assert_eq!(first, second, "CT-1 determinism violated: passes disagree");

    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..profile.iterations() {
        let start = Instant::now();
        black_box(run_pass(&recipes, &policy));
        let elapsed = start.elapsed();
        best = best.min(elapsed);
        total += elapsed;
    }
    let avg = total / profile.iterations();

    println!(
        "scenario=CT-1 profile={} recipes={} faces={} lookups={} map_bytes={} \
         build_ns={} best_ns={} avg_ns={} signature={:016x}",
        profile.label(),
        profile.recipe_count(),
        first.faces,
        first.lookups,
        first.map_bytes,
        build_time.as_nanos(),
        best.as_nanos(),
        avg.as_nanos(),
        first.signature,
    );
}

/// Builds one recipe with `nodes` extrude bodies under a single group;
/// `edited` overrides the height of node `edit_index` when set.
fn build_ct2_recipe(nodes: u32, edit_index: u32, edited: Option<f64>) -> Recipe {
    let mut b = RecipeBuilder::new();
    let mut children = Vec::new();
    for i in 0..nodes {
        let width = 200.0 + f64::from(i % 17) * 10.0;
        let height = 120.0 + f64::from(i % 11) * 8.0;
        let radius = 8.0 + f64::from(i % 5) * 3.0;
        let mut depth = 400.0 + f64::from(i % 7) * 25.0;
        if i == edit_index
            && let Some(d) = edited
        {
            depth = d;
        }
        let profile = b.add_profile(
            builders::rounded_rect(width, height, radius)
                .expect("wind-tunnel dimensions are valid"),
        );
        let node = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::translate(f64::from(i) * 300.0, 0.0, 0.0),
                height: depth,
                caps: CapMode::Both,
            })
            .expect("wind-tunnel extrude is valid");
        children.push(node);
    }
    let root = b
        .add(NodeKind::Group { children })
        .expect("wind-tunnel group is valid");
    b.finish(root).expect("wind-tunnel recipe is valid")
}

/// Folds every body's trimesh signature (extraction outside any timing).
fn fold_signature(evaluation: &Evaluation) -> u64 {
    let mut signature = 0_u64;
    for (index, placed) in evaluation.bodies.iter().enumerate() {
        let (tri, _) = placed.body.mesh.to_trimesh(&ExtractParams::default());
        let rotation = u32::try_from(index % 63).expect("modulo 63 fits");
        signature ^= trimesh_signature(&tri).rotate_left(rotation);
    }
    signature
}

/// CT-2: one-parameter edit on an N-body recipe re-tessellates exactly the
/// edited body, bit-identical to a full rebuild, and reports the speedup.
fn run_ct2(profile: Profile) {
    let policy = EvalPolicy::default();
    let nodes = profile.ct2_nodes();
    let edit_index = nodes / 2;
    let base = build_ct2_recipe(nodes, edit_index, None);
    let edited = build_ct2_recipe(nodes, edit_index, Some(555.0));

    // Determinism oracle before any timing: the warm (cached) evaluation
    // of the edited recipe must equal a cold full rebuild bit for bit,
    // twice over.
    let full = evaluate(&edited, &policy).expect("wind-tunnel recipes evaluate");
    let full_signature = fold_signature(&full);
    for round in 0..2 {
        let mut cache = EvalCache::with_capacity(4096);
        let cold =
            evaluate_with_cache(&base, &policy, &mut cache).expect("wind-tunnel recipes evaluate");
        assert_eq!(
            u64::from(cold.report.counters.tessellations),
            u64::from(nodes),
            "cold pass tessellates every body"
        );
        let warm = evaluate_with_cache(&edited, &policy, &mut cache)
            .expect("wind-tunnel recipes evaluate");
        assert_eq!(
            warm.report.counters.cache_misses, 1,
            "round {round}: exactly the edited node misses"
        );
        assert_eq!(
            warm.report.counters.tessellations, 1,
            "round {round}: exactly the edited node re-tessellates"
        );
        assert_eq!(
            u64::from(warm.report.counters.cache_hits),
            u64::from(nodes - 1),
            "round {round}: every other body reuses"
        );
        assert_eq!(
            fold_signature(&warm),
            full_signature,
            "round {round}: incremental output equals the full rebuild"
        );
    }

    // Timing: full rebuild of the edited recipe vs cached re-evaluation
    // with the pre-edit cache (re-primed untimed each iteration, since the
    // warm pass itself inserts the edited body).
    let mut cold_best = Duration::MAX;
    let mut warm_best = Duration::MAX;
    for _ in 0..profile.iterations() {
        let start = Instant::now();
        black_box(evaluate(&edited, &policy).expect("wind-tunnel recipes evaluate"));
        cold_best = cold_best.min(start.elapsed());

        let mut cache = EvalCache::with_capacity(4096);
        black_box(
            evaluate_with_cache(&base, &policy, &mut cache).expect("wind-tunnel recipes evaluate"),
        );
        let start = Instant::now();
        black_box(
            evaluate_with_cache(&edited, &policy, &mut cache)
                .expect("wind-tunnel recipes evaluate"),
        );
        warm_best = warm_best.min(start.elapsed());
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "nanosecond ratios for reporting only"
    )]
    let speedup = cold_best.as_nanos() as f64 / warm_best.as_nanos().max(1) as f64;

    println!(
        "scenario=CT-2 profile={} nodes={} edit_index={} cold_ns={} warm_ns={} \
         speedup={speedup:.1} warm_misses=1 signature={full_signature:016x}",
        profile.label(),
        nodes,
        edit_index,
        cold_best.as_nanos(),
        warm_best.as_nanos(),
    );
}

/// CT-3 operands mirror the gallery's rounded-drill card: a 4 × 4 × 1 slab
/// minus a 16-gon prism which passes through both caps. Keeping this fixture in
/// the benchmark crate lets profiling isolate the kernel phases without OBJ
/// formatting, filesystem I/O, or the gallery's other scenarios.
fn build_gallery_drill_operands() -> (Mesh, Mesh) {
    let mut slab = MeshBuilder::new();
    for position in [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [4.0, 4.0, 0.0],
        [0.0, 4.0, 0.0],
        [0.0, 0.0, 1.0],
        [4.0, 0.0, 1.0],
        [4.0, 4.0, 1.0],
        [0.0, 4.0, 1.0],
    ] {
        slab.push_vertex(position);
    }
    for face in [
        [3_u32, 2, 1, 0],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ] {
        slab.add_face(&face).expect("slab face is valid");
    }

    let sides = 16_u32;
    let mut drill = MeshBuilder::new();
    for z in [-1.0_f64, 2.0] {
        for side in 0..sides {
            let angle = core::f64::consts::TAU * f64::from(side) / f64::from(sides);
            let point = [2.0 + 0.8 * angle.cos(), 2.0 + 0.8 * angle.sin(), z];
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the gallery fixture stores finite unit-scale f32 positions"
            )]
            drill.push_vertex([point[0] as f32, point[1] as f32, point[2] as f32]);
        }
    }
    let bottom: Vec<u32> = (0..sides).rev().collect();
    drill.add_face(&bottom).expect("drill bottom cap is valid");
    let top: Vec<u32> = (sides..2 * sides).collect();
    drill.add_face(&top).expect("drill top cap is valid");
    for side in 0..sides {
        let next = (side + 1) % sides;
        drill
            .add_face(&[side, next, sides + next, sides + side])
            .expect("drill wall is valid");
    }

    (
        slab.build().expect("slab is manifold").mesh,
        drill.build().expect("drill is manifold").mesh,
    )
}

/// The gallery's public constructive CSG card. Unlike the direct 16-sided
/// rounding fixture, this uses the default circle discretization and therefore
/// exercises the higher-resolution face-splitting path seen in the gallery
/// process sample.
fn build_gallery_csg_recipe() -> Recipe {
    let mut builder = RecipeBuilder::new();
    let block = builder.add_profile(builders::rect(200.0, 100.0).expect("valid block profile"));
    let drill = builder.add_profile(builders::circle(30.0).expect("valid drill profile"));
    let block = builder
        .add(NodeKind::Extrude {
            profile: block,
            placement: Placement3::IDENTITY,
            height: 80.0,
            caps: CapMode::Both,
        })
        .expect("valid block extrusion");
    let drill = builder
        .add(NodeKind::Extrude {
            profile: drill,
            placement: Placement3::translate(130.0, 50.0, -20.0),
            height: 120.0,
            caps: CapMode::Both,
        })
        .expect("valid drill extrusion");
    let difference = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![block, drill],
        })
        .expect("valid difference");
    let moved = builder
        .add(NodeKind::Transform {
            child: difference,
            xf: Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_4, 50.0, 0.0, 0.0),
        })
        .expect("valid transform");
    builder.finish(moved).expect("valid gallery CSG recipe")
}

fn gallery_boolean(slab: &Mesh, drill: &Mesh) -> (Mesh, BooleanStats) {
    let mut scratch = BooleanScratch::new();
    let mut diagnostics = BooleanDiagnostics::default();
    let output = boolean_mesh(
        slab,
        drill,
        BooleanOp::Difference,
        FaceTriangulation::Fan,
        &mut scratch,
        &mut diagnostics,
    )
    .expect("gallery drill Boolean succeeds");
    assert!(
        diagnostics.is_clean(),
        "gallery drill diagnostics: {:?}",
        diagnostics.entries()
    );
    (output.mesh, output.stats)
}

fn gallery_round(drilled: &Mesh) -> (Mesh, RoundStats) {
    let mut rounded = drilled.clone();
    let mut policy = RoundPolicy::fillet(0.3);
    policy.region = Some(9);
    let stats = round_sharp_edges(&mut rounded, &policy).expect("gallery drill rounds");
    (rounded, stats)
}

/// Times a complete phase, including destruction of its returned allocation.
/// Best and average are both reported: best is useful for before/after local
/// comparisons, while a large best/average gap exposes scheduler noise.
fn time_phase(iterations: u32, mut phase: impl FnMut()) -> (Duration, Duration) {
    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        phase();
        let elapsed = start.elapsed();
        best = best.min(elapsed);
        total += elapsed;
    }
    (best, total / iterations)
}

/// CT-3: the gallery's direct Boolean-plus-rounding card, with Boolean,
/// rounding, and render extraction measured independently. Determinism and
/// deep validity are established before timing so performance work cannot
/// trade away topology, attributes, or orientation.
fn run_ct3(profile: Profile) {
    let policy = EvalPolicy::default();
    let recipe = build_gallery_csg_recipe();
    let evaluated_a = evaluate(&recipe, &policy).expect("gallery CSG evaluates");
    let evaluated_b = evaluate(&recipe, &policy).expect("gallery CSG evaluates");
    assert_eq!(
        fold_signature(&evaluated_a),
        fold_signature(&evaluated_b),
        "constructive CSG is deterministic"
    );
    assert!(
        evaluated_a
            .bodies
            .iter()
            .all(|body| body.body.mesh.validate_deep().is_empty()),
        "constructive CSG bodies are valid"
    );
    let constructive_signature = fold_signature(&evaluated_a);
    let constructive_faces: usize = evaluated_a
        .bodies
        .iter()
        .map(|body| body.body.mesh.faces().count())
        .sum();

    let (slab, drill) = build_gallery_drill_operands();
    let (drilled_a, boolean_stats_a) = gallery_boolean(&slab, &drill);
    let (drilled_b, boolean_stats_b) = gallery_boolean(&slab, &drill);
    assert_eq!(boolean_stats_a, boolean_stats_b, "Boolean stats are stable");
    assert!(
        drilled_a.validate_deep().is_empty(),
        "drilled mesh is valid"
    );
    let drilled_signature = fold_mesh_signature(&drilled_a);
    assert_eq!(
        drilled_signature,
        fold_mesh_signature(&drilled_b),
        "drilled mesh is deterministic"
    );

    let (rounded_a, round_stats_a) = gallery_round(&drilled_a);
    let (rounded_b, round_stats_b) = gallery_round(&drilled_a);
    assert_eq!(round_stats_a, round_stats_b, "rounding stats are stable");
    assert!(
        rounded_a.validate_deep().is_empty(),
        "rounded mesh is valid"
    );
    let rounded_signature = fold_mesh_signature(&rounded_a);
    assert_eq!(
        rounded_signature,
        fold_mesh_signature(&rounded_b),
        "rounded mesh is deterministic"
    );

    let iterations = profile.gallery_iterations();
    let (constructive_best, constructive_avg) = time_phase(iterations, || {
        black_box(evaluate(black_box(&recipe), black_box(&policy)).expect("gallery CSG evaluates"));
    });
    let (boolean_best, boolean_avg) = time_phase(iterations, || {
        black_box(gallery_boolean(black_box(&slab), black_box(&drill)));
    });
    let (round_best, round_avg) = time_phase(iterations, || {
        black_box(gallery_round(black_box(&drilled_a)));
    });
    let params = ExtractParams::default();
    let (extract_best, extract_avg) = time_phase(iterations, || {
        black_box(rounded_a.to_trimesh(black_box(&params)));
    });

    println!(
        "scenario=CT-3 profile={} iterations={} constructive_best_ns={} constructive_avg_ns={} \
         boolean_best_ns={} boolean_avg_ns={} \
         round_best_ns={} round_avg_ns={} extract_best_ns={} extract_avg_ns={} \
         constructive_faces={} drilled_faces={} rounded_faces={} segments={} seam_edges={} \
         strip_faces={} constructive_signature={constructive_signature:016x} \
         drilled_signature={drilled_signature:016x} rounded_signature={rounded_signature:016x}",
        profile.label(),
        iterations,
        constructive_best.as_nanos(),
        constructive_avg.as_nanos(),
        boolean_best.as_nanos(),
        boolean_avg.as_nanos(),
        round_best.as_nanos(),
        round_avg.as_nanos(),
        extract_best.as_nanos(),
        extract_avg.as_nanos(),
        constructive_faces,
        drilled_a.faces().count(),
        rounded_a.faces().count(),
        boolean_stats_a.segments,
        boolean_stats_a.seam_edges,
        round_stats_a.strip_faces,
    );
}

fn fold_mesh_signature(mesh: &Mesh) -> u64 {
    let (triangles, _) = mesh.to_trimesh(&ExtractParams::default());
    trimesh_signature(&triangles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct3_gallery_contract_is_deterministic_and_valid() {
        // The profiling fixtures must remain the gallery's two through-drill
        // paths: public constructive evaluation plus the direct Boolean mesh
        // used for rounding. Both must be deterministic and deeply valid.
        let recipe = build_gallery_csg_recipe();
        let policy = EvalPolicy::default();
        let evaluated_a = evaluate(&recipe, &policy).expect("evaluates");
        let evaluated_b = evaluate(&recipe, &policy).expect("evaluates");
        assert_eq!(fold_signature(&evaluated_a), fold_signature(&evaluated_b));
        assert!(
            evaluated_a
                .bodies
                .iter()
                .all(|body| body.body.mesh.validate_deep().is_empty())
        );

        let (slab, drill) = build_gallery_drill_operands();
        let (drilled_a, boolean_a) = gallery_boolean(&slab, &drill);
        let (drilled_b, boolean_b) = gallery_boolean(&slab, &drill);
        assert_eq!(boolean_a, boolean_b);
        assert!(boolean_a.segments > 0);
        assert!(boolean_a.seam_edges > 0);
        assert!(drilled_a.validate_deep().is_empty());
        assert_eq!(
            fold_mesh_signature(&drilled_a),
            fold_mesh_signature(&drilled_b)
        );

        let (rounded_a, round_a) = gallery_round(&drilled_a);
        let (rounded_b, round_b) = gallery_round(&drilled_a);
        assert_eq!(round_a, round_b);
        assert!(round_a.strip_faces > 0);
        assert!(rounded_a.validate_deep().is_empty());
        assert_eq!(
            fold_mesh_signature(&rounded_a),
            fold_mesh_signature(&rounded_b)
        );
    }

    #[test]
    fn ct2_incremental_contract() {
        // CI-sized CT-2: the assertions, not the timing.
        assert_eq!(Profile::Quick.ct2_nodes(), 100);
        assert_eq!(Profile::Stress.ct2_nodes(), 400);
        let policy = EvalPolicy::default();
        let nodes = 12_u32;
        let base = build_ct2_recipe(nodes, 6, None);
        let edited = build_ct2_recipe(nodes, 6, Some(555.0));
        let mut cache = EvalCache::new();
        let cold = evaluate_with_cache(&base, &policy, &mut cache).expect("evaluates");
        assert_eq!(
            u64::from(cold.report.counters.tessellations),
            u64::from(nodes)
        );
        let warm = evaluate_with_cache(&edited, &policy, &mut cache).expect("evaluates");
        assert_eq!(warm.report.counters.cache_misses, 1);
        assert_eq!(warm.report.counters.tessellations, 1);
        let full = evaluate(&edited, &policy).expect("evaluates");
        assert_eq!(
            fold_signature(&warm),
            fold_signature(&full),
            "incremental equals full rebuild"
        );
    }

    #[test]
    fn quick_profile_contract() {
        // Pin the profile contract like WT-1 does.
        assert_eq!(Profile::Quick.recipe_count(), 64);
        assert_eq!(Profile::Stress.recipe_count(), 1024);
        assert_eq!(Profile::Sample.gallery_iterations(), 4096);
        let recipes: Vec<Recipe> = (0..4).map(build_recipe).collect();
        let policy = EvalPolicy::default();
        let a = run_pass(&recipes, &policy);
        let b = run_pass(&recipes, &policy);
        assert_eq!(a, b, "pass determinism");
        // Every recipe evaluates to valid geometry.
        for recipe in &recipes {
            let result = evaluate(recipe, &policy).expect("evaluates");
            for placed in &result.bodies {
                assert!(placed.body.mesh.validate_deep().is_empty());
            }
        }
    }
}
