// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic quality and timing wind tunnel for `exedra_triangulate`.

mod fixtures;
mod metrics;
mod svg;
#[cfg(test)]
mod tests;
mod timing;

use exedra_triangulate::{TriParams, TriStrategy};
use fixtures::{fixtures, incircle_scenarios, predicate_scenarios};
use metrics::{QualityReport, analyze, analyze_refined};
use svg::write_svg_gallery;
use timing::{time_fixture, time_incircle_path, time_predicate_path, time_refined};

pub(crate) const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
pub(crate) const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn main() {
    let options = Options::from_args(std::env::args().skip(1));
    let profile = options.profile;
    let fixtures = fixtures();
    if let Some(directory) = &options.svg_directory {
        write_svg_gallery(directory, &fixtures);
    }
    for strategy in [TriStrategy::EarClip, TriStrategy::ConstrainedDelaunay] {
        let reports: Vec<QualityReport> = fixtures
            .iter()
            .map(|fixture| analyze(fixture, strategy))
            .collect();

        println!(
            "phase=quality profile={} strategy={} fixtures={}",
            profile.label(),
            strategy_label(strategy),
            fixtures.len()
        );
        for report in &reports {
            report.print();
        }

        println!(
            "phase=timing profile={} strategy={} fixtures={}",
            profile.label(),
            strategy_label(strategy),
            fixtures.len()
        );
        for fixture in &fixtures {
            time_fixture(fixture, profile, strategy).print();
        }
    }

    println!(
        "phase=refine_quality profile={} fixtures={}",
        profile.label(),
        fixtures.len()
    );
    for fixture in &fixtures {
        analyze_refined(fixture).print();
    }
    println!(
        "phase=refine_timing profile={} fixtures={}",
        profile.label(),
        fixtures.len()
    );
    for fixture in &fixtures {
        time_refined(fixture, profile).print();
    }

    let predicate_scenarios = predicate_scenarios();
    let incircle_scenarios = incircle_scenarios();
    println!(
        "phase=predicate_timing profile={} scenarios={}",
        profile.label(),
        predicate_scenarios.len() + incircle_scenarios.len()
    );
    for scenario in predicate_scenarios {
        time_predicate_path(scenario, profile).print();
    }
    for scenario in incircle_scenarios {
        time_incircle_path(scenario, profile).print();
    }
}

/// Command-line options: the timing profile and an optional SVG gallery.
struct Options {
    profile: Profile,
    svg_directory: Option<std::path::PathBuf>,
}

impl Options {
    fn from_args(mut args: impl Iterator<Item = String>) -> Self {
        let mut options = Self {
            profile: Profile::Quick,
            svg_directory: None,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--quick" => options.profile = Profile::Quick,
                "--stress" => options.profile = Profile::Stress,
                "--svg" => match args.next() {
                    Some(directory) => options.svg_directory = Some(directory.into()),
                    None => {
                        eprintln!("--svg requires a directory");
                        print_help();
                        std::process::exit(2);
                    }
                },
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
        options
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Profile {
    Quick,
    Stress,
}

impl Profile {
    const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Stress => "stress",
        }
    }

    const fn target_vertices(self) -> usize {
        match self {
            Self::Quick => 12_000,
            Self::Stress => 240_000,
        }
    }

    const fn predicate_samples(self) -> usize {
        match self {
            Self::Quick => 9,
            Self::Stress => 21,
        }
    }

    const fn predicates_per_sample(self) -> usize {
        match self {
            Self::Quick => 10_000,
            Self::Stress => 100_000,
        }
    }
}

fn print_help() {
    println!("usage: exedra_triangulate_bench [--quick | --stress] [--svg <directory>]");
    println!("  --quick   short timing profile (default)");
    println!("  --stress  longer timing profile over the same fixed corpus");
    println!("  --svg     also write one SVG per fixture and strategy into <directory>");
}

pub(crate) fn strategy_params(strategy: TriStrategy) -> TriParams {
    match strategy {
        TriStrategy::EarClip => TriParams::ear_clip(),
        TriStrategy::ConstrainedDelaunay => TriParams::constrained_delaunay(),
        _ => TriParams::default(),
    }
}

pub(crate) const fn strategy_label(strategy: TriStrategy) -> &'static str {
    match strategy {
        TriStrategy::EarClip => "EarClip",
        TriStrategy::ConstrainedDelaunay => "ConstrainedDelaunay",
        _ => "Unknown",
    }
}
