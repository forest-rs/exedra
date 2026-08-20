// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dual-witness cross-validation oracle for mesh booleans and field CSG.
//!
//! Seeded scenario classes (see [`scenario`]) are evaluated three
//! independent ways — the exedra mesh boolean pipeline, the
//! `exedra_isosurface` field combinators, and a closed-form union-of-convex
//! half-space referee — and every sampled point's membership is
//! cross-checked. The referee is exact for the polyhedral solid the mesh
//! pipeline consumes, so it attributes each disagreement to the responsible
//! witness instead of guessing.
//!
//! A fixed typed suite also validates opt-in semi-analytic box/cylinder
//! extraction across CSG operators and coordinate scales. Pass
//! `--feature-obj` to emit its region-grouped reference mesh under
//! `target/boolean_oracle`.
//!
//! Run the quick profile (the default, all classes):
//! `cargo run --release -p boolean_oracle`
//!
//! Deep sweep of one class:
//! `cargo run --release -p boolean_oracle -- --seed 1 --cases 200 --points 2000 --class curved_wall`

mod cases;
mod feature;
mod membership;
mod operands;
mod rng;
mod scenario;

use std::collections::BTreeMap;

use cases::{CaseOutcome, Finding, run_case};
use scenario::ScenarioClass;

fn main() {
    let config = Config::from_args(std::env::args().skip(1));

    let feature_reports = feature::run_suite();
    feature::assert_suite(&feature_reports);
    feature::print_suite(&feature_reports);
    if config.feature_obj {
        let path = feature::write_reference_obj().unwrap_or_else(|error| {
            eprintln!("failed to write semi-analytic OBJ: {error}");
            std::process::exit(1);
        });
        println!("semi_analytic.obj={}", path.display());
    }

    // Determinism oracle before anything else: the same seed must produce
    // byte-identical outcomes, per class.
    for &class in &config.classes {
        let probe = run_case(class, config.seed, config.points.min(120));
        let reprobe = run_case(class, config.seed, config.points.min(120));
        assert!(
            probe == reprobe,
            "oracle run is not deterministic for class {} seed {}",
            class.key(),
            config.seed
        );
    }
    println!("determinism=ok");

    let mut reports = BTreeMap::new();
    for &class in &config.classes {
        reports.insert(
            class,
            run_batch(class, config.seed, config.cases, config.points),
        );
    }
    print_report(&reports, &config);
}

