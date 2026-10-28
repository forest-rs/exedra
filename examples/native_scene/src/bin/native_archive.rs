// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Save native geometry, reopen in a fresh store, machine again and export STEP.

use exedra_analytic::{
    body::ValidationPolicy,
    extrusion::Extrusion,
    machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder_with_roles},
    realize::RealizationPolicy,
};
use exedra_compile::NativeSource;
use exedra_constructive::{builders, ir::Placement3, profile::SegTag};
use exedra_scene::{
    composition::*,
    native::{ArchiveLimits, NativeArchive, define_native_body, read_native_body},
};
use std::{error::Error, path::Path};

struct NoAssets;
impl AssetResolver for NoAssets {
    fn resolve(
        &mut self,
        _: &str,
        _: Option<LayerId>,
        _: &TokenInterner,
        _: &PathInterner,
    ) -> Result<ResolvedAsset, AssetResolveError> {
        Err(AssetResolveError::NotFound)
    }
    fn resolved_path(&self, _: LayerId) -> Option<&str> {
        None
    }
}
fn other_support_sources(
    frame: Placement3,
) -> Result<Vec<(&'static str, NativeSource)>, Box<dyn Error>> {
    use exedra_analytic::{
        loft::{Loft, LoftPolicy},
        revolve::Revolution,
    };
    use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    let policy = ProfilePolicy::new(1e-10);
    let cone = Profile::new(
        Loop::new(vec![
            Segment::line([0.3, 0.0]),
            Segment::line([0.0, 0.7]),
            Segment::line([0.0, 0.0]),
        ])?,
        vec![],
        &policy,
    )?;
    let radius = 0.1_f64.next_up();
    let sphere = Profile::new(
        Loop::new(vec![
            Segment::line([0.0, -radius]),
            Segment::arc([0.0, radius], 1.0),
        ])?,
        vec![],
        &policy,
    )?;
    let torus = builders::circle(0.1)?.translated((1.0, 0.0))?;
    let section = builders::rect_centered(0.7, 0.9)?.tagged(SegTag(102));
    Ok(vec![
        (
            "/Cone",
            NativeSource::from(Revolution::new(cone, frame, core::f64::consts::TAU)?),
        ),
        (
            "/Sphere",
            NativeSource::from(Revolution::new(sphere, frame, core::f64::consts::TAU)?),
        ),
        (
            "/Torus",
            NativeSource::from(Revolution::from_profile(
                &torus,
                frame,
                core::f64::consts::TAU,
                &policy,
            )?),
        ),
        (
            "/Ruled",
            NativeSource::from(Loft::from_profiles(
                [
                    (Placement3::IDENTITY, &section),
                    (Placement3::translate(0.05, 0.0, 1.0), &section),
                ],
                frame,
                &LoftPolicy::new(policy),
            )?),
        ),
    ])
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "native-archive-output".into());
    run(Path::new(&path))
}
fn run(output: &Path) -> Result<(), Box<dyn Error>> {
    let profile = builders::rect_centered(20.0, 12.0)?.tagged(SegTag(17));
    let stock = Extrusion::from_profile(
        &profile,
        Placement3::IDENTITY,
        5.0,
        &exedra_profile::ProfilePolicy::new(1e-9),
    )?
    .body()
    .clone()
    .map_roles(|_| vec![SegTag(17)], |_| ());
    let policy = MachiningPolicy::default();
    let machined = subtract_through_cylinder_with_roles(
        &stock,
        &CylinderTool::new([-4.0, 0.0, 0.0], [0.15, 0.0, 1.0], 0.75_f64.next_up())?,
        &policy,
        vec![SegTag(73), SegTag(u32::MAX)],
        (),
    )?;
    assert!(
        !machined.body.trims().is_empty(),
        "operation must author actual parameter trims"
    );
    let frame = Placement3::rotate_y_then_translate(
        0.321234567890123,
        1_000_000.03,
        -0.0,
        -2.345678901234567,
    );
    let source = NativeSource::from_body(machined.body, frame)?;
    let limits = ArchiveLimits::default();
    let validation = ValidationPolicy { max_gap: 1e-9 };
    let expected = NativeArchive::capture(&source, &limits)?;
    let mut store = InMemoryStore::default();
    let root = LayerId(1);
    store.insert_layer(Layer::new(root));
    let path = store.path("/Housing");
    let mut live = LiveStage::compose(&mut store, root, StageOptions::default());
    let tx = define_native_body(
        &mut store,
        &EditTarget::for_layer(root),
        path,
        &source,
        &limits,
    )?;
    live.apply(&mut store, &tx)?;
    let mut support_archives = Vec::new();
    for (name, native) in other_support_sources(frame)? {
        let path = store.path(name);
        let tx = define_native_body(
            &mut store,
            &EditTarget::for_layer(root),
            path,
            &native,
            &limits,
        )?;
        live.apply(&mut store, &tx)?;
        support_archives.push((name, NativeArchive::capture(&native, &limits)?));
    }
    let text = layerstack_usda::save::save_usda(
        store.layer(root).expect("authored layer"),
        &store.tokens,
        &store.paths,
    )?;
    std::fs::create_dir_all(output)?;
    let file = output.join("housing.usda");
    std::fs::write(&file, text)?;
    drop(live);
    drop(store);
    let text = std::fs::read_to_string(&file)?;
    let mut fresh = InMemoryStore::default();
    let loaded = layerstack_usda::read_usda(
        &text,
        LayerId(77),
        &fresh.tokens,
        &fresh.paths,
        &mut NoAssets,
    );
    assert!(loaded.parse_diagnostics.is_empty(), "saved USDA must parse");
    assert!(
        loaded.lower_diagnostics.is_empty(),
        "saved values must lower exactly"
    );
    assert!(
        loaded.emitted.diagnostics.is_empty(),
        "saved layer must emit exactly"
    );
    fresh.insert_layer(loaded.emitted.layer);
    let path = fresh.path("/Housing");
    let stage = Stage::compose(&mut fresh, LayerId(77), StageOptions::default());
    let restored = read_native_body(&stage, &fresh, path, &validation, &limits)?;
    let actual = NativeArchive::capture(&restored, &limits)?;
    assert_eq!(
        actual.topology, expected.topology,
        "all shared topology, trim incidence and feature IDs survive fresh-store reopen"
    );
    assert_eq!(
        actual
            .geometry
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        expected
            .geometry
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "all f64 support/curve/trim/frame bits survive USDA"
    );
    assert!(
        !restored.same_installation(&source),
        "reopen installs a fresh selection scope"
    );
    for (name, expected) in &support_archives {
        let path = fresh.path(name);
        let restored = read_native_body(&stage, &fresh, path, &validation, &limits)?;
        let actual = NativeArchive::capture(&restored, &limits)?;
        assert_eq!(
            actual.topology, expected.topology,
            "support source topology must survive USDA: {name}"
        );
        assert_eq!(
            actual
                .geometry
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            expected
                .geometry
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            "every support geometry bit must survive USDA: {name}"
        );
    }
    let second = subtract_through_cylinder_with_roles(
        &restored.to_body(),
        &CylinderTool::new([4.0, 0.0, 0.0], [0.15, 0.0, 1.0], 0.65)?,
        &policy,
        vec![SegTag(91)],
        (),
    )?;
    // Realize the body locally; its exact large-datum occurrence placement
    // remains separate and is applied in the f64 STEP export below.
    let realized = second
        .body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())?;
    assert!(
        realized.report.within_tolerance(),
        "continued native operation must realize accurately"
    );
    assert!(
        realized.mesh.boundary_loops()?.is_empty(),
        "reopened solid stays closed after machining"
    );
    let step = exedra_step::export_body(
        &second.body,
        restored.frame(),
        &exedra_step::StepOptions {
            name: "Reopened native housing",
            timestamp: "2026-10-29T00:00:00Z",
            tolerance: 1e-6,
            ..Default::default()
        },
    )?;
    std::fs::write(output.join("housing-second-bore.step"), &step.text)?;
    println!(
        "{}",
        serde_json::json!({"native_faces":second.body.faces().len(),"trim_records":second.body.trims().len(),"triangles":realized.mesh_faces.len(),"exact_f64_values":actual.geometry.len(),"topology_words":actual.topology.len(),"fresh_store":true,"all_support_variants":true,"continued_operation":true,"step_bytes":step.text.len()})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;

    struct OutputDirectory(PathBuf);

    impl OutputDirectory {
        fn create() -> io::Result<Self> {
            // Stay inside the WASI runner's preopened workspace. Claim a fresh
            // directory atomically so concurrent runs never share or remove it.
            for suffix in 0..u64::MAX {
                let path = PathBuf::from(format!(".exedra-native-archive-test-{suffix}"));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Ok(Self(path)),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "no unused native archive test directory",
            ))
        }
    }

    impl Drop for OutputDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn usda_fresh_store_reopen_preserves_native_bits_and_continues_machining() {
        let output = OutputDirectory::create().unwrap();
        super::run(&output.0).unwrap();
    }
}
