// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use exedra_assembly::{
    Assembly, CompileCounters, CompiledParts, PartCompiler, PartId, RenderList,
    assembly_fingerprint, flatten,
};
use exedra_constructive::ir::Placement3;
use exedra_constructive::tessellate::EvalPolicy;

use crate::BasilicaParams;
use crate::architecture::{Inventory, build_assembly};
use crate::names;

const WARM_DOME_HEIGHT_DELTA: f64 = 0.25;

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

#[derive(Debug)]
struct WarmReconfiguration {
    baseline: Scenario,
    warm: Scenario,
    fresh: Scenario,
}

#[derive(Debug)]
struct WarmProof {
    obj: String,
    changed_parts: Vec<String>,
    groups: usize,
    hits_per_miss: u64,
}

#[derive(Debug)]
enum CliMode {
    Export(PathBuf),
    WarmReconfigure,
}

pub(crate) fn run_cli() {
    match parse_cli_mode() {
        CliMode::Export(output) => run_export(&output),
        CliMode::WarmReconfigure => run_warm_reconfiguration(),
    }
}

fn run_export(output: &Path) {
    let scenario = build_scenario();
    let obj = export_obj(&scenario.compiled, &scenario.render_list);
    let (gltf_path, gltf_meshes) = write_artifacts(output, &scenario, &obj);

    print_scenario_summary(&scenario, &obj, gltf_meshes);
    println!("obj={}", output.display());
    println!("gltf={}", gltf_path.display());
}

fn run_warm_reconfiguration() {
    let reconfiguration = build_warm_reconfiguration();
    let proof = validate_warm_reconfiguration(&reconfiguration);
    let output = default_reconfigured_output_path();
    let (gltf_path, _) = write_artifacts(&output, &reconfiguration.warm, &proof.obj);

    println!(
        "basilica warm_reconfigure parameter=dome_height from={:.3} to={:.3} parts_compiled={} cache_hits={} hits_per_miss={} groups={} changed_parts={} signature={:016x} warm_matches_fresh=true",
        BasilicaParams::default().dome_height,
        BasilicaParams::default().dome_height + WARM_DOME_HEIGHT_DELTA,
        reconfiguration.warm.compile_counters.parts_compiled,
        reconfiguration.warm.compile_counters.cache_hits,
        proof.hits_per_miss,
        proof.groups,
        proof.changed_parts.join(","),
        byte_signature(proof.obj.as_bytes()),
    );
    println!("obj={}", output.display());
    println!("gltf={}", gltf_path.display());
}

fn write_artifacts(output: &Path, scenario: &Scenario, obj: &str) -> (PathBuf, u64) {
    let gltf = exedra_gltf::export_gltf_with_options(
        &scenario.assembly,
        &scenario.compiled,
        &scenario.render_list,
        exedra_gltf::GltfExportOptions::z_up_to_y_up(),
    )
    .expect("the render list references its compiled assembly");

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).expect("create artifact directory");
    }
    std::fs::write(output, obj).expect("write OBJ artifact");
    let gltf_path = output.with_extension("gltf");
    std::fs::write(&gltf_path, &gltf.json).expect("write glTF artifact");

    (gltf_path, gltf.stats.meshes)
}

fn print_scenario_summary(scenario: &Scenario, obj: &str, gltf_meshes: u64) {
    let stats = scene_stats(scenario);
    println!(
        "basilica instances={} items={} triangles={} vertices={} nave_walls={} nave_wall_plates={} aisles={} aisle_roofs={} arches={} interior_arcades={} interior_arcade_openings={} buttresses={} crossing_piers={} crossing_stages={} pendentives={} drum_windows={} cornice_bands={} chancel_openings={} apses={} drums={} domes={} ruined_bays={} nave_trusses={} nave_truss_members={} omitted_nave_trusses={} parts_compiled={} cache_hits={} gltf_meshes={} diagnostics={} fingerprint={:032x} signature={:016x}",
        stats.instances,
        stats.render_items,
        stats.triangles,
        stats.vertices,
        scenario.inventory.nave_walls,
        scenario.inventory.nave_wall_plates,
        scenario.inventory.aisles,
        scenario.inventory.aisle_roofs,
        scenario.inventory.round_head_openings,
        scenario.inventory.interior_arcades,
        scenario.inventory.interior_arcade_openings,
        scenario.inventory.buttresses,
        scenario.inventory.crossing_piers,
        scenario.inventory.crossing_stages,
        scenario.inventory.pendentives,
        scenario.inventory.drum_windows,
        scenario.inventory.cornice_bands,
        scenario.inventory.chancel_openings,
        scenario.inventory.apses,
        scenario.inventory.drums,
        scenario.inventory.domes,
        scenario.inventory.ruined_bays,
        scenario.inventory.nave_trusses,
        scenario.inventory.nave_truss_members,
        scenario.inventory.omitted_nave_trusses,
        scenario.compile_counters.parts_compiled,
        scenario.compile_counters.cache_hits,
        gltf_meshes,
        scenario.diagnostics,
        assembly_fingerprint(&scenario.assembly),
        byte_signature(obj.as_bytes()),
    );
}

