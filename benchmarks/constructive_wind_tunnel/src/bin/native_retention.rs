// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native installation, display realization and prepared-cache retention workload.

use exedra_analytic::{
    End,
    extrusion::{Extrusion, Pocket},
};
use exedra_compile::{
    CompilePolicy, GeometrySnapshot, NativeFamily, NativeSelectionError, NativeSource,
    PartCompiler, PartId, PartInput, PartSource, PreparedSource,
};
use exedra_constructive::{builders, ir::Placement3};
use exedra_profile::{ProfilePolicy, SegTag};
use std::{hint::black_box, sync::Arc, time::Instant};

const FLOOR: SegTag = SegTag(7);

fn model() -> Extrusion {
    let policy = ProfilePolicy::new(1e-9);
    let profile = builders::rounded_rect_centered(120.0, 80.0, 6.0)
        .unwrap()
        .to_line_arc(&policy)
        .unwrap();
    let recess = builders::rounded_rect_centered(90.0, 50.0, 4.0)
        .unwrap()
        .to_line_arc(&policy)
        .unwrap();
    Extrusion::new(profile, Placement3::IDENTITY, 12.0)
        .unwrap()
        .with_pocket(
            Pocket::from_region(recess.outer(), End::End, 4.0).tagged(FLOOR),
            &policy,
        )
        .unwrap()
}
fn snapshot(
    compiler: &mut PartCompiler,
    source: &PreparedSource,
    tolerance: f64,
) -> GeometrySnapshot {
    let mut policy = CompilePolicy::default();
    policy.native.tolerance = tolerance;
    compiler
        .compile_snapshot(&[PartInput::prepared("panel", source)], &policy)
        .unwrap()
}
fn measure<T>(count: usize, mut action: impl FnMut() -> T) -> Vec<u128> {
    for _ in 0..3 {
        black_box(action());
    }
    (0..count)
        .map(|_| {
            let start = Instant::now();
            let output = action();
            black_box(&output);
            let elapsed = start.elapsed().as_nanos();
            drop(output);
            elapsed
        })
        .collect()
}
fn verify_assembly(native: &NativeSource) {
    use exedra_assembly::{
        Assembly,
        interchange::{AssemblyInterchangeError, to_dto},
    };
    let mut assembly = Assembly::new();
    let part = assembly
        .add_native_part("panel", native.clone(), &[])
        .unwrap();
    assembly
        .add_instance(None, "left", part, Placement3::IDENTITY)
        .unwrap();
    assembly
        .add_instance(None, "right", part, Placement3::translate(150.0, 0.0, 0.0))
        .unwrap();
    let mut compiler = exedra_assembly::PartCompiler::new();
    let snapshot = compiler
        .compile_snapshot(&assembly, &CompilePolicy::default())
        .unwrap();
    assert!(
        snapshot
            .body(part, 0)
            .unwrap()
            .native()
            .unwrap()
            .same_installation(native),
        "assembly snapshots retain the installed native source"
    );
    assert_eq!(
        compiler.counters().native_realizations,
        1,
        "occurrences share one native realization"
    );
    assert!(
        matches!(to_dto(&assembly), Err(AssemblyInterchangeError::UnsupportedNativeSource {
        part: 0, key, family: NativeFamily::Extrusion
    }) if key == "panel"),
        "legacy interchange must identify the unsupported native part"
    );
}
fn main() {
    let original = model();
    let native = NativeSource::from(original.clone());
    let prepared = PreparedSource::new(PartSource::Native(native.clone()));
    let floor = native.as_extrusion().unwrap().floor_tagged(FLOOR).unwrap();
    let selection = native.select_face(floor).unwrap();
    let workplane = native.as_extrusion().unwrap().workplane(floor).unwrap();
    black_box(workplane);
    verify_assembly(&native);
    let mut compiler = PartCompiler::new();
    let coarse = snapshot(&mut compiler, &prepared, 0.5);
    let fine = snapshot(&mut compiler, &prepared, 0.01);
    let triangles = |s: &GeometrySnapshot| s.compiled().part(PartId(0)).unwrap().triangle_count();
    assert!(
        triangles(&fine) > triangles(&coarse),
        "finer display policy must add curve samples"
    );
    for s in [&coarse, &fine] {
        let body = s.body(PartId(0), 0).unwrap();
        let mesh = &body.geometry().mesh;
        assert!(
            mesh.validate_deep().is_empty(),
            "realized panel must have valid topology"
        );
        assert!(
            mesh.boundary_loops().unwrap().is_empty(),
            "pocketed panel must remain closed"
        );
        assert!(
            body.native_realization().unwrap().report.within_tolerance(),
            "display realization must meet requested tolerance"
        );
        assert_eq!(
            body.native().unwrap().resolve_face(&selection),
            Ok(floor),
            "native floor selection must survive display changes"
        );
        for triangle in 0..body.native_realization().unwrap().render_faces().len() {
            let selected = body.native_render_face(triangle).unwrap();
            assert_eq!(
                native.resolve_face(&selected),
                Ok(selected.native_id()),
                "every render triangle must resolve to this source"
            );
        }
    }
    let fresh = NativeSource::from(original.clone());
    let equal = PreparedSource::new(PartSource::Native(fresh.clone()));
    let hit = snapshot(&mut compiler, &equal, 0.01);
    assert!(
        Arc::ptr_eq(
            fine.compiled().part(PartId(0)).unwrap(),
            hit.compiled().part(PartId(0)).unwrap()
        ),
        "equal-content installations must share cached display buffers"
    );
    assert_eq!(
        hit.body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .resolve_face(&selection),
        Err(NativeSelectionError::SourceMismatch),
        "fresh equal-content installation must reject the old native selection"
    );
    assert_eq!(
        compiler.counters().native_realizations,
        2,
        "cache hits must not realize native geometry again"
    );
    let cache_samples = measure(101, || snapshot(&mut compiler, &prepared, 0.01));
    assert_eq!(
        compiler.counters().native_realizations,
        2,
        "cache hits must not realize native geometry again"
    );
    let install_samples = measure(101, || NativeSource::from(original.clone()));
    let cold_samples = measure(21, || snapshot(&mut PartCompiler::new(), &prepared, 0.01));
    let storage = compiler.native_storage();
    let counters = compiler.counters();
    compiler.clear_cache();
    assert_eq!(
        compiler.native_storage().sources,
        0,
        "cache eviction must release cache-owned native sources"
    );
    assert_eq!(
        fine.body(PartId(0), 0)
            .unwrap()
            .native()
            .unwrap()
            .resolve_face(&selection),
        Ok(floor),
        "held snapshots and selections must survive cache eviction"
    );
    println!(
        "{}",
        serde_json::json!({
            "fixture":"rounded pocketed panel 120x80x12", "coarse_tolerance":0.5,"fine_tolerance":0.01,
            "native_volume":native.volume().unwrap(),"native_surface_area":native.surface_area().unwrap(),
            "coarse_triangles":triangles(&coarse),"fine_triangles":triangles(&fine),
            "cache_native_sources":storage.sources,"cache_native_realizations":storage.realizations,
            "cache_native_boundary_payload_bytes":storage.boundary_payload_bytes,
            "payload_exclusions":"constructor/profile/path records, spare capacity, allocator overhead, mesh/render/correspondence buffers, externally held snapshots",
            "work":{"native_realizations":counters.native_realizations,"parts_compiled":counters.parts_compiled,"cache_hits":counters.cache_hits},
            "samples_ns":{"native_install_with_constructor_clone":install_samples,"cold_fine_compile_snapshot":cold_samples,"prepared_fine_cache_hit_snapshot":cache_samples},
            "timing":"3 warmups; output drop outside timer; constructor already authored; installation includes clone and content fingerprint; cold compiler creation included"
        })
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_native_assembly_preserves_source_and_refuses_legacy_export() {
        super::verify_assembly(&super::NativeSource::from(super::model()));
    }
}
