// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Assembly adapters over the independent geometry compilation service.
//! Placements and appearance are captured here; geometry algorithms and caches
//! belong to `exedra_compile`.

use crate::{Assembly, PartId};
use alloc::{sync::Arc, vec::Vec};
use exedra_compile::PartInput;
use exedra_constructive::evaluate::GeometryReport;

pub use exedra_compile::{
    BodyLookupError, CompilationMismatch, CompileCounters, CompileError, CompilePolicy,
    CompiledBody, CompiledPart, PartFingerprint, PolicyFingerprint, RegionRange, SignedVolume,
    SnapshotBody, SnapshotSurfaceInventory, SnapshotWorkplane, StaleSelection, VolumeError,
    policy_fingerprint,
};
mod evaluated;
#[cfg(test)]
mod failure_tests;
mod fingerprint;
mod snapshot;
pub use evaluated::EvaluationSnapshot;
pub use fingerprint::assembly_fingerprint;

fn inputs(assembly: &Assembly) -> Vec<PartInput<'_>> {
    assembly
        .parts()
        .iter()
        .map(|part| PartInput::prepared(part.key(), part.prepared_source()))
        .collect()
}

/// Compiled geometry associated with an assembly's local part handles.
/// Geometry and reports are owned by the independent compilation service.
#[derive(Clone, Debug)]
pub struct CompiledParts {
    geometry: exedra_compile::CompiledParts,
}
impl CompiledParts {
    /// Independent geometry results with the same local part ordering.
    #[must_use]
    pub fn geometry(&self) -> &exedra_compile::CompiledParts {
        &self.geometry
    }
    /// Compiled entry for an assembly-local part.
    #[must_use]
    pub fn part(&self, part: PartId) -> Option<&Arc<CompiledPart>> {
        self.geometry.part(part)
    }
    /// All compiled entries in local part order.
    #[must_use]
    pub fn parts(&self) -> &[Arc<CompiledPart>] {
        self.geometry.parts()
    }
    /// Constructive report for a recipe part, including cached diagnostics.
    /// Baked geometry and unknown handles have no constructive report.
    #[must_use]
    pub fn report(&self, part: PartId) -> Option<&GeometryReport> {
        self.geometry.report(part)
    }
}

/// Convenience adapter from assembly part records to independent compilation.
/// Has no geometry implementation, invalidation channels or separate cache.
#[derive(Debug, Default)]
pub struct PartCompiler {
    geometry: exedra_compile::PartCompiler,
}
impl PartCompiler {
    /// Creates an empty geometry compiler behind the assembly adapter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Access to the same service for standalone or native scene compilation.
    pub fn geometry_mut(&mut self) -> &mut exedra_compile::PartCompiler {
        &mut self.geometry
    }
    /// Lifetime work measured by the shared geometry compiler.
    #[must_use]
    pub fn counters(&self) -> CompileCounters {
        self.geometry.counters()
    }
    /// Content/policy pairs retained by the shared compiler.
    #[must_use]
    pub fn cached_entries(&self) -> usize {
        self.geometry.cached_entries()
    }
    /// Explicitly evicts content addressed by the last successful input ordering.
    /// This controls retention; fresh content fingerprints ensure correctness.
    pub fn mark_part_changed(&mut self, part: PartId) {
        self.geometry.mark_part_changed(part);
    }
    /// Releases the shared compiler cache; held snapshots remain usable.
    pub fn clear_cache(&mut self) {
        self.geometry.clear_cache();
    }
    /// Compiles geometry from part records without reading instance structure.
    /// # Errors
    /// Preserves checked evaluation failures and complete refusal reports.
    pub fn compile_parts(
        &mut self,
        assembly: &Assembly,
        policy: &CompilePolicy,
    ) -> Result<CompiledParts, CompileError> {
        self.geometry
            .compile_parts(&inputs(assembly), policy)
            .map(|geometry| CompiledParts { geometry })
    }
    /// Compiles retained geometry and captures assembly placement and appearance.
    /// # Errors
    /// Preserves the same failures as [`Self::compile_parts`].
    pub fn compile_snapshot(
        &mut self,
        assembly: &Assembly,
        policy: &CompilePolicy,
    ) -> Result<EvaluationSnapshot, CompileError> {
        let geometry = self.geometry.compile_snapshot(&inputs(assembly), policy)?;
        let compiled = CompiledParts {
            geometry: geometry.compiled().clone(),
        };
        let render = Arc::new(crate::flatten(assembly, &compiled));
        Ok(EvaluationSnapshot::new(geometry, compiled, render))
    }
}

