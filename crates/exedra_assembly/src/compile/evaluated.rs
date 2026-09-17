// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{CompiledParts, PartId, RenderList};
use alloc::{rc::Rc, string::String, sync::Arc, vec::Vec};
use exedra_constructive::{
    clearance::{BoundaryPolicy, ClearanceError, PlanarPatch},
    evaluate::PlacedBody,
    ir::{NodeId, Plane3, Recipe, SlotId},
    section::{PlaneSection, SectionError, SectionPolicy, section_body},
    tessellate::TessellatedBody,
    workplane::{
        Workplane, WorkplaneAttachment, WorkplaneError, WorkplanePolicy, WorkplaneSelection,
        face_workplane,
    },
};

#[derive(Debug)]
struct EvaluatedBody {
    node: Option<NodeId>,
    source: Option<String>,
    material: Option<SlotId>,
    body: TessellatedBody,
}

#[derive(Debug)]
pub(super) struct EvaluatedPart {
    bodies: Vec<EvaluatedBody>,
}

impl EvaluatedPart {
    pub(super) fn from_evaluation(bodies: Vec<PlacedBody>, recipe: &Recipe) -> Self {
        Self {
            bodies: bodies
                .into_iter()
                .map(|placed| EvaluatedBody {
                    node: Some(placed.node),
                    source: recipe.source_of(placed.node).map(String::from),
                    material: placed.material,
                    // Move unique meshes out of the single-threaded evaluator. A
                    // repeated occurrence may share its Rc, requiring a mesh copy,
                    // but never another evaluation or tessellation.
                    body: Rc::try_unwrap(placed.body).unwrap_or_else(|body| (*body).clone()),
                })
                .collect(),
        }
    }

    pub(super) fn from_baked(mesh: &exedra_mesh::Mesh) -> Self {
        Self {
            bodies: alloc::vec![EvaluatedBody {
                node: None,
                source: None,
                material: None,
                body: TessellatedBody::from_imported_mesh(mesh.clone()),
            }],
        }
    }
}

/// An immutable assembly evaluation for both rendering and geometric inspection.
///
/// Bodies are in part-local coordinates, with recipe transforms already baked
/// in. [`Self::render`] freezes occurrence paths, world placements and resolved
/// materials from this compilation; body indices agree with its render items
/// and [`Self::compiled`]. No query evaluates or tessellates the recipe again.
///
/// Clones share topology and snapshot identity. A new compilation creates a
/// new selection scope, even on a cache hit. Snapshots and their body handles
/// survive cache eviction and are `Send + Sync`. Geometry may still carry
/// refusals or warnings: consult the compiled constructive reports.
#[derive(Clone, Debug)]
pub struct EvaluationSnapshot {
    compiled: CompiledParts,
    evaluated: Vec<Arc<EvaluatedPart>>,
    render: Arc<RenderList>,
    scope: Arc<()>,
}

impl EvaluationSnapshot {
    pub(super) fn new(
        compiled: CompiledParts,
        evaluated: Vec<Arc<EvaluatedPart>>,
        render: RenderList,
    ) -> Self {
        Self {
            compiled,
            evaluated,
            render: Arc::new(render),
            scope: Arc::new(()),
        }
    }

    /// Render buffers and evaluation reports produced from the retained bodies.
    #[must_use]
    pub fn compiled(&self) -> &CompiledParts {
        &self.compiled
    }

    /// Captured occurrence placements and material bindings for those buffers.
    #[must_use]
    pub fn render(&self) -> &RenderList {
        &self.render
    }

    /// Resolves one producing source label within a part of this snapshot.
    /// Use authored labels after parameter edits instead of retaining a previous
    /// body's numeric index. Equal labels are not silently resolved by order.
    ///
    /// # Errors
    /// Identifies an unknown part, absent source, or ambiguous source with its
    /// match count. The source is the producing recipe node's label, not a
    /// recursive search for labels erased by later geometry operations.
    pub fn body_by_source(
        &self,
        part: PartId,
        source: &str,
    ) -> Result<SnapshotBody, BodyLookupError> {
        let evaluated = self
            .evaluated
            .get(part.0 as usize)
            .ok_or(BodyLookupError::UnknownPart { part })?;
        let mut first = None;
        let mut count = 0;
        for (index, body) in evaluated.bodies.iter().enumerate() {
            if body.source.as_deref() == Some(source) {
                first.get_or_insert(crate::len_u32(index));
                count += 1;
            }
        }
        match (first, count) {
            (Some(index), 1) => Ok(self.body(part, index).expect("resolved snapshot body")),
            (None, _) => Err(BodyLookupError::MissingSource {
                part,
                source: String::from(source),
            }),
            _ => Err(BodyLookupError::AmbiguousSource {
                part,
                source: String::from(source),
                matches: count,
            }),
        }
    }

