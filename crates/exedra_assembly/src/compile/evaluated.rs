// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{CompiledParts, PartId, RenderList};
use alloc::sync::Arc;
use exedra_compile::{BodyLookupError, GeometrySnapshot, SnapshotBody};

/// Shared geometry plus captured assembly occurrence placements and appearance.
/// Geometry queries delegate to the scene-independent snapshot; selections keep
/// its scope and remain valid after cache eviction. A fresh compilation creates
/// a fresh selection scope even when the geometry is reused.
#[derive(Clone, Debug)]
pub struct EvaluationSnapshot {
    geometry: GeometrySnapshot,
    compiled: CompiledParts,
    render: Arc<RenderList>,
}
impl EvaluationSnapshot {
    pub(super) fn new(
        geometry: GeometrySnapshot,
        compiled: CompiledParts,
        render: Arc<RenderList>,
    ) -> Self {
        Self {
            geometry,
            compiled,
            render,
        }
    }
    /// Scene-independent geometry, queries and extraction results.
    #[must_use]
    pub fn geometry(&self) -> &GeometrySnapshot {
        &self.geometry
    }
    /// Compiled geometry in the assembly's local part ordering.
    #[must_use]
    pub fn compiled(&self) -> &CompiledParts {
        &self.compiled
    }
    /// Captured occurrence placements and resolved material bindings.
    #[must_use]
    pub fn render(&self) -> &RenderList {
        &self.render
    }
    /// Resolves a unique producing source label within a local part.
    /// # Errors
    /// Reports unknown parts, missing labels and ambiguity without choosing one.
    pub fn body_by_source(
        &self,
        part: PartId,
        source: &str,
    ) -> Result<SnapshotBody, BodyLookupError> {
        self.geometry.body_by_source(part, source)
    }
    /// Retains an immutable body by snapshot-local part and body indices.
    #[must_use]
    pub fn body(&self, part: PartId, body: u32) -> Option<SnapshotBody> {
        self.geometry.body(part, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Assembly, CompilePolicy, PartCompiler, SnapshotWorkplane, StaleSelection};
    use alloc::vec::Vec;
    use exedra_constructive::workplane::WorkplaneError;
    use exedra_constructive::{
        builders,
        ir::{CapMode, NodeKind, Placement3, RecipeBuilder},
        tessellate::REGION_CAP_END,
    };
    use exedra_constructive::{
        clearance::BoundaryPolicy,
        ir::Plane3,
        section::SectionPolicy,
        workplane::{
            SurfaceInventoryPolicy, WorkplaneAttachment, WorkplaneFailure, WorkplanePolicy,
            WorkplaneSelection,
        },
    };

    fn naming_context(duplicate: bool, retained: bool) -> Assembly {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(2.0, 3.0).unwrap());
        let label = b.source_ref("panel");
        let kind = NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        };
        let first = b.with_source(label).add(kind.clone()).unwrap();
        let second = if duplicate {
            b.with_source(label).add(kind).unwrap()
        } else {
            first
        };
        let second = if retained {
            b.add(NodeKind::OnWorkplane {
                support: second,
                child: first,
                attachment: named_cap(),
            })
            .unwrap()
        } else {
            second
        };
        let root = b
            .add(NodeKind::Group {
                children: alloc::vec![first, second],
            })
            .unwrap();
        let mut assembly = Assembly::new();
        assembly
            .add_recipe_part("panel", b.finish(root).unwrap())
            .unwrap();
        assembly
    }

    fn named_cap() -> WorkplaneAttachment {
        WorkplaneAttachment {
            surface: exedra_constructive::workplane::SurfaceSelector::SourceEndCap("panel".into()),
            anchor: [0.0; 3],
            projection: [0.0, 0.0, 1.0],
            x_direction: [1.0, 0.0, 0.0],
        }
    }

    #[test]
    fn naming_context_separates_snapshot_cache_entries_in_both_directions() {
        let query = |snapshot: &EvaluationSnapshot| {
            snapshot
                .body(PartId(0), 0)
                .unwrap()
                .resolve_attachment(&named_cap(), &WorkplanePolicy::default())
                .map(|_| ())
                .map_err(|failure| failure.kind)
        };
        for first_duplicate in [false, true] {
            let mut compiler = PartCompiler::new();
            let policy = CompilePolicy::default();
            let first = compiler
                .compile_snapshot(&naming_context(first_duplicate, false), &policy)
                .unwrap();
            let expected_first = if first_duplicate {
                Err(WorkplaneError::AmbiguousSelection)
            } else {
                Ok(())
            };
            assert_eq!(query(&first), expected_first);
            let second = compiler
                .compile_snapshot(&naming_context(!first_duplicate, false), &policy)
                .unwrap();
            assert_eq!(
                query(&second),
                if first_duplicate {
                    Ok(())
                } else {
                    Err(WorkplaneError::AmbiguousSelection)
                }
            );
            assert_eq!(
                query(&first),
                expected_first,
                "old snapshots retain their naming context"
            );
            let again = compiler
                .compile_snapshot(&naming_context(first_duplicate, false), &policy)
                .unwrap();
            assert_eq!(query(&again), expected_first);
            assert_eq!(compiler.counters().cache_hits, 1);
            compiler.clear_cache();
            assert_eq!(query(&first), expected_first);
        }
    }

    #[test]
    fn naming_context_cannot_skip_retained_attachment_failure_on_cache_hit() {
        for retain in [false, true] {
            let mut compiler = PartCompiler::new();
            let policy = CompilePolicy::default();
            let compile = |compiler: &mut PartCompiler, duplicate| {
                let assembly = naming_context(duplicate, true);
                if retain {
                    compiler.compile_snapshot(&assembly, &policy).map(|_| ())
                } else {
                    compiler.compile_parts(&assembly, &policy).map(|_| ())
                }
            };
            compile(&mut compiler, false).unwrap();
            let error = compile(&mut compiler, true).unwrap_err();
            let crate::CompileError::Evaluate { error, .. } = error else {
                panic!("expected evaluation failure");
            };
            assert!(matches!(
                error.error,
                exedra_constructive::tessellate::TessellateError::Attachment(WorkplaneFailure {
                    kind: WorkplaneError::AmbiguousSelection,
                    ..
                })
            ));
            compile(&mut compiler, false).unwrap();
        }
    }

    #[test]
    fn surface_inspection_diagnoses_and_recovers_without_private_geometry_access() {
        use exedra_constructive::{
            ir::CsgOp,
            workplane::{SurfaceSelector, WorkplaneEvidence},
        };
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(4.0, 3.0).unwrap());
        let mut operands = Vec::new();
        for (source, x) in [("left", 0.0), ("right", 6.0)] {
            let source = b.source_ref(source);
            operands.push(
                b.with_source(source)
                    .add(NodeKind::Extrude {
                        profile,
                        placement: Placement3::translate(x, 0.0, 0.0),
                        height: 2.0,
                        caps: CapMode::Both,
                    })
                    .unwrap(),
            );
        }
        let source = b.source_ref("panels");
        let root = b
            .with_source(source)
            .add(NodeKind::Csg {
                op: CsgOp::Union,
                operands,
            })
            .unwrap();
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part("panels", b.finish(root).unwrap())
            .unwrap();
        let mut compiler = PartCompiler::new();
        let snapshot = compiler
            .compile_snapshot(&assembly, &CompilePolicy::default())
            .unwrap();
        let body = snapshot.body_by_source(part, "panels").unwrap();
        let attachment = WorkplaneAttachment {
            surface: SurfaceSelector::EndCap,
            anchor: [6.5, 0.5, 0.0],
            projection: [0.0, 0.0, 1.0],
            x_direction: [1.0, 0.0, 0.0],
        };
        let failure = body
            .resolve_attachment(&attachment, &WorkplanePolicy::default())
            .unwrap_err();
        assert_eq!(failure.selection, attachment.surface.selection());
        assert!(
            matches!(&failure.evidence,WorkplaneEvidence::Disconnected {representatives} if representatives.len()==2)
        );
        let inventory = body
            .inspect_surfaces(&SurfaceInventoryPolicy::default())
            .unwrap();
        inventory.check(&body).unwrap();
        let candidate = inventory
            .inventory()
            .entries()
            .iter()
            .find(|entry| entry.selector == SurfaceSelector::SourceEndCap("right".into()))
            .unwrap();
        candidate.status.as_ref().unwrap();
        assert_eq!(candidate.patches.len(), 1);
        assert!(
            candidate
                .origins
                .iter()
                .any(|origin| origin.source.as_deref() == Some("right"))
        );
        let explicit = WorkplaneAttachment {
            surface: candidate.selector.clone(),
            ..attachment
        };
        let plane = inventory
            .body()
            .resolve_attachment(&explicit, &WorkplanePolicy::default())
            .unwrap();
        assert_eq!(plane.plane().to_body([0.0; 3]), [6.5, 0.5, 2.0]);
        let patch = plane.planar_patch(&BoundaryPolicy::default()).unwrap();
        assert!(patch.circle_clearance([0.0; 2], 0.1).unwrap().clearance > 0.39);
        let next = compiler
            .compile_snapshot(&assembly, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().cache_hits, 1);
        let next_body = next.body_by_source(part, "panels").unwrap();
        assert_eq!(inventory.check(&next_body), Err(StaleSelection));
        compiler.clear_cache();
        drop(snapshot);
        inventory.check(inventory.body()).unwrap();
        inventory
            .body()
            .resolve_attachment(&explicit, &WorkplanePolicy::default())
            .unwrap();
    }

    fn assembly(height: f64) -> Assembly {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::rect_from_corner(2.0, 3.0).unwrap());
        let label = builder.source_ref("panel");
        let node = builder
            .with_source(label)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height,
                caps: CapMode::Both,
            })
            .unwrap();
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part("panel", builder.finish(node).unwrap())
            .unwrap();
        assembly
            .add_instance(None, "left", part, Placement3::translate(10.0, 0.0, 0.0))
            .unwrap();
        assembly
            .add_instance(None, "right", part, Placement3::translate(20.0, 0.0, 0.0))
            .unwrap();
        assembly
    }

    #[test]
    fn render_and_queries_share_a_surviving_snapshot_and_selections_are_scoped() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<EvaluationSnapshot>();
        send_sync::<SnapshotWorkplane>();
        let mut compiler = PartCompiler::new();
        let policy = CompilePolicy::default();
        let first = compiler.compile_snapshot(&assembly(4.0), &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(first.render().items.len(), 2);
        assert_eq!(first.render().items[0].world.rows[0][3], 10.0);
        assert_eq!(first.render().items[1].world.rows[0][3], 20.0);
        let body = first.body(PartId(0), 0).unwrap();
        assert_eq!(body.source(), Some("panel"));
        assert!(body.node().is_some());
        assert!(first.body(PartId(0), 1).is_none());
        assert!(first.body(PartId(1), 0).is_none());
        let plane = body
            .workplane(
                WorkplaneSelection::Region(REGION_CAP_END),
                [0.0, 0.0, 4.0],
                [1.0, 0.0, 0.0],
                &WorkplanePolicy::default(),
            )
            .unwrap();
        assert_eq!(plane.check(&body), Ok(()));
        assert_eq!(
            plane.check(&first.clone().body(PartId(0), 0).unwrap()),
            Ok(())
        );
        let section = body
            .section(
                Plane3 {
                    normal: [0.0, 0.0, 1.0],
                    distance: 1.75,
                },
                &SectionPolicy::default(),
            )
            .unwrap();
        assert_eq!(section.regions.len(), 1);
        let second = compiler.compile_snapshot(&assembly(4.0), &policy).unwrap();
        let second_body = second.body(PartId(0), 0).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert!(core::ptr::eq(body.geometry(), second_body.geometry()));
        assert!(Arc::ptr_eq(
            first.compiled().part(PartId(0)).unwrap(),
            second.compiled().part(PartId(0)).unwrap()
        ));
        assert_eq!(plane.check(&second_body), Err(StaleSelection));
        let changed = compiler.compile_snapshot(&assembly(5.0), &policy).unwrap();
        assert_eq!(
            plane.check(&changed.body(PartId(0), 0).unwrap()),
            Err(StaleSelection)
        );
        assert!(first.compiled().validate_for(&assembly(5.0)).is_err());
        // Every rendered point comes from this exact retained geometry.
        for position in &first.compiled().part(PartId(0)).unwrap().bodies[0]
            .tri
            .positions
        {
            assert!(
                body.geometry()
                    .mesh
                    .vertices()
                    .any(|v| body.geometry().mesh.vertex_position(v) == Some(position))
            );
        }
        compiler.clear_cache();
        drop(compiler);
        drop(first);
        assert_eq!(plane.check(&body), Ok(()));
        assert_eq!(
            body.section(
                Plane3 {
                    normal: [0.0, 0.0, 1.0],
                    distance: 1.75
                },
                &SectionPolicy::default()
            )
            .unwrap(),
            section
        );
    }

    #[test]
    fn baked_meshes_keep_imported_provenance_and_aliases_have_distinct_selection_identity() {
        let mesh = exedra_constructive::tessellate::tessellate_extrude(
            &builders::rect_from_corner(2.0, 3.0).unwrap(),
            &Placement3::IDENTITY,
            4.0,
            CapMode::Both,
            &exedra_constructive::tessellate::EvalPolicy::default(),
        )
        .unwrap()
        .mesh;
        let mut assembly = Assembly::new();
        let first = assembly.add_baked_part("first", mesh.clone(), &[]).unwrap();
        let alias = assembly.add_baked_part("alias", mesh, &[]).unwrap();
        let mut compiler = PartCompiler::new();
        let snapshot = compiler
            .compile_snapshot(&assembly, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        let body = snapshot.body(first, 0).unwrap();
        let other = snapshot.body(alias, 0).unwrap();
        assert!(core::ptr::eq(body.geometry(), other.geometry()));
        assert_eq!(body.node(), None);
        assert_eq!(body.source(), None);
        assert!(
            body.geometry()
                .source_map
                .face_features()
                .iter()
                .all(|feature| *feature == exedra_constructive::tessellate::Feature::Imported)
        );
        let plane = body
            .workplane(
                WorkplaneSelection::Region(REGION_CAP_END),
                [0.0, 0.0, 4.0],
                [1.0, 0.0, 0.0],
                &WorkplanePolicy::default(),
            )
            .unwrap();
        assert_eq!(plane.check(&other), Err(StaleSelection));
        compiler.mark_part_changed(alias);
        compiler
            .compile_snapshot(&assembly, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert_eq!(plane.check(&body), Ok(()));
    }

    #[test]
    fn semantic_attachment_and_clearance_survive_edits_with_explicit_stale_and_missing_results() {
        use exedra_constructive::{
            clearance::ClearanceDecision,
            ir::Plane3,
            workplane::{SurfaceSelector, WorkplaneAttachment},
        };
        let attachment = WorkplaneAttachment {
            surface: SurfaceSelector::SourceEndCap("panel/origin".into()),
            anchor: [0.75, 1.0, 0.0],
            projection: [0.0, 0.0, 1.0],
            x_direction: [1.0, 0.0, 0.0],
        };
        let make = |width, duplicate| {
            let mut b = RecipeBuilder::new();
            let label = b.source_ref("panel/origin");
            let profile = b.add_profile(builders::rect_from_corner(width, 3.0).unwrap());
            let support = b
                .with_source(label)
                .add(NodeKind::ExtrudeToPlane {
                    profile,
                    placement: Placement3::IDENTITY,
                    plane: Plane3 {
                        normal: [-0.2, 0.0, 1.0],
                        distance: 1.0,
                    },
                })
                .unwrap();
            let hole_profile = b.add_profile(builders::rect_from_corner(0.2, 0.2).unwrap());
            let hole = b
                .add(NodeKind::Extrude {
                    profile: hole_profile,
                    placement: Placement3::translate(0.1, 2.0, -1.0),
                    height: 4.0,
                    caps: CapMode::Both,
                })
                .unwrap();
            let label = b.source_ref("panel");
            let support = b
                .with_source(label)
                .add(NodeKind::Csg {
                    op: exedra_constructive::ir::CsgOp::Difference,
                    operands: alloc::vec![support, hole],
                })
                .unwrap();
            let root = if duplicate {
                b.add(NodeKind::Group {
                    children: alloc::vec![support, support],
                })
                .unwrap()
            } else {
                support
            };
            let mut assembly = Assembly::new();
            assembly
                .add_recipe_part("panel", b.finish(root).unwrap())
                .unwrap();
            assembly
        };
        let mut compiler = PartCompiler::new();
        let mut previous: Option<SnapshotWorkplane> = None;
        for (width, expected) in [
            (4.0, ClearanceDecision::Satisfied),
            (1.0, ClearanceDecision::Violated),
        ] {
            let snapshot = compiler
                .compile_snapshot(&make(width, false), &CompilePolicy::default())
                .unwrap();
            let body = snapshot.body_by_source(PartId(0), "panel").unwrap();
            if let Some(old) = previous {
                assert_eq!(old.check(&body), Err(StaleSelection));
            }
            let plane = body
                .resolve_attachment(&attachment, &WorkplanePolicy::default())
                .unwrap();
            assert!((plane.plane().frame().rows[2][3] - 1.15).abs() < 1e-6);
            let patch = plane.planar_patch(&BoundaryPolicy::default()).unwrap();
            let clearance = patch.circle_clearance([0.0, 0.0], 0.2).unwrap();
            assert_eq!(clearance.classify(0.1, 1e-5), Ok(expected));
            assert!(matches!(
                snapshot.body_by_source(PartId(0), "absent"),
                Err(BodyLookupError::MissingSource { .. })
            ));
            assert!(matches!(
                snapshot.body_by_source(PartId(1), "panel"),
                Err(BodyLookupError::UnknownPart { .. })
            ));
            previous = Some(plane);
        }
        let ambiguous = compiler
            .compile_snapshot(&make(4.0, true), &CompilePolicy::default())
            .unwrap();
        assert!(matches!(
            ambiguous.body_by_source(PartId(0), "panel"),
            Err(BodyLookupError::AmbiguousSource { matches: 2, .. })
        ));
        compiler.clear_cache();
        assert!(
            previous
                .unwrap()
                .planar_patch(&BoundaryPolicy::default())
                .is_ok()
        );
    }

    #[test]
    fn render_only_compilation_does_not_retain_topology_and_upgrade_is_explicit() {
        let mut compiler = PartCompiler::new();
        let policy = CompilePolicy::default();
        let assembly = assembly(4.0);
        let render = compiler.compile_parts(&assembly, &policy).unwrap();
        assert!(compiler.geometry.retained_entries() == 0);
        assert_eq!(compiler.counters().parts_compiled, 1);
        let snapshot = compiler.compile_snapshot(&assembly, &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert!(compiler.geometry.retained_entries() == compiler.cached_entries());
        assert_eq!(
            render.part(PartId(0)).unwrap().bodies[0].tri.positions,
            snapshot.compiled().part(PartId(0)).unwrap().bodies[0]
                .tri
                .positions
        );
        compiler.compile_parts(&assembly, &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
    }
}