fn parse_cli_mode() -> CliMode {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (None, None, None) => CliMode::Export(default_output_path()),
        (Some("--obj"), Some(path), None) => CliMode::Export(PathBuf::from(path)),
        (Some("--warm-reconfigure"), None, None) => CliMode::WarmReconfigure,
        _ => panic!("usage: basilica_ruin [--obj <path> | --warm-reconfigure]"),
    }
}

fn default_output_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target/basilica_ruin/basilica_ruin.obj")
}

fn default_reconfigured_output_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target/basilica_ruin/basilica_ruin_warm.obj")
}

pub(crate) fn build_scenario() -> Scenario {
    let params = BasilicaParams::default();
    let mut compiler = PartCompiler::new();
    compile_scenario(&params, &mut compiler)
}

fn build_warm_reconfiguration() -> WarmReconfiguration {
    let mut compiler = PartCompiler::new();
    let baseline_params = BasilicaParams::default();
    let baseline = compile_scenario(&baseline_params, &mut compiler);

    let mut edited_params = baseline_params;
    edited_params.dome_height += WARM_DOME_HEIGHT_DELTA;
    let warm = compile_scenario(&edited_params, &mut compiler);
    let fresh = compile_scenario(&edited_params, &mut PartCompiler::new());

    WarmReconfiguration {
        baseline,
        warm,
        fresh,
    }
}

fn compile_scenario(params: &BasilicaParams, compiler: &mut PartCompiler) -> Scenario {
    let (assembly, inventory) = build_assembly(params);
    let policy = EvalPolicy::default();
    let counters_before = compiler.counters();
    let compiled = compiler
        .compile_parts(&assembly, &policy)
        .expect("constant basilica recipes evaluate");
    let diagnostics = compiled
        .parts()
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let id = PartId(u32::try_from(index).expect("part count is bounded by PartId"));
            compiled
                .report(id)
                .map_or(0, |report| report.diagnostics.len())
        })
        .sum();
    let render_list = flatten(&assembly, &compiled);
    Scenario {
        assembly,
        compiled,
        render_list,
        compile_counters: counter_delta(counters_before, compiler.counters()),
        inventory,
        diagnostics,
    }
}

fn counter_delta(before: CompileCounters, after: CompileCounters) -> CompileCounters {
    CompileCounters {
        parts_compiled: after
            .parts_compiled
            .checked_sub(before.parts_compiled)
            .expect("compiler counters are monotonic"),
        cache_hits: after
            .cache_hits
            .checked_sub(before.cache_hits)
            .expect("compiler counters are monotonic"),
        cache_evictions: after
            .cache_evictions
            .checked_sub(before.cache_evictions)
            .expect("compiler counters are monotonic"),
        triangles_emitted: after
            .triangles_emitted
            .checked_sub(before.triangles_emitted)
            .expect("compiler counters are monotonic"),
    }
}

