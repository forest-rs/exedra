// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dual-witness cross-validation oracle for mesh booleans and field CSG.
//!
//! Random, seeded CSG expression trees are evaluated three independent
//! ways — the exedra mesh boolean pipeline, the `exedra_isosurface` field
//! combinators, and a closed-form convex half-space referee — and every
//! sampled point's membership is cross-checked. The referee is exact for
//! the polyhedral solid the mesh pipeline consumes, so it attributes each
//! disagreement to the responsible witness instead of guessing.
//!
//! Run the quick profile (the default):
//! `cargo run --release -p boolean_oracle`
//!
//! Deep sweep:
//! `cargo run --release -p boolean_oracle -- --seed 1 --cases 400 --points 2000`

mod cases;
mod membership;
mod operands;
mod rng;

use std::collections::BTreeMap;

use cases::{CaseOutcome, Finding, run_case};

fn main() {
    let config = Config::from_args(std::env::args().skip(1));

    // Determinism oracle before anything else: the same seed must produce
    // byte-identical outcomes.
    let probe = run_case(config.seed, config.points.min(200));
    let reprobe = run_case(config.seed, config.points.min(200));
    assert!(
        probe == reprobe,
        "oracle run is not deterministic for a fixed seed"
    );
    println!("determinism=ok");

    let report = run_batch(config.seed, config.cases, config.points);
    print_report(&report, &config);
}

struct Config {
    seed: u64,
    cases: u64,
    points: u64,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Self {
        let mut config = Self {
            seed: 1,
            cases: 50,
            points: 400,
        };
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| {
                        eprintln!("{name} needs an integer value");
                        std::process::exit(2);
                    })
            };
            match arg.as_str() {
                "--seed" => config.seed = value("--seed"),
                "--cases" => config.cases = value("--cases"),
                "--points" => config.points = value("--points"),
                "--help" | "-h" => {
                    println!("boolean_oracle --seed <u64> --cases <n> --points <per-case>");
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("unknown argument: {arg}");
                    std::process::exit(2);
                }
            }
        }
        config
    }
}

/// Batch aggregate over many cases.
#[derive(Default)]
struct Report {
    cases_run: u64,
    cases_skipped: u64,
    skip_by_reason: BTreeMap<&'static str, u64>,
    mesh_points: u64,
    field_points: u64,
    mesh_band_points: u64,
    field_band_points: u64,
    exhausted_points: u64,
    findings: Vec<Finding>,
    skipped_case_seeds: Vec<(&'static str, u64)>,
}

fn run_batch(seed: u64, case_count: u64, points: u64) -> Report {
    let mut report = Report::default();
    for index in 0..case_count {
        // Independent per-case seeds derived from the batch seed.
        let case_seed = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(index.wrapping_mul(0x2545_F491_4F6C_DD1D))
            | 1;
        let outcome = run_case(case_seed, points);
        absorb(&mut report, case_seed, &outcome);
    }
    report
}

fn absorb(report: &mut Report, case_seed: u64, outcome: &CaseOutcome) {
    if let Some(reason) = outcome.skip {
        report.cases_skipped += 1;
        *report.skip_by_reason.entry(reason.key()).or_insert(0) += 1;
        report.skipped_case_seeds.push((reason.key(), case_seed));
    } else {
        report.cases_run += 1;
    }
    report.mesh_points += outcome.mesh_points;
    report.field_points += outcome.field_points;
    report.mesh_band_points += outcome.mesh_band_points;
    report.field_band_points += outcome.field_band_points;
    report.exhausted_points += outcome.exhausted_points;
    report.findings.extend(outcome.findings.iter().cloned());
}

fn print_report(report: &Report, config: &Config) {
    println!("seed={}", config.seed);
    println!("cases_requested={}", config.cases);
    println!("points_per_case={}", config.points);
    println!("cases_run={}", report.cases_run);
    println!("cases_skipped={}", report.cases_skipped);
    for (reason, count) in &report.skip_by_reason {
        println!("skip.{reason}={count}");
    }
    println!("mesh_points_checked={}", report.mesh_points);
    println!("field_points_checked={}", report.field_points);
    println!("mesh_band_points={}", report.mesh_band_points);
    println!("field_band_points={}", report.field_band_points);
    println!("ray_exhausted_points={}", report.exhausted_points);
    let mesh_findings = report
        .findings
        .iter()
        .filter(|f| f.witness == "mesh")
        .count();
    let field_findings = report
        .findings
        .iter()
        .filter(|f| f.witness == "field")
        .count();
    println!("mesh_disagreements={mesh_findings}");
    println!("field_disagreements={field_findings}");
    let mut by_case: BTreeMap<(&'static str, u64), u64> = BTreeMap::new();
    for finding in &report.findings {
        *by_case
            .entry((finding.witness, finding.case_seed))
            .or_insert(0) += 1;
    }
    for ((witness, case_seed), count) in &by_case {
        println!("finding_case witness={witness} case_seed={case_seed} points={count}");
    }
    for (reason, case_seed) in &report.skipped_case_seeds {
        if *reason == "build_failure" || *reason == "other_suspect" {
            println!("skip_case reason={reason} case_seed={case_seed}");
        }
    }
    for finding in report.findings.iter().take(20) {
        println!(
            "finding witness={} case_seed={} point=({:.6},{:.6},{:.6}) referee_inside={} margin={:.6} witness_value={} case=\"{}\"",
            finding.witness,
            finding.case_seed,
            finding.point[0],
            finding.point[1],
            finding.point[2],
            finding.referee_inside,
            finding.margin,
            finding.witness_value,
            finding.describe,
        );
    }
    if report.findings.len() > 20 {
        println!("findings_elided={}", report.findings.len() - 20);
    }
}

#[cfg(test)]
mod tests {
    use super::run_batch;
    use crate::cases::run_case;

    /// Fast CI subset: a fixed seed batch must produce zero disagreements
    /// outside the documented bands.
    #[test]
    fn fixed_seed_batch_is_clean() {
        let report = run_batch(42, 12, 150);
        assert!(
            report.findings.is_empty(),
            "cross-witness disagreements: {:?}",
            report.findings
        );
        // The batch must actually exercise both witnesses.
        assert!(report.mesh_points > 0, "no mesh points were checked");
        assert!(report.field_points > 0, "no field points were checked");
    }

    /// Same seed, same outcome — end to end.
    #[test]
    fn oracle_is_deterministic() {
        let a = run_case(97, 200);
        let b = run_case(97, 200);
        assert!(a == b);
    }
}
