// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use exedra_assembly::{
    Assembly, CompileCounters, CompiledParts, PartCompiler, PartSource, RenderList,
    assembly_fingerprint, flatten,
};
use exedra_constructive::evaluate::evaluate;
use exedra_constructive::ir::Placement3;
use exedra_constructive::tessellate::EvalPolicy;

use crate::BasilicaParams;
use crate::architecture::{Inventory, build_assembly};

#[derive(Debug)]
pub(crate) struct Scenario {
    pub(crate) assembly: Assembly,
    pub(crate) compiled: CompiledParts,
    pub(crate) render_list: RenderList,
    compile_counters: CompileCounters,
    pub(crate) inventory: Inventory,
    pub(crate) diagnostics: usize,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SceneStats {
    instances: u64,
    render_items: u64,
    triangles: u64,
    vertices: u64,
}

pub(crate) fn run_cli() {
    let output = parse_output_path();
    let scenario = build_scenario();
    let obj = export_obj(&scenario.compiled, &scenario.render_list);
    let gltf = exedra_gltf::export_gltf(
        &scenario.assembly,
        &scenario.compiled,
        &scenario.render_list,
    )
    .expect("the render list references its compiled assembly");

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).expect("create artifact directory");
    }
    std::fs::write(&output, &obj).expect("write OBJ artifact");
    let gltf_path = output.with_extension("gltf");
    std::fs::write(&gltf_path, &gltf.json).expect("write glTF artifact");

    let stats = scene_stats(&scenario);
    println!(
        "basilica instances={} items={} triangles={} vertices={} nave_walls={} aisles={} aisle_roofs={} arches={} interior_arcades={} interior_arcade_openings={} buttresses={} crossing_piers={} crossing_stages={} drum_windows={} cornice_bands={} chancel_openings={} apses={} drums={} domes={} ruined_bays={} parts_compiled={} cache_hits={} gltf_meshes={} diagnostics={} fingerprint={:032x} signature={:016x}",
        stats.instances,
        stats.render_items,
        stats.triangles,
        stats.vertices,
        scenario.inventory.nave_walls,
        scenario.inventory.aisles,
        scenario.inventory.aisle_roofs,
        scenario.inventory.round_head_openings,
        scenario.inventory.interior_arcades,
        scenario.inventory.interior_arcade_openings,
        scenario.inventory.buttresses,
        scenario.inventory.crossing_piers,
        scenario.inventory.crossing_stages,
        scenario.inventory.drum_windows,
        scenario.inventory.cornice_bands,
        scenario.inventory.chancel_openings,
        scenario.inventory.apses,
        scenario.inventory.drums,
        scenario.inventory.domes,
        scenario.inventory.ruined_bays,
        scenario.compile_counters.parts_compiled,
        scenario.compile_counters.cache_hits,
        gltf.stats.meshes,
        scenario.diagnostics,
        assembly_fingerprint(&scenario.assembly),
        byte_signature(obj.as_bytes()),
    );
    println!("obj={}", output.display());
    println!("gltf={}", gltf_path.display());
}

fn parse_output_path() -> PathBuf {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (None, None, None) => default_output_path(),
        (Some("--obj"), Some(path), None) => PathBuf::from(path),
        _ => panic!("usage: basilica_ruin [--obj <path>]"),
    }
}

fn default_output_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target/basilica_ruin/basilica_ruin.obj")
}

pub(crate) fn build_scenario() -> Scenario {
    let params = BasilicaParams::default();
    let (assembly, inventory) = build_assembly(&params);
    let policy = EvalPolicy::default();
    // Assembly compilation does not yet retain constructive reports
    // (ea-8tpb), so the scenario audits each recipe before compilation.
    let diagnostics = assembly
        .parts()
        .iter()
        .map(|part| match part.source() {
            PartSource::Recipe(recipe) => evaluate(recipe, &policy)
                .expect("constant basilica recipe evaluates")
                .report
                .diagnostics
                .len(),
            PartSource::Baked(_) => 0,
        })
        .sum();
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&assembly, &policy)
        .expect("constant basilica recipes evaluate");
    let render_list = flatten(&assembly, &compiled);
    Scenario {
        assembly,
        compiled,
        render_list,
        compile_counters: compiler.counters(),
        inventory,
        diagnostics,
    }
}