impl AsRef<exedra_compile::CompiledParts> for CompiledParts {
    fn as_ref(&self) -> &exedra_compile::CompiledParts {
        self.geometry()
    }
}

#[cfg(test)]
#[path = "normal_tests.rs"]
mod normal_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly::{Assembly, InstanceId, PartSource};
    use exedra_constructive::builders;
    use exedra_constructive::discretize::{DiscretizeError, DiscretizePolicy};
    use exedra_constructive::evaluate::{Aabb3, EvalError, evaluate};
    use exedra_constructive::ir::{CapMode, NodeKind, Placement3, Plane3, Recipe, RecipeBuilder};
    use exedra_constructive::tessellate::EvalPolicy;
    use exedra_constructive::tessellate::TessellateError;
    use exedra_mesh::{ExtractAttribute, ExtractStats, TriMesh, UvSource};

    fn prism_recipe(width: f64) -> Recipe {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(width, 20.0).unwrap());
        let node = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        b.finish(node).unwrap()
    }

    fn two_body_recipe() -> Recipe {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::rect_from_corner(40.0, 20.0).unwrap());
        let first = builder
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let second = builder
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::translate(-5.0, 30.0, -2.0),
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let root = builder
            .add(NodeKind::Group {
                children: alloc::vec![first, second],
            })
            .unwrap();
        builder.finish(root).unwrap()
    }

    fn refused_open_shell_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(40.0, 20.0).unwrap());
        let face = b
            .add(NodeKind::PlanarFace {
                profile,
                placement: Placement3::IDENTITY,
            })
            .unwrap();
        let stretch = b
            .add(NodeKind::Stretch {
                child: face,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 20.0,
                },
                length: 10.0,
            })
            .unwrap();
        b.finish(stretch).unwrap()
    }

    fn partially_supported_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(40.0, 20.0).unwrap());
        let solid = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let open_face = b
            .add(NodeKind::PlanarFace {
                profile,
                placement: Placement3::translate(0.0, 0.0, 20.0),
            })
            .unwrap();
        let refused = b
            .add(NodeKind::Stretch {
                child: open_face,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 20.0,
                },
                length: 10.0,
            })
            .unwrap();
        let root = b
            .add(NodeKind::Group {
                children: alloc::vec![solid, refused],
            })
            .unwrap();
        b.finish(root).unwrap()
    }

    fn n_instance_assembly(n: u32) -> Assembly {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", prism_recipe(40.0)).unwrap();
        for i in 0..n {
            asm.add_instance(
                None,
                &alloc::format!("p{i}"),
                part,
                Placement3::translate(f64::from(i) * 50.0, 0.0, 0.0),
            )
            .unwrap();
        }
        asm
    }

    #[test]
    fn body_volume_is_local_and_occurrence_scale_is_explicit() {
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part("two-bodies", two_body_recipe())
            .unwrap();
        let placement = Placement3::from_axes(
            [-2.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
            [0.0, 0.0, 4.0],
            [100.0; 3],
        );
        assembly
            .add_instance(None, "original", part, Placement3::IDENTITY)
            .unwrap();
        assembly
            .add_instance(None, "scaled-reflection", part, placement)
            .unwrap();
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        let entry = compiled.part(part).unwrap();
        for body in &entry.bodies {
            let measurement = body.signed_volume().unwrap();
            assert_eq!(measurement.value, 8000.0);
            assert_eq!(
                measurement.transformed(&placement).unwrap().value,
                -192_000.0
            );
            assert_eq!(measurement.triangles_examined, body.tri.indices.len() / 3);
        }
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(crate::flatten(&assembly, &compiled).items.len(), 4);
    }

    #[test]
    fn n_instances_compile_once() {
        let asm = n_instance_assembly(8);
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiled.parts().len(), 1);
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(compiler.counters().cache_hits, 0);
        // Re-compiling the same assembly is a pure hit.
        let again = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(compiler.counters().cache_hits, 1);
        assert!(Arc::ptr_eq(
            compiled.part(PartId(0)).unwrap(),
            again.part(PartId(0)).unwrap()
        ));
    }

    #[test]
    fn compiled_bounds_cover_each_body_and_the_whole_part() {
        // Bounds describe the emitted render positions in part-local space.
        // The part query must union every body, including negative minima,
        // while an empty render body has no geometric bounds.
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part("two-bodies", two_body_recipe())
            .unwrap();
        let compiled = PartCompiler::new()
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        let compiled = compiled.part(part).unwrap();

        assert_eq!(
            compiled.bodies[0].bounds(),
            Some(Aabb3 {
                min: [0.0, 0.0, 0.0],
                max: [40.0, 20.0, 10.0],
            })
        );
        assert_eq!(
            compiled.bodies[1].bounds(),
            Some(Aabb3 {
                min: [-5.0, 30.0, -2.0],
                max: [35.0, 50.0, 8.0],
            })
        );
        assert_eq!(
            compiled.bounds(),
            Some(Aabb3 {
                min: [-5.0, 0.0, -2.0],
                max: [40.0, 50.0, 10.0],
            })
        );

        let empty = CompiledBody {
            tri: TriMesh::default(),
            regions: Vec::new(),
            extraction: ExtractStats::default(),
        };
        assert_eq!(empty.bounds(), None);
    }

    #[test]
    fn carried_attributes_on_recipe_parts_are_reported_missing() {
        // Constructive evaluation does not carry caller-defined layers yet,
        // so a request resolves to its missing value and is counted.
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", prism_recipe(40.0)).unwrap();
        let mut compiler = PartCompiler::new();
        let policy = CompilePolicy {
            attributes: alloc::vec![ExtractAttribute::new(
                exedra_mesh::attr::CORNER_COLOR,
                [1.0; 4]
            )],
            ..CompilePolicy::default()
        };
        let compiled = compiler.compile_parts(&asm, &policy).unwrap();
        let body = &compiled.part(part).unwrap().bodies[0];
        assert_eq!(body.extraction.missing_attribute_layers, 1);
        assert_eq!(
            body.extraction.attribute_fallback_count,
            body.tri.positions.len() as u64
        );
        assert_eq!(compiler.counters().missing_attribute_layers, 1);
        assert_eq!(
            compiler.counters().attribute_fallbacks,
            body.tri.positions.len() as u64
        );
        let _ = compiler.compile_parts(&asm, &policy).unwrap();
        assert_eq!(
            compiler.counters().missing_attribute_layers,
            1,
            "cache hits add nothing"
        );
    }

    #[test]
    fn failed_compilation_preserves_the_last_successful_invalidation_handles() {
        let policy = CompilePolicy::default();
        let mut compiler = PartCompiler::new();
        let mut source = Assembly::new();
        let first = source.add_recipe_part("first", prism_recipe(10.0)).unwrap();
        let second = source
            .add_recipe_part("second", prism_recipe(20.0))
            .unwrap();
        let before = compiler.compile_parts(&source, &policy).unwrap();

        let mut failing = Assembly::new();
        failing
            .add_recipe_part("second", prism_recipe(20.0))
            .unwrap();
        failing.add_recipe_part("new", prism_recipe(30.0)).unwrap();
        let refused = failing
            .add_recipe_part("refused", refused_open_shell_recipe())
            .unwrap();
        assert!(matches!(
            compiler.compile_parts(&failing, &policy),
            Err(CompileError::NoGeometry { part, .. }) if part == refused
        ));
        assert_eq!(compiler.counters().parts_compiled, 3);
        assert_eq!(compiler.cached_entries(), 3);

        compiler.mark_part_changed(first);
        let after = compiler.compile_parts(&source, &policy).unwrap();
        assert!(
            !Arc::ptr_eq(before.part(first).unwrap(), after.part(first).unwrap()),
            "invalidation still addresses the last successful assembly"
        );
        assert!(
            Arc::ptr_eq(before.part(second).unwrap(), after.part(second).unwrap()),
            "a failed attempt must not replace the meaning of reused handles"
        );
        assert_eq!(compiler.counters().parts_compiled, 4);
        assert_eq!(compiler.counters().cache_evictions, 1);
    }

    #[test]
    fn error_diagnostic_cannot_compile_as_an_empty_part() {
        // Stretching a crossing open shell is a supported, deterministic
        // refusal: evaluation emits an Error diagnostic and no body. Assembly
        // compilation must not turn that explicit refusal into a successful,
        // silently omitted instance.
        let mut asm = Assembly::new();
        let part = asm
            .add_recipe_part("refused", refused_open_shell_recipe())
            .unwrap();
        asm.add_instance(None, "omitted", part, Placement3::IDENTITY)
            .unwrap();

        let error = PartCompiler::new()
            .compile_parts(&asm, &CompilePolicy::default())
            .expect_err("an error diagnostic with no geometry must fail compilation");
        let CompileError::NoGeometry {
            part: rejected,
            part_key,
            report,
        } = error
        else {
            panic!("expected the report-bearing no-geometry error");
        };
        assert_eq!(rejected, part);
        assert_eq!(part_key, "refused");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "eval.stretch.open_shell")
        );
    }

    #[test]
    fn topology_boolean_refusal_survives_compilation_and_cache_reuse() {
        use exedra_constructive::ir::{CsgOp, PrimitiveSpec};

        for partial in [false, true] {
            let mut builder = RecipeBuilder::new();
            let slots = [builder.material_slot("a"), builder.material_slot("b")];
            let boxes: Vec<_> = slots
                .into_iter()
                .enumerate()
                .map(|(index, slot)| {
                    builder
                        .with_material(slot)
                        .add(NodeKind::Primitive {
                            spec: PrimitiveSpec::Box { size: [1.0; 3] },
                            placement: if index == 0 {
                                Placement3::IDENTITY
                            } else {
                                Placement3::translate(1.0, 1.0, 0.0)
                            },
                        })
                        .unwrap()
                })
                .collect();
            let boolean = builder
                .add(NodeKind::Csg {
                    op: CsgOp::Union,
                    operands: boxes.to_vec(),
                })
                .unwrap();
            let root = if partial {
                builder
                    .add(NodeKind::Group {
                        children: alloc::vec![boxes[0], boolean],
                    })
                    .unwrap()
            } else {
                boolean
            };
            let mut assembly = Assembly::new();
            let part = assembly
                .add_recipe_part("mixed", builder.finish(root).unwrap())
                .unwrap();
            let mut compiler = PartCompiler::new();
            for _ in 0..2 {
                let result = compiler.compile_parts(&assembly, &CompilePolicy::default());
                let report = if partial {
                    let compiled = result.unwrap();
                    assert_eq!(compiled.part(part).unwrap().bodies.len(), 1);
                    compiled.report(part).unwrap().clone()
                } else {
                    let CompileError::NoGeometry { report, .. } = result.unwrap_err() else {
                        panic!("expected report-bearing refusal");
                    };
                    (*report).clone()
                };
                assert!(
                    report
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "eval.csg.unsupported")
                );
            }
        }
    }

    #[test]
    fn tolerance_budget_failure_keeps_its_typed_payload_through_compilation() {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::circle(1.0).expect("circle"));
        let source = builder.source_ref("circle/profile-extrusion");
        let node = builder
            .with_source(source)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("extrude");
        let recipe = builder.finish(node).expect("recipe");
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part("tight-circle", recipe)
            .expect("part");
        assembly
            .add_instance(None, "tight-circle", part, Placement3::IDENTITY)
            .expect("instance");
        let mut policy = EvalPolicy::default();
        policy.discretize = DiscretizePolicy {
            chord_tolerance: 1.0e-9,
            min_arc_edges: 1,
            max_segment_edges: 4,
        };

        let error = PartCompiler::new()
            .compile_parts(&assembly, &policy.into())
            .expect_err("insufficient tolerance budget must fail compilation");
        let CompileError::Evaluate {
            part: failed_part,
            part_key,
            error,
        } = error
        else {
            panic!("expected evaluation failure");
        };
        assert_eq!(failed_part, part);
        assert_eq!(part_key, "tight-circle");
        assert!(matches!(*error,
            EvalError {
                source,
                loft_sections: None,
                node: failed_node,
                error: TessellateError::Discretize(
                    DiscretizeError::ToleranceBudgetExceeded { required, maximum: 4 }
                ),
            } if failed_node == node && required > 4
                && source.as_deref() == Some("circle/profile-extrusion")
        ));
    }

    #[test]
    fn partial_geometry_retains_its_report_across_cache_hits() {
        // A group may deliberately preserve usable geometry even when one
        // child is refused. Compilation succeeds, but the refusal—including
        // diagnostics of every severity—must remain inspectable and must not
        // change when the content-addressed geometry cache is hit.
        let mut asm = Assembly::new();
        let part = asm
            .add_recipe_part("partial", partially_supported_recipe())
            .unwrap();
        asm.add_instance(None, "partial", part, Placement3::IDENTITY)
            .unwrap();
        let mut compiler = PartCompiler::new();

        let first = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        assert_eq!(first.part(part).unwrap().bodies.len(), 1);
        let first_report = first.report(part).expect("recipe report");
        assert!(
            first_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "eval.stretch.open_shell")
        );

        let again = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let cached_report = again.report(part).expect("cached recipe report");
        assert_eq!(cached_report, first_report);
        assert!(
            core::ptr::eq(cached_report, first_report),
            "the report must be cached with its compiled geometry"
        );
    }

    #[test]
    fn double_run_determinism() {
        let asm = n_instance_assembly(2);
        let mut c1 = PartCompiler::new();
        let mut c2 = PartCompiler::new();
        let a = c1.compile_parts(&asm, &CompilePolicy::default()).unwrap();
        let b = c2.compile_parts(&asm, &CompilePolicy::default()).unwrap();
        let (pa, pb) = (a.part(PartId(0)).unwrap(), b.part(PartId(0)).unwrap());
        assert_eq!(pa.fingerprint, pb.fingerprint);
        assert_eq!(pa.bodies.len(), pb.bodies.len());
        for (body_a, body_b) in pa.bodies.iter().zip(&pb.bodies) {
            assert_eq!(body_a.tri, body_b.tri);
            assert_eq!(body_a.regions, body_b.regions);
        }
    }

    #[test]
    fn region_ranges_cover_all_triangles_contiguously() {
        let asm = n_instance_assembly(1);
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let body = &compiled.part(PartId(0)).unwrap().bodies[0];
        let mut cursor = 0;
        let mut prev_region = None;
        for range in &body.regions {
            assert_eq!(range.start, cursor, "ranges must be contiguous");
            assert_eq!(range.count % 3, 0, "ranges must be whole triangles");
            if let Some(prev) = prev_region {
                assert!(range.region > prev, "regions must be ascending and unique");
            }
            prev_region = Some(range.region);
            cursor += range.count;
        }
        assert_eq!(cursor as usize, body.tri.indices.len());
        // A rect prism has both caps and four wall segments.
        assert!(body.regions.len() >= 6, "expected caps plus four walls");
    }

    #[test]
    fn policy_change_misses_cache() {
        let asm = n_instance_assembly(1);
        let mut compiler = PartCompiler::new();
        compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let mut coarse = EvalPolicy::default();
        coarse.discretize.chord_tolerance = 0.5;
        compiler.compile_parts(&asm, &coarse.into()).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert_ne!(
            policy_fingerprint(&CompilePolicy::default()),
            policy_fingerprint(&coarse.into())
        );
    }

    #[test]
    fn uv_source_participates_in_policy_fingerprint() {
        let custom_only = CompilePolicy::default();
        let projected = |scale| CompilePolicy {
            uvs: UvSource::CustomOrBoxProjected { scale },
            ..CompilePolicy::default()
        };
        let fingerprints = [
            policy_fingerprint(&custom_only),
            policy_fingerprint(&projected(1.0)),
            policy_fingerprint(&projected(2.0)),
        ];
        assert_ne!(fingerprints[0], fingerprints[1], "variant changes identity");
        assert_ne!(fingerprints[1], fingerprints[2], "scale changes identity");
        assert_ne!(fingerprints[0], fingerprints[2]);
        assert_eq!(policy_fingerprint(&projected(1.0)), fingerprints[1]);

        // The policy is a cache key: a UV change compiles again.
        let asm = n_instance_assembly(1);
        let mut compiler = PartCompiler::new();
        compiler.compile_parts(&asm, &custom_only).unwrap();
        compiler.compile_parts(&asm, &projected(1.0)).unwrap();
        compiler.compile_parts(&asm, &projected(1.0)).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert_eq!(compiler.counters().cache_hits, 1);
    }

    #[test]
    fn box_projection_gives_every_range_uv_coverage() {
        let asm = n_instance_assembly(1);
        let mut compiler = PartCompiler::new();
        let plain = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let projected = compiler
            .compile_parts(
                &asm,
                &CompilePolicy {
                    uvs: UvSource::CustomOrBoxProjected { scale: 1.0 },
                    ..CompilePolicy::default()
                },
            )
            .unwrap();
        let plain_part = plain.part(PartId(0)).unwrap();
        let projected_part = projected.part(PartId(0)).unwrap();
        assert_eq!(plain_part.bodies.len(), projected_part.bodies.len());
        for (plain_body, projected_body) in plain_part.bodies.iter().zip(&projected_part.bodies) {
            assert!(!projected_body.regions.is_empty());
            assert!(
                plain_body.regions.iter().all(|range| !range.has_uvs),
                "the generated prism has no authored UVs"
            );
            assert!(
                projected_body.regions.iter().all(|range| range.has_uvs),
                "projection covers every corner"
            );
            assert_eq!(plain_body.regions.len(), projected_body.regions.len());
            for (a, b) in plain_body.regions.iter().zip(&projected_body.regions) {
                assert_eq!(
                    (a.region, a.material_slot, a.start, a.count),
                    (b.region, b.material_slot, b.start, b.count),
                    "the UV policy changes coverage, not range layout"
                );
            }
            assert_eq!(plain_body.tri.indices, projected_body.tri.indices);
            assert_eq!(plain_body.tri.positions, projected_body.tri.positions);
            assert!(plain_body.tri.uvs.iter().all(|uv| *uv == [0.0, 0.0]));
            assert!(
                projected_body
                    .tri
                    .uvs
                    .iter()
                    .all(|uv| uv.iter().all(|v| v.is_finite()))
            );
            assert!(projected_body.tri.uvs.iter().any(|uv| *uv != [0.0, 0.0]));
        }
    }

    #[test]
    fn box_projection_keeps_authored_uvs_and_nonfinite_authored_values_stay_uncovered() {
        let mut builder = exedra_mesh::MeshBuilder::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            builder.push_vertex(p);
        }
        builder.add_face(&[0, 1, 2]).unwrap();
        let mut mesh = builder.build().unwrap().mesh;
        let corners: Vec<_> = mesh.face_loop(mesh.faces().next().unwrap()).collect();
        let mut edit = mesh.edit();
        exedra_mesh::op::set_corner_uv(&mut edit, corners[0], [5.0, 6.0]).unwrap();
        let _: () = edit.finish();
        let policy = CompilePolicy {
            uvs: UvSource::CustomOrBoxProjected { scale: 1.0 },
            ..CompilePolicy::default()
        };

        let mut asm = Assembly::new();
        let part = asm.add_baked_part("tri", mesh.clone(), &[]).unwrap();
        asm.add_instance(None, "tri", part, Placement3::IDENTITY)
            .unwrap();
        let compiled = PartCompiler::new().compile_parts(&asm, &policy).unwrap();
        let body = &compiled.part(part).unwrap().bodies[0];
        assert!(body.regions[0].has_uvs);
        assert!(body.tri.uvs.contains(&[5.0, 6.0]), "authored UV survives");

        let mut edit = mesh.edit();
        exedra_mesh::op::set_corner_uv(&mut edit, corners[1], [f32::NAN, 0.0]).unwrap();
        let _: () = edit.finish();
        let mut asm = Assembly::new();
        let part = asm.add_baked_part("tri", mesh, &[]).unwrap();
        asm.add_instance(None, "tri", part, Placement3::IDENTITY)
            .unwrap();
        let compiled = PartCompiler::new().compile_parts(&asm, &policy).unwrap();
        assert!(
            !compiled.part(part).unwrap().bodies[0].regions[0].has_uvs,
            "projection fills missing UVs, never repairs authored ones"
        );
    }

    #[test]
    fn slot_rebinding_compiles_nothing() {
        let mut asm = n_instance_assembly(3);
        let mut compiler = PartCompiler::new();
        compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let compiled_before = compiler.counters().parts_compiled;
        // Rebind materials on every instance: pure structure.
        let part = asm.part_by_key("panel").unwrap();
        let _ = part;
        for id in 0..3 {
            // The test recipe declares no slots, so bind_material errors —
            // metadata stands in for a structure-only edit here; slot
            // rebinding is covered in the flatten tests with real slots.
            asm.set_metadata(InstanceId(id), "note", "edited").unwrap();
        }
        compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, compiled_before);
        assert_eq!(compiler.counters().cache_hits, 1);
    }

    #[test]
    fn replacing_one_part_evicts_only_it() {
        let mut asm = Assembly::new();
        let a = asm.add_recipe_part("a", prism_recipe(40.0)).unwrap();
        let b = asm.add_recipe_part("b", prism_recipe(60.0)).unwrap();
        asm.add_instance(None, "ia", a, Placement3::IDENTITY)
            .unwrap();
        asm.add_instance(None, "ib", b, Placement3::IDENTITY)
            .unwrap();
        let mut compiler = PartCompiler::new();
        compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);

        asm.replace_part_source(a, PartSource::Recipe(prism_recipe(45.0)))
            .unwrap();
        compiler.mark_part_changed(a);
        compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        // Part a: evicted + recompiled (content changed). Part b: pure hit.
        assert_eq!(compiler.counters().cache_evictions, 1);
        assert_eq!(compiler.counters().parts_compiled, 3);
        assert_eq!(compiler.counters().cache_hits, 1);
    }

    #[test]
    fn baked_parts_fingerprint_and_compile() {
        let recipe = prism_recipe(40.0);
        let evaluation = evaluate(&recipe, &EvalPolicy::default()).unwrap();
        let mesh = evaluation
            .bodies
            .into_iter()
            .next()
            .unwrap()
            .body
            .mesh
            .clone();
        let mut asm = Assembly::new();
        let part = asm.add_baked_part("baked", mesh, &["shell"]).unwrap();
        asm.add_instance(None, "i", part, Placement3::IDENTITY)
            .unwrap();
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let entry = compiled.part(part).unwrap();
        assert_eq!(entry.bodies.len(), 1);
        assert!(entry.triangle_count() > 0);
        assert!(
            compiled.report(part).is_none(),
            "baked parts do not have constructive reports"
        );
        // Second run finds the same content and reuses the compiled mesh.
        compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(compiler.counters().cache_hits, 1);
    }
}