fn validate_warm_reconfiguration(reconfiguration: &WarmReconfiguration) -> WarmProof {
    let baseline_paths = render_paths(&reconfiguration.baseline.render_list);
    let warm_paths = render_paths(&reconfiguration.warm.render_list);
    assert_eq!(
        warm_paths, baseline_paths,
        "a dome-height edit must preserve every instance path and its order"
    );
    assert_eq!(
        reconfiguration.warm.assembly.instances().len(),
        reconfiguration.warm.render_list.items.len(),
        "the proof requires exactly one render item per assembly instance"
    );
    assert_eq!(
        warm_paths,
        assembly_paths(&reconfiguration.warm.assembly),
        "render items must be a path-ordered bijection with assembly instances"
    );
    assert_unique(&warm_paths, "render instance paths");

    let changed_render_paths = changed_render_paths(
        &reconfiguration.baseline,
        &reconfiguration.warm.compiled,
        &reconfiguration.warm.render_list,
    );
    assert_eq!(
        changed_render_paths,
        [names::instances::CROSSING_DOME],
        "only the dome render payload may change"
    );

    let warm_obj = export_obj(
        &reconfiguration.warm.compiled,
        &reconfiguration.warm.render_list,
    );
    let baseline_obj = export_obj(
        &reconfiguration.baseline.compiled,
        &reconfiguration.baseline.render_list,
    );
    let fresh_obj = export_obj(
        &reconfiguration.fresh.compiled,
        &reconfiguration.fresh.render_list,
    );
    assert_eq!(
        warm_obj, fresh_obj,
        "warm and fresh edited exports must be byte-identical"
    );

    let expected_groups: Vec<String> = warm_paths
        .iter()
        .map(|path| path.replace('/', "_"))
        .collect();
    let actual_groups = obj_groups(&warm_obj);
    assert_eq!(
        actual_groups, expected_groups,
        "OBJ must retain one ordered group per render instance"
    );
    assert_eq!(
        actual_groups,
        obj_groups(&baseline_obj),
        "reconfiguration must preserve every OBJ group and its order"
    );
    assert_eq!(
        actual_groups.len(),
        reconfiguration.warm.assembly.instances().len(),
        "the proof requires exactly one OBJ group per assembly instance"
    );
    assert_unique(&actual_groups, "OBJ groups");

    let changed_parts = changed_part_keys(&reconfiguration.baseline, &reconfiguration.warm);
    assert_eq!(
        changed_parts,
        [names::parts::CROSSING_DOME],
        "only the named dome recipe may change"
    );

    let misses = reconfiguration.warm.compile_counters.parts_compiled;
    let hits = reconfiguration.warm.compile_counters.cache_hits;
    assert!(misses > 0, "the edited dome must be recompiled");
    assert_eq!(
        misses,
        u64::try_from(changed_parts.len()).expect("example part count fits u64"),
        "every changed part and only a changed part must miss the cache"
    );
    assert_eq!(
        hits + misses,
        u64::try_from(reconfiguration.warm.assembly.parts().len())
            .expect("example part count fits u64"),
        "every concrete part must be either reused or compiled"
    );
    assert!(
        hits >= misses.saturating_mul(10),
        "warm rebuild must record at least ten cache hits per miss: {hits}/{misses}"
    );

    WarmProof {
        obj: warm_obj,
        changed_parts,
        groups: actual_groups.len(),
        hits_per_miss: hits / misses,
    }
}

fn render_paths(list: &RenderList) -> Vec<String> {
    list.items
        .iter()
        .map(|item| item.path.to_string())
        .collect()
}

fn assembly_paths(assembly: &Assembly) -> Vec<String> {
    assembly
        .instances_with_ids()
        .map(|(id, _)| {
            assembly
                .path_of(id)
                .expect("every assembly instance has a path")
                .to_string()
        })
        .collect()
}

fn obj_groups(obj: &str) -> Vec<String> {
    obj.lines()
        .filter_map(|line| line.strip_prefix("g ").map(str::to_owned))
        .collect()
}

fn assert_unique(values: &[String], label: &str) {
    let mut unique = values.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), values.len(), "{label} must be unique");
}

fn changed_render_paths(
    baseline: &Scenario,
    edited_compiled: &CompiledParts,
    edited_list: &RenderList,
) -> Vec<String> {
    assert_eq!(
        baseline.render_list.items.len(),
        edited_list.items.len(),
        "reconfiguration must preserve render-item cardinality"
    );
    baseline
        .render_list
        .items
        .iter()
        .zip(&edited_list.items)
        .filter_map(|(baseline_item, edited_item)| {
            assert_eq!(
                baseline_item.path, edited_item.path,
                "render items must stay aligned by stable path"
            );
            let baseline_part = baseline
                .compiled
                .part(baseline_item.part)
                .expect("baseline render part is compiled");
            let edited_part = edited_compiled
                .part(edited_item.part)
                .expect("edited render part is compiled");
            let changed = baseline_item.world != edited_item.world
                || baseline_item.part != edited_item.part
                || baseline_item.body != edited_item.body
                || baseline_part.fingerprint != edited_part.fingerprint
                || baseline_item.regions != edited_item.regions;
            changed.then(|| baseline_item.path.to_string())
        })
        .collect()
}