pub(crate) fn scene_stats(scenario: &Scenario) -> SceneStats {
    let mut stats = SceneStats {
        instances: scenario.assembly.instances().len() as u64,
        render_items: scenario.render_list.items.len() as u64,
        ..SceneStats::default()
    };
    for item in &scenario.render_list.items {
        let body = &scenario
            .compiled
            .part(item.part)
            .expect("compiled part exists")
            .bodies[item.body as usize];
        stats.triangles += (body.tri.indices.len() / 3) as u64;
        stats.vertices += body.tri.positions.len() as u64;
    }
    stats
}

pub(crate) fn export_obj(compiled: &CompiledParts, list: &RenderList) -> String {
    let mut out = String::from("# Byzantine basilica ruin; generated deterministically\n");
    let mut vertex_base = 1_u32;
    for item in &list.items {
        let body = &compiled
            .part(item.part)
            .expect("render item part is compiled")
            .bodies[item.body as usize];
        let name = item.path.to_string().replace('/', "_");
        writeln!(out, "o {name}.body{}", item.body).expect("write to String");
        writeln!(out, "g {name}").expect("write to String");
        for &position in &body.tri.positions {
            let p = transform_point(&item.world, position);
            writeln!(out, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]).expect("write to String");
        }
        for &uv in &body.tri.uvs {
            writeln!(out, "vt {:.6} {:.6}", uv[0], uv[1]).expect("write to String");
        }
        for &normal in &body.tri.normals {
            let n = transform_vector(&item.world, normal);
            writeln!(out, "vn {:.6} {:.6} {:.6}", n[0], n[1], n[2]).expect("write to String");
        }
        for triangle in body.tri.indices.chunks_exact(3) {
            let a = vertex_base + triangle[0];
            let b = vertex_base + triangle[1];
            let c = vertex_base + triangle[2];
            writeln!(out, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}").expect("write to String");
        }
        vertex_base += u32::try_from(body.tri.positions.len()).expect("example fits u32");
    }
    out
}

fn transform_point(placement: &Placement3, p: [f32; 3]) -> [f64; 3] {
    let p = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
    let r = &placement.rows;
    [
        r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2] + r[0][3],
        r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2] + r[1][3],
        r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2] + r[2][3],
    ]
}

fn transform_vector(placement: &Placement3, p: [f32; 3]) -> [f64; 3] {
    let p = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
    let r = &placement.rows;
    [
        r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2],
        r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2],
        r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2],
    ]
}

#[cfg(test)]
pub(crate) fn bounds(compiled: &CompiledParts, list: &RenderList) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for item in &list.items {
        let body = &compiled
            .part(item.part)
            .expect("render item part is compiled")
            .bodies[item.body as usize];
        for &position in &body.tri.positions {
            let p = transform_point(&item.world, position);
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
    }
    (min, max)
}

#[cfg(test)]
pub(crate) fn bounds_for_path(
    compiled: &CompiledParts,
    list: &RenderList,
    expected_path: &str,
) -> ([f64; 3], [f64; 3]) {
    let item = list
        .items
        .iter()
        .find(|item| item.path.to_string() == expected_path)
        .unwrap_or_else(|| panic!("missing render item {expected_path}"));
    let body = &compiled
        .part(item.part)
        .expect("render item part is compiled")
        .bodies[item.body as usize];
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for &position in &body.tri.positions {
        let p = transform_point(&item.world, position);
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    (min, max)
}

pub(crate) fn byte_signature(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::{build_scenario, export_obj};

    #[test]
    fn obj_groups_and_indices_are_well_formed() {
        let scenario = build_scenario();
        let obj = export_obj(&scenario.compiled, &scenario.render_list);
        assert!(obj.contains("g nave-wall-south-west-broken"));
        assert!(obj.contains("g interior-arcade-north-west"));
        assert!(obj.contains("g interior-arcade-south-east"));
        assert!(obj.contains("g crossing-dome"));
        assert_eq!(
            obj.lines().filter(|line| line.starts_with("g ")).count(),
            67
        );
        let vertices = obj.lines().filter(|line| line.starts_with("v ")).count();
        let mut max_index = 0_usize;
        for face in obj.lines().filter(|line| line.starts_with("f ")) {
            for corner in face.split_whitespace().skip(1) {
                let index = corner
                    .split('/')
                    .next()
                    .expect("vertex index")
                    .parse::<usize>()
                    .expect("numeric OBJ index");
                max_index = max_index.max(index);
            }
        }
        assert_eq!(max_index, vertices);
    }
}
