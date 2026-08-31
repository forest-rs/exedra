// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable one-bay structural laboratory for the basilica example.

mod emit;
mod model;

use std::path::{Path, PathBuf};

use basilica_ruin::BasilicaPremises;

use joiner::Construction;

use crate::emit::Layer;

fn main() {
    if let Err(error) = run() {
        eprintln!("basilica_structure_lab: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = parse_args()?;
    let construction = model::western_bay(&BasilicaPremises::default());
    let report = model::check(&construction);
    println!("{}", model::stats_line(&construction));
    println!("{report}");
    if !report.is_clean() {
        return Err("refusing to emit invalid structural geometry".to_owned());
    }

    match command {
        Command::Check => Ok(()),
        Command::Explain(key) => {
            let explanation = model::explain(&construction, &key)?
                .ok_or_else(|| format!("unknown structural key {key:?}"))?;
            println!("{explanation}");
            Ok(())
        }
        Command::Layer { layer, path } => {
            export_layer(&construction, layer, &path)?;
            Ok(())
        }
        Command::All => {
            let output = default_output_dir();
            for layer in Layer::ALL {
                export_layer(
                    &construction,
                    layer,
                    &output.join(format!("{}.obj", layer.label())),
                )?;
            }
            Ok(())
        }
        Command::Help => {
            print_help();
            Ok(())
        }
    }
}

#[derive(Debug)]
enum Command {
    All,
    Check,
    Explain(String),
    Layer { layer: Layer, path: PathBuf },
    Help,
}

fn parse_args() -> Result<Command, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Ok(Command::All);
    }
    match args[0].as_str() {
        "--check" if args.len() == 1 => Ok(Command::Check),
        "--explain" if args.len() == 2 => Ok(Command::Explain(args[1].clone())),
        "--layer" if args.len() == 2 || args.len() == 4 => {
            let layer = Layer::parse(&args[1]).ok_or_else(|| {
                format!(
                    "unknown layer {:?}; expected full, structure, load-path, bearings, or transparent-roof",
                    args[1]
                )
            })?;
            let path = if args.len() == 4 {
                if args[2] != "--obj" {
                    return Err(format!("expected --obj, found {:?}", args[2]));
                }
                PathBuf::from(&args[3])
            } else {
                default_output_dir().join(format!("{}.obj", layer.label()))
            };
            Ok(Command::Layer { layer, path })
        }
        "--help" | "-h" if args.len() == 1 => Ok(Command::Help),
        _ => Err("usage: basilica_structure_lab [--check | --explain <type:key> | --layer <name> [--obj <path>]]".to_owned()),
    }
}

fn export_layer(construction: &Construction, layer: Layer, path: &Path) -> Result<(), String> {
    let scene = emit::emit(construction, layer)?;
    scene.write_grouped_obj(path)?;
    println!(
        "layer={} parts={} groups={} obj={}",
        layer.label(),
        scene.assembly.parts().len(),
        scene.group_count(),
        path.display()
    );
    Ok(())
}

fn default_output_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/basilica_structure_lab")
}

fn print_help() {
    println!(
        "basilica_structure_lab\n\n\
         no arguments                  validate and export every semantic layer\n\
         --check                       validate only\n\
         --explain <type:key>          print one route, measured witness, or graph record\n\
         --layer <name> [--obj <path>] validate and export one layer\n\n\
         selector types: element, node, member, joint, bearing, support, transfer, evidence\n\
         layers: full, structure, load-path, bearings, transparent-roof"
    );
}