fn changed_part_keys(baseline: &Scenario, edited: &Scenario) -> Vec<String> {
    assert_eq!(
        baseline.assembly.parts().len(),
        edited.assembly.parts().len(),
        "reconfiguration must preserve concrete part registration"
    );
    baseline
        .assembly
        .parts()
        .iter()
        .zip(edited.assembly.parts())
        .zip(
            baseline
                .compiled
                .parts()
                .iter()
                .zip(edited.compiled.parts()),
        )
        .filter_map(
            |((baseline_def, edited_def), (baseline_part, edited_part))| {
                assert_eq!(
                    baseline_def.key(),
                    edited_def.key(),
                    "part registration order is stable"
                );
                (baseline_part.fingerprint != edited_part.fingerprint)
                    .then(|| baseline_def.key().to_owned())
            },
        )
        .collect()
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
    use super::{
        build_scenario, build_warm_reconfiguration, byte_signature, export_obj,
        validate_warm_reconfiguration,
    };
    use crate::names;

    #[test]
    fn obj_groups_and_indices_are_well_formed() {
        let scenario = build_scenario();
        let obj = export_obj(&scenario.compiled, &scenario.render_list);
        assert!(obj.contains("g nave-wall-south-west-broken"));
        assert!(obj.contains("g interior-arcade-north-west"));
        assert!(obj.contains("g interior-arcade-south-east"));
        assert!(obj.contains("g crossing-pendentive-north-east"));
        assert!(obj.contains("g crossing-dome"));
        assert_eq!(
            obj.lines().filter(|line| line.starts_with("g ")).count(),
            112
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

    #[test]
    fn obj_group_order_preserves_every_existing_instance_identity() {
        let scenario = build_scenario();
        let obj = export_obj(&scenario.compiled, &scenario.render_list);
        let groups: Vec<&str> = obj
            .lines()
            .filter_map(|line| line.strip_prefix("g "))
            .collect();
        let instance_paths: Vec<String> = scenario
            .assembly
            .instances_with_ids()
            .map(|(id, _)| {
                scenario
                    .assembly
                    .path_of(id)
                    .expect("existing instance has a path")
                    .to_string()
            })
            .collect();
        const ACCEPTED_PATHS: [&str; 76] = [
            "nave-wall-north-west",
            "nave-wall-south-west-broken",
            "nave-wall-north-east",
            "nave-wall-south-east",
            "nave-wall-plate-north-west",
            "nave-wall-plate-south-west-a",
            "nave-wall-plate-south-west-b",
            "nave-wall-plate-north-east",
            "nave-wall-plate-south-east",
            "aisle-wall-north",
            "aisle-wall-south",
            "west-facade",
            "nave-roof-north-west",
            "nave-roof-south-west-a",
            "nave-roof-south-west-b",
            "nave-roof-north-east",
            "nave-roof-south-east",
            "crossing-shoulder-west",
            "crossing-shoulder-east",
            "aisle-roof-north",
            "aisle-roof-south",
            "aisle-eave-north",
            "aisle-eave-south",
            "east-aisle-end-north",
            "east-aisle-end-south",
            "interior-arcade-north-west",
            "interior-arcade-south-west",
            "interior-arcade-north-east",
            "interior-arcade-south-east",
            "east-chancel-gable",
            "east-apse",
            "east-apse-roof",
            "crossing-pier-south-west",
            "crossing-pier-north-west",
            "crossing-pier-south-east",
            "crossing-pier-north-east",
            "crossing-spandrel-south",
            "crossing-spandrel-north",
            "crossing-spandrel-west",
            "crossing-spandrel-east",
            "crossing-platform",
            "crossing-drum-panel-00",
            "crossing-drum-panel-01",
            "crossing-drum-panel-02",
            "crossing-drum-panel-03",
            "crossing-drum-panel-04",
            "crossing-drum-panel-05",
            "crossing-drum-panel-06",
            "crossing-drum-panel-07",
            "crossing-drum-panel-08",
            "crossing-drum-panel-09",
            "crossing-drum-panel-10",
            "crossing-drum-panel-11",
            "crossing-drum-cornice-base",
            "crossing-drum-cornice-top",
            "crossing-dome",
            "crossing-pendentive-north-east",
            "crossing-pendentive-north-west",
            "crossing-pendentive-south-west",
            "crossing-pendentive-south-east",
            "buttress-north-00",
            "buttress-north-01",
            "buttress-north-02",
            "buttress-north-03",
            "buttress-north-04",
            "buttress-north-05",
            "buttress-north-06",
            "buttress-north-07",
            "buttress-south-00",
            "buttress-south-01",
            "buttress-south-02",
            "buttress-south-03",
            "buttress-south-04",
            "buttress-south-05",
            "buttress-south-06",
            "buttress-south-07",
        ];

        const TRUSS_PATHS: [&str; 36] = [
            "nave-truss-west-00-tie-beam",
            "nave-truss-west-00-principal-rafter-north",
            "nave-truss-west-00-principal-rafter-south",
            "nave-truss-west-00-king-post",
            "nave-truss-west-00-diagonal-brace-north",
            "nave-truss-west-00-diagonal-brace-south",
            "nave-truss-west-01-tie-beam",
            "nave-truss-west-01-principal-rafter-north",
            "nave-truss-west-01-principal-rafter-south",
            "nave-truss-west-01-king-post",
            "nave-truss-west-01-diagonal-brace-north",
            "nave-truss-west-01-diagonal-brace-south",
            "nave-truss-west-03-tie-beam",
            "nave-truss-west-03-principal-rafter-north",
            "nave-truss-west-03-principal-rafter-south",
            "nave-truss-west-03-king-post",
            "nave-truss-west-03-diagonal-brace-north",
            "nave-truss-west-03-diagonal-brace-south",
            "nave-truss-west-04-tie-beam",
            "nave-truss-west-04-principal-rafter-north",
            "nave-truss-west-04-principal-rafter-south",
            "nave-truss-west-04-king-post",
            "nave-truss-west-04-diagonal-brace-north",
            "nave-truss-west-04-diagonal-brace-south",
            "nave-truss-west-05-tie-beam",
            "nave-truss-west-05-principal-rafter-north",
            "nave-truss-west-05-principal-rafter-south",
            "nave-truss-west-05-king-post",
            "nave-truss-west-05-diagonal-brace-north",
            "nave-truss-west-05-diagonal-brace-south",
            "nave-truss-east-00-tie-beam",
            "nave-truss-east-00-principal-rafter-north",
            "nave-truss-east-00-principal-rafter-south",
            "nave-truss-east-00-king-post",
            "nave-truss-east-00-diagonal-brace-north",
            "nave-truss-east-00-diagonal-brace-south",
        ];

        assert_eq!(instance_paths.len(), 112);
        assert_eq!(groups.len(), 112);
        assert_eq!(&instance_paths[..76], ACCEPTED_PATHS);
        assert_eq!(&groups[..76], ACCEPTED_PATHS);
        assert_eq!(&instance_paths[76..], TRUSS_PATHS);
        assert_eq!(&groups[76..], TRUSS_PATHS);
    }

    #[test]
    fn warm_dome_reconfiguration_reuses_every_unaffected_named_part() {
        let reconfiguration = build_warm_reconfiguration();
        let proof = validate_warm_reconfiguration(&reconfiguration);
        let baseline_obj = export_obj(
            &reconfiguration.baseline.compiled,
            &reconfiguration.baseline.render_list,
        );
        let fresh_obj = export_obj(
            &reconfiguration.fresh.compiled,
            &reconfiguration.fresh.render_list,
        );

        assert_eq!(proof.changed_parts, [names::parts::CROSSING_DOME]);
        assert_eq!(proof.groups, 112);
        assert_eq!(
            proof.groups,
            reconfiguration.warm.assembly.instances().len()
        );
        assert_eq!(proof.groups, reconfiguration.warm.render_list.items.len());
        assert!(proof.hits_per_miss >= 10);
        assert_ne!(proof.obj, baseline_obj);
        assert_eq!(proof.obj, fresh_obj);
        // Formatted floating-point coordinates may differ in their last decimal
        // across targets. The proof pins same-target determinism and semantic
        // identity instead of treating one platform's OBJ bytes as canonical.
        assert_eq!(
            byte_signature(proof.obj.as_bytes()),
            byte_signature(fresh_obj.as_bytes())
        );
    }

    #[test]
    #[should_panic(expected = "only the dome render payload may change")]
    fn warm_validation_rejects_non_dome_placement_drift() {
        let mut reconfiguration = build_warm_reconfiguration();
        let item = reconfiguration
            .warm
            .render_list
            .items
            .iter_mut()
            .find(|item| item.path.to_string() == names::instances::NAVE_WALL_NORTH_WEST)
            .expect("named non-dome render item");
        item.world.rows[0][3] += 0.5;

        validate_warm_reconfiguration(&reconfiguration);
    }

    #[test]
    #[should_panic(expected = "only the dome render payload may change")]
    fn warm_validation_rejects_non_dome_material_drift() {
        let mut reconfiguration = build_warm_reconfiguration();
        let item = reconfiguration
            .warm
            .render_list
            .items
            .iter_mut()
            .find(|item| item.path.to_string() == names::instances::NAVE_WALL_NORTH_WEST)
            .expect("named non-dome render item");
        item.regions
            .first_mut()
            .expect("nave wall has a resolved region")
            .material = Some("unexpected-material".to_owned());

        validate_warm_reconfiguration(&reconfiguration);
    }
}