struct Config {
    seed: u64,
    cases: u64,
    points: u64,
    classes: Vec<ScenarioClass>,
    feature_obj: bool,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Self {
        let mut config = Self {
            seed: 1,
            cases: 50,
            points: 400,
            classes: ScenarioClass::ALL.to_vec(),
            feature_obj: false,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--seed" | "--cases" | "--points" => {
                    let value = args
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or_else(|| {
                            eprintln!("{arg} needs an integer value");
                            std::process::exit(2);
                        });
                    match arg.as_str() {
                        "--seed" => config.seed = value,
                        "--cases" => config.cases = value,
                        _ => config.points = value,
                    }
                }
                "--class" => {
                    let key = args.next().unwrap_or_default();
                    let Some(class) = ScenarioClass::parse(&key) else {
                        eprintln!(
                            "unknown class {key}; known: {}",
                            ScenarioClass::ALL.map(|c| c.key()).join(",")
                        );
                        std::process::exit(2);
                    };
                    config.classes = vec![class];
                }
                "--feature-obj" => config.feature_obj = true,
                "--help" | "-h" => {
                    println!(
                        "boolean_oracle --seed <u64> --cases <n> --points <per-case> [--class <key>] [--feature-obj]"
                    );
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

/// Batch aggregate over many cases of one class.
#[derive(Default)]
struct Report {
    cases_run: u64,
    cases_skipped: u64,
    empty_results: u64,
    skip_by_reason: BTreeMap<&'static str, u64>,
    submode_run: BTreeMap<&'static str, u64>,
    submode_skip: BTreeMap<&'static str, u64>,
    mesh_points: u64,
    field_points: u64,
    mesh_band_points: u64,
    field_band_points: u64,
    exhausted_points: u64,
    findings: Vec<Finding>,
    skipped_case_seeds: Vec<(&'static str, u64)>,
}

fn case_seed_for(class: ScenarioClass, seed: u64, index: u64) -> u64 {
    let class_bit = ScenarioClass::ALL
        .iter()
        .position(|c| *c == class)
        .unwrap_or(0) as u64;
    seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(index.wrapping_mul(0x2545_F491_4F6C_DD1D))
        .wrapping_add(class_bit.wrapping_mul(0xD6E8_FEB8_6659_FD93))
        | 1
}

fn run_batch(class: ScenarioClass, seed: u64, case_count: u64, points: u64) -> Report {
    let mut report = Report::default();
    for index in 0..case_count {
        let case_seed = case_seed_for(class, seed, index);
        let outcome = run_case(class, case_seed, points);
        absorb(&mut report, case_seed, &outcome);
    }
    report
}

fn absorb(report: &mut Report, case_seed: u64, outcome: &CaseOutcome) {
    if let Some(reason) = outcome.skip {
        report.cases_skipped += 1;
        *report.skip_by_reason.entry(reason.key()).or_insert(0) += 1;
        *report.submode_skip.entry(outcome.submode).or_insert(0) += 1;
        report.skipped_case_seeds.push((reason.key(), case_seed));
    } else {
        report.cases_run += 1;
        *report.submode_run.entry(outcome.submode).or_insert(0) += 1;
        if outcome.empty_result {
            report.empty_results += 1;
        }
    }
    report.mesh_points += outcome.mesh_points;
    report.field_points += outcome.field_points;
    report.mesh_band_points += outcome.mesh_band_points;
    report.field_band_points += outcome.field_band_points;
    report.exhausted_points += outcome.exhausted_points;
    report.findings.extend(outcome.findings.iter().cloned());
}

fn print_report(reports: &BTreeMap<ScenarioClass, Report>, config: &Config) {
    println!("seed={}", config.seed);
    println!("cases_requested_per_class={}", config.cases);
    println!("points_per_case={}", config.points);

    let mut total_mesh_findings = 0_usize;
    let mut total_field_findings = 0_usize;
    for (class, report) in reports {
        let key = class.key();
        println!("class.{key}.cases_run={}", report.cases_run);
        println!("class.{key}.cases_skipped={}", report.cases_skipped);
        println!("class.{key}.empty_results={}", report.empty_results);
        for (reason, count) in &report.skip_by_reason {
            println!("class.{key}.skip.{reason}={count}");
        }
        for (submode, count) in &report.submode_run {
            if *submode != "-" {
                println!("class.{key}.submode.{submode}.run={count}");
            }
        }
        for (submode, count) in &report.submode_skip {
            if *submode != "-" {
                println!("class.{key}.submode.{submode}.skip={count}");
            }
        }
        println!("class.{key}.mesh_points_checked={}", report.mesh_points);
        println!("class.{key}.field_points_checked={}", report.field_points);
        println!("class.{key}.mesh_band_points={}", report.mesh_band_points);
        println!("class.{key}.field_band_points={}", report.field_band_points);
        println!(
            "class.{key}.ray_exhausted_points={}",
            report.exhausted_points
        );
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
        println!("class.{key}.mesh_disagreements={mesh_findings}");
        println!("class.{key}.field_disagreements={field_findings}");
        total_mesh_findings += mesh_findings;
        total_field_findings += field_findings;

        let mut by_case: BTreeMap<(&'static str, u64), u64> = BTreeMap::new();
        for finding in &report.findings {
            *by_case
                .entry((finding.witness, finding.case_seed))
                .or_insert(0) += 1;
        }
        for ((witness, case_seed), count) in &by_case {
            println!(
                "finding_case class={key} witness={witness} case_seed={case_seed} points={count}"
            );
        }
        for (reason, case_seed) in &report.skipped_case_seeds {
            if *reason == "build_failure"
                || *reason == "other_suspect"
                || *reason == "invariant_violation"
            {
                println!("skip_case class={key} reason={reason} case_seed={case_seed}");
            }
        }
        for finding in report.findings.iter().take(10) {
            println!(
                "finding class={key} witness={} case_seed={} point=({:.6},{:.6},{:.6}) referee_inside={} margin={:.6} witness_value={} case=\"{}\"",
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
        if report.findings.len() > 10 {
            println!(
                "findings_elided class={key} count={}",
                report.findings.len() - 10
            );
        }
    }
    println!("mesh_disagreements={total_mesh_findings}");
    println!("field_disagreements={total_field_findings}");
}

#[cfg(test)]
mod tests {
    use super::{case_seed_for, run_batch};
    use crate::cases::run_case;
    use crate::scenario::ScenarioClass;

    /// Fast CI subset: every class runs a small fixed-seed batch with zero
    /// disagreements outside the documented bands, and both witnesses are
    /// actually exercised.
    #[test]
    fn fixed_seed_batches_are_clean_across_classes() {
        for class in ScenarioClass::ALL {
            let report = run_batch(class, 42, 8, 120);
            assert!(
                report.findings.is_empty(),
                "class {}: cross-witness disagreements: {:?}",
                class.key(),
                report.findings
            );
            assert!(
                report.field_points > 0,
                "class {}: no field points were checked",
                class.key()
            );
            // Mesh points may legitimately be zero for a class whose entire
            // small batch typed-defers; require coverage OR counted skips.
            assert!(
                report.mesh_points > 0 || report.cases_skipped > 0,
                "class {}: mesh witness neither ran nor skipped",
                class.key()
            );
        }
    }

    /// Same seed, same outcome — end to end, per class.
    #[test]
    fn oracle_is_deterministic_across_classes() {
        for class in ScenarioClass::ALL {
            let seed = case_seed_for(class, 97, 3);
            let a = run_case(class, seed, 150);
            let b = run_case(class, seed, 150);
            assert!(a == b, "class {} diverged between reruns", class.key());
        }
    }
}