    /// Obtains a shared, immutable body by the same part/body indices used in
    /// render items. Invalid indices return `None`; baked meshes are supported
    /// with imported provenance and no producing recipe node.
    #[must_use]
    pub fn body(&self, part: PartId, body: u32) -> Option<SnapshotBody> {
        let evaluated = self.evaluated.get(part.0 as usize)?;
        evaluated.bodies.get(body as usize)?;
        Some(SnapshotBody {
            evaluated: Arc::clone(evaluated),
            scope: Arc::clone(&self.scope),
            part,
            body,
        })
    }
}

/// Shared body identity pinned to one evaluation snapshot.
///
/// The public geometry is borrowed immutably; raw face IDs and workplanes are
/// transient selections, not attachments to faces after parameter edits.
#[derive(Clone, Debug)]
pub struct SnapshotBody {
    evaluated: Arc<EvaluatedPart>,
    scope: Arc<()>,
    part: PartId,
    body: u32,
}
impl SnapshotBody {
    fn entry(&self) -> &EvaluatedBody {
        &self.evaluated.bodies[self.body as usize]
    }

    /// Assembly-local part index within the owning snapshot.
    #[must_use]
    pub fn part(&self) -> PartId {
        self.part
    }

    /// Body index within that part, also used by render extraction.
    #[must_use]
    pub fn index(&self) -> u32 {
        self.body
    }

    /// Mesh, topology, face materials and element provenance in part coordinates.
    #[must_use]
    pub fn geometry(&self) -> &TessellatedBody {
        &self.entry().body
    }

    /// Producing node in the original recipe, or `None` for baked geometry.
    #[must_use]
    pub fn node(&self) -> Option<NodeId> {
        self.entry().node
    }

    /// Captured author source label of the producing node.
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.entry().source.as_deref()
    }

    /// Effective authored face material slot before assembly binding resolution.
    #[must_use]
    pub fn material_for_face(&self, face: exedra_mesh::FaceId) -> Option<SlotId> {
        self.geometry()
            .face_materials
            .get(&face)
            .copied()
            .or(self.entry().material)
    }

    /// Sections this retained body using a plane and tolerances in part units.
    ///
    /// # Errors
    /// Same closed-mesh, contact and budget checks as [`section_body`].
    pub fn section(
        &self,
        plane: Plane3,
        policy: &SectionPolicy,
    ) -> Result<PlaneSection, SectionError> {
        section_body(self.geometry(), plane, policy)
    }

    /// Resolves persistent surface and frame intent into this snapshot's scope.
    /// All authored attachment vectors use part-local coordinates.
    ///
    /// # Errors
    /// Same missing/ambiguous surface, projection and budget checks as
    /// [`WorkplaneAttachment::resolve`].
    pub fn resolve_attachment(
        &self,
        attachment: &WorkplaneAttachment,
        policy: &WorkplanePolicy,
    ) -> Result<SnapshotWorkplane, WorkplaneError> {
        let plane = attachment.resolve(self.geometry(), policy)?;
        Ok(SnapshotWorkplane {
            body: self.clone(),
            plane,
        })
    }

    /// Constructs an authored workplane pinned to this body and snapshot.
    /// Origin and X direction are in part coordinates, not occurrence world
    /// coordinates. Use the captured render item's placement when placing it.
    ///
    /// # Errors
    /// Same selection, planarity and budget checks as [`face_workplane`].
    pub fn workplane(
        &self,
        selection: WorkplaneSelection,
        origin: [f64; 3],
        x_direction: [f64; 3],
        policy: &WorkplanePolicy,
    ) -> Result<SnapshotWorkplane, WorkplaneError> {
        let plane = face_workplane(self.geometry(), selection, origin, x_direction, policy)?;
        Ok(SnapshotWorkplane {
            body: self.clone(),
            plane,
        })
    }
}

