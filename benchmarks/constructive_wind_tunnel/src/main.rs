// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable wind-tunnel scenarios for constructive recipe evaluation.
//!
//! Run the quick profile (the default):
//! `cargo run --release -p constructive_wind_tunnel -- --quick`
//!
//! Run the formal CT-1 stress profile:
//! `cargo run --release -p constructive_wind_tunnel -- --ct1-stress`

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra::ExtractParams;
use exedra_constructive::builders;
use exedra_constructive::evaluate::{Evaluation, evaluate};
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_testkit::trimesh_signature;

fn main() {
    let profile = Profile::from_args(std::env::args().skip(1));
    run_ct1(profile);
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Profile {
    Quick,
    Stress,
}

impl Profile {
    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut profile = Self::Quick;
        for arg in args {
            match arg.as_str() {
                "--quick" => profile = Self::Quick,
                "--ct1-stress" => profile = Self::Stress,
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
        profile
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Stress => "stress",
        }
    }

    /// Number of parameterized recipes per pass.
    const fn recipe_count(self) -> u32 {
        match self {
            Self::Quick => 64,
            Self::Stress => 1024,
        }
    }

    const fn iterations(self) -> u32 {
        match self {
            Self::Quick => 4,
            Self::Stress => 3,
        }
    }
}

fn print_help() {
    eprintln!("usage: constructive_wind_tunnel [--quick | --ct1-stress]");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_profile_contract() {
        // Pin the profile contract like WT-1 does.
        assert_eq!(Profile::Quick.recipe_count(), 64);
        assert_eq!(Profile::Stress.recipe_count(), 1024);
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