/// A workplane with immutable owning geometry and explicit snapshot identity.
/// It has no serialization contract and is not a persistent face attachment.
#[derive(Clone, Debug)]
pub struct SnapshotWorkplane {
    body: SnapshotBody,
    plane: Workplane,
}
impl SnapshotWorkplane {
    /// The checked local workplane. It stays valid for [`Self::body`].
    #[must_use]
    pub fn plane(&self) -> &Workplane {
        &self.plane
    }

    /// The immutable body that owns this selection.
    #[must_use]
    pub fn body(&self) -> &SnapshotBody {
        &self.body
    }

    /// Extracts a checked planar material boundary for footprint measurements.
    /// Uses this workplane's immutable owning body, preserving its association.
    /// Build once, then query multiple footprints without repeating extraction.
    ///
    /// # Errors
    /// Reports invalid boundaries, numeric limits or exhausted work budgets.
    pub fn planar_patch(&self, policy: &BoundaryPolicy) -> Result<PlanarPatch, ClearanceError> {
        PlanarPatch::from_workplane(self.body.geometry(), &self.plane, policy)
    }

    /// Verifies identity before using this selection with another body handle.
    /// A new snapshot, a different part, or a different body is stale even if
    /// its mesh revision counter or face indices happen to be identical.
    ///
    /// # Errors
    /// Returns [`StaleSelection`] for another snapshot/body or a stale mesh.
    pub fn check(&self, body: &SnapshotBody) -> Result<(), StaleSelection> {
        if !Arc::ptr_eq(&self.body.scope, &body.scope)
            || self.body.part != body.part
            || self.body.body != body.body
            || self.plane.check(&body.geometry().mesh).is_err()
        {
            return Err(StaleSelection);
        }
        Ok(())
    }
}

/// Failure to find a unique body by authored producing-source label.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyLookupError {
    /// The part handle is absent from this snapshot.
    UnknownPart {
        /// Requested snapshot-local part.
        part: PartId,
    },
    /// No emitted body has the requested producing source.
    MissingSource {
        /// Requested snapshot-local part.
        part: PartId,
        /// Requested authored label.
        source: String,
    },
    /// Several emitted bodies have the same producing source.
    AmbiguousSource {
        /// Requested snapshot-local part.
        part: PartId,
        /// Requested authored label.
        source: String,
        /// Number of candidates, none of which was silently chosen.
        matches: u32,
    },
}
impl core::fmt::Display for BodyLookupError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownPart { part } => write!(f, "snapshot has no part {part:?}"),
            Self::MissingSource { part, source } => {
                write!(f, "part {part:?} has no body produced by source {source:?}")
            }
            Self::AmbiguousSource {
                part,
                source,
                matches,
            } => write!(
                f,
                "part {part:?} has {matches} bodies produced by source {source:?}"
            ),
        }
    }
}
impl core::error::Error for BodyLookupError {}

/// A transient selection was used with a different evaluation snapshot or body.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StaleSelection;
impl core::fmt::Display for StaleSelection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("selection belongs to another evaluation snapshot or body")
    }
}
impl core::error::Error for StaleSelection {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Assembly, CompilePolicy, PartCompiler};
    use exedra_constructive::{
        builders,
        ir::{CapMode, NodeKind, Placement3, RecipeBuilder},
        tessellate::REGION_CAP_END,
    };

    fn naming_context(duplicate: bool, retained: bool) -> Assembly {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect(2.0, 3.0).unwrap());
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
            assert!(matches!(
                error,
                crate::CompileError::Evaluate {
                    error: exedra_constructive::evaluate::EvalError {
                        error: exedra_constructive::tessellate::TessellateError::Attachment(
                            WorkplaneError::AmbiguousSelection
                        ),
                        ..
                    },
                    ..
                }
            ));
            compile(&mut compiler, false).unwrap();
        }
    }

    fn assembly(height: f64) -> Assembly {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::rect(2.0, 3.0).unwrap());
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
            &builders::rect(2.0, 3.0).unwrap(),
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
            let profile = b.add_profile(builders::rect(width, 3.0).unwrap());
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
            let hole_profile = b.add_profile(builders::rect(0.2, 0.2).unwrap());
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
        assert!(
            compiler
                .cache
                .values()
                .all(|entry| entry.evaluated.is_none())
        );
        assert_eq!(compiler.counters().parts_compiled, 1);
        let snapshot = compiler.compile_snapshot(&assembly, &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert!(
            compiler
                .cache
                .values()
                .all(|entry| entry.evaluated.is_some())
        );
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
