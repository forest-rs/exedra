// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{CompiledParts, PartId};
use alloc::{rc::Rc, string::String, sync::Arc, vec::Vec};
use exedra_constructive::{
    clearance::{BoundaryPolicy, ClearanceError, PlanarPatch},
    evaluate::PlacedBody,
    ir::{NodeId, Plane3, Recipe, SlotId},
    section::{PlaneSection, SectionError, SectionPolicy, section_body},
    tessellate::TessellatedBody,
    workplane::{
        SurfaceInventory, SurfaceInventoryError, SurfaceInventoryPolicy, Workplane,
        WorkplaneAttachment, WorkplaneFailure, WorkplanePolicy, WorkplaneSelection, face_workplane,
        inspect_surfaces,
    },
};

#[derive(Debug)]
struct EvaluatedBody {
    node: Option<NodeId>,
    source: Option<String>,
    material: Option<SlotId>,
    body: TessellatedBody,
    native: Option<crate::NativeRealization>,
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
                    native: None,
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
                native: None,
                body: TessellatedBody::from_imported_mesh(mesh.clone()),
            }],
        }
    }
    pub(super) fn from_native(mesh: exedra_mesh::Mesh, native: crate::NativeRealization) -> Self {
        Self {
            bodies: alloc::vec![EvaluatedBody {
                node: None,
                source: None,
                material: None,
                body: TessellatedBody::from_imported_mesh(mesh),
                native: Some(native)
            }],
        }
    }
}

/// Immutable geometry, render buffers and query scope for an explicit input list.
///
/// Contains part-local topology and source correspondence with recipe transforms
/// already baked in. No hierarchy, occurrence placement or material binding is
/// captured. Queries never evaluate geometry again. Clones share tables, topology
/// and selection scope in constant time; each successful compilation creates a
/// fresh scope, including cache hits. Held handles survive compiler eviction and
/// are `Send + Sync`. Inspect reports for partial evaluations and refusals.
#[derive(Clone, Debug)]
pub struct GeometrySnapshot {
    compiled: Arc<CompiledParts>,
    evaluated: Arc<[Arc<EvaluatedPart>]>,
    scope: Arc<()>,
    native_sources: Arc<[Option<crate::NativeSource>]>,
}

impl GeometrySnapshot {
    pub(super) fn new(
        compiled: CompiledParts,
        evaluated: Vec<Arc<EvaluatedPart>>,
        native_sources: Vec<Option<crate::NativeSource>>,
    ) -> Self {
        Self {
            compiled: Arc::new(compiled),
            evaluated: evaluated.into(),
            scope: Arc::new(()),
            native_sources: native_sources.into(),
        }
    }

    /// Render buffers and evaluation reports produced from the retained bodies.
    #[must_use]
    pub fn compiled(&self) -> &CompiledParts {
        &self.compiled
    }

    /// Retains one input's geometry, buffers and report without retaining other inputs.
    /// Invalid input indices return `None`; clones share this snapshot's selection scope.
    #[must_use]
    pub fn part(&self, part: PartId) -> Option<PartSnapshot> {
        Some(PartSnapshot {
            compiled: Arc::clone(self.compiled.part(part)?),
            report: self
                .compiled
                .reports
                .get(part.0 as usize)?
                .as_ref()
                .map(Arc::clone),
            evaluated: Arc::clone(self.evaluated.get(part.0 as usize)?),
            scope: Arc::clone(&self.scope),
            part,
            native_source: self.native_sources.get(part.0 as usize)?.clone(),
        })
    }

    /// Resolves a unique producing source within an input.
    /// # Errors
    /// Identifies unknown inputs, missing labels and ambiguity.
    pub fn body_by_source(
        &self,
        part: PartId,
        source: &str,
    ) -> Result<SnapshotBody, BodyLookupError> {
        self.part(part)
            .ok_or(BodyLookupError::UnknownPart { part })?
            .body_by_source(source)
    }

    /// Retains one body using input-local indices; invalid indices return `None`.
    #[must_use]
    pub fn body(&self, part: PartId, body: u32) -> Option<SnapshotBody> {
        self.part(part)?.body(body)
    }
}

/// One input's immutable geometry, extraction buffers and report.
/// Retains no other inputs and shares the parent snapshot's query scope. It remains
/// usable after that snapshot and the compiler are dropped.
#[derive(Clone, Debug)]
pub struct PartSnapshot {
    compiled: Arc<crate::CompiledPart>,
    report: Option<Arc<exedra_constructive::evaluate::GeometryReport>>,
    evaluated: Arc<EvaluatedPart>,
    scope: Arc<()>,
    part: PartId,
    native_source: Option<crate::NativeSource>,
}
impl PartSnapshot {
    /// Shared compiled buffers and content fingerprint.
    #[must_use]
    pub fn compiled(&self) -> &Arc<crate::CompiledPart> {
        &self.compiled
    }
    /// Complete constructive report; baked inputs have no evaluation report.
    #[must_use]
    pub fn report(&self) -> Option<&exedra_constructive::evaluate::GeometryReport> {
        self.report.as_deref()
    }

    /// Resolves one producing source label within a part of this snapshot.
    /// Use authored labels after parameter edits instead of retaining a previous
    /// body's numeric index. Equal labels are not silently resolved by order.
    ///
    /// # Errors
    /// Identifies an absent or ambiguous source with its match count. The source is the producing recipe node's label, not a
    /// recursive search for labels erased by later geometry operations.
    pub fn body_by_source(&self, source: &str) -> Result<SnapshotBody, BodyLookupError> {
        let part = self.part;
        let evaluated = &self.evaluated;
        let mut first = None;
        let mut count = 0;
        for (index, body) in evaluated.bodies.iter().enumerate() {
            if body.source.as_deref() == Some(source) {
                first.get_or_insert(crate::len_u32(index));
                count += 1;
            }
        }
        match (first, count) {
            (Some(index), 1) => Ok(self.body(index).expect("resolved snapshot body")),
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
    pub fn body(&self, body: u32) -> Option<SnapshotBody> {
        let evaluated = &self.evaluated;
        let part = self.part;
        evaluated.bodies.get(body as usize)?;
        Some(SnapshotBody {
            evaluated: Arc::clone(evaluated),
            scope: Arc::clone(&self.scope),
            part,
            body,
            native_source: self.native_source.clone(),
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
    native_source: Option<crate::NativeSource>,
}
impl SnapshotBody {
    fn entry(&self) -> &EvaluatedBody {
        &self.evaluated.bodies[self.body as usize]
    }

    /// Input-local part index within the owning snapshot.
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

    /// Immutable native installation for this input, or `None` for recipes and
    /// baked meshes. Equal-content cache hits retain this input's own source
    /// scope rather than borrowing a previous installation's selection scope.
    #[must_use]
    pub fn native(&self) -> Option<&crate::NativeSource> {
        self.native_source.as_ref()
    }

    /// Accuracy and native correspondence for the current display realization.
    /// The legacy mesh source map describes mesh topology; native provenance is
    /// explicitly supplied by this record.
    #[must_use]
    pub fn native_realization(&self) -> Option<&crate::NativeRealization> {
        self.entry().native.as_ref()
    }

    /// Selects the native source of a topology face in this realization.
    /// Returns `None` for nonnative bodies or an unknown mesh face.
    #[must_use]
    pub fn native_face(&self, face: exedra_mesh::FaceId) -> Option<crate::NativeFaceSelection> {
        let mapping = &self.native_realization()?.mesh_faces;
        let index = mapping.binary_search_by_key(&face, |(f, _)| *f).ok()?;
        let id = mapping[index].1;
        self.native()?.select_face(id).ok()
    }

    /// Selects the native source of one final render triangle ordinal.
    /// This follows the compiler's actual regrouping and needs no region decoding.
    /// Returns `None` for nonnative bodies or unknown triangle ordinals.
    #[must_use]
    pub fn native_render_face(&self, triangle: usize) -> Option<crate::NativeFaceSelection> {
        let id = self.native_realization()?.render_face(triangle)?;
        self.native()?.select_face(id).ok()
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

    /// Effective authored face material slot before caller binding resolution.
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

    /// Inventories surviving semantic selectors and connected patches in part
    /// coordinates. Retains this body and snapshot; no geometry is reevaluated.
    ///
    /// # Errors
    /// Rejects invalid policy, stale provenance and exhausted inventory budgets.
    pub fn inspect_surfaces(
        &self,
        policy: &SurfaceInventoryPolicy,
    ) -> Result<SnapshotSurfaceInventory, SurfaceInventoryError> {
        Ok(SnapshotSurfaceInventory {
            body: self.clone(),
            inventory: inspect_surfaces(self.geometry(), policy)?,
        })
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
    ) -> Result<SnapshotWorkplane, WorkplaneFailure> {
        let plane = attachment.resolve(self.geometry(), policy)?;
        Ok(SnapshotWorkplane {
            body: self.clone(),
            plane,
        })
    }

    /// Constructs an authored workplane pinned to this body and snapshot.
    /// Origin and X direction are in part coordinates, not occurrence world
    /// coordinates. Use the caller's occurrence placement when placing it.
    ///
    /// # Errors
    /// Same selection, planarity and budget checks as [`face_workplane`].
    pub fn workplane(
        &self,
        selection: WorkplaneSelection,
        origin: [f64; 3],
        x_direction: [f64; 3],
        policy: &WorkplanePolicy,
    ) -> Result<SnapshotWorkplane, WorkplaneFailure> {
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

/// Surface evidence owning its immutable body and evaluation-snapshot scope.
#[derive(Clone, Debug)]
pub struct SnapshotSurfaceInventory {
    body: SnapshotBody,
    inventory: SurfaceInventory,
}
impl SnapshotSurfaceInventory {
    /// Complete inspected selectors and patches. IDs belong to [`Self::body`].
    #[must_use]
    pub fn inventory(&self) -> &SurfaceInventory {
        &self.inventory
    }
    /// Immutable source body, retained independently of compiler-cache eviction.
    #[must_use]
    pub fn body(&self) -> &SnapshotBody {
        &self.body
    }
    /// Checks identity before applying evidence to another body handle.
    ///
    /// # Errors
    /// Another snapshot, part or body is stale even if geometry was cached.
    pub fn check(&self, body: &SnapshotBody) -> Result<(), StaleSelection> {
        if !Arc::ptr_eq(&self.body.scope, &body.scope)
            || self.body.part != body.part
            || self.body.body != body.body
            || self.inventory.check(&body.geometry().mesh).is_err()
        {
            Err(StaleSelection)
        } else {
            Ok(())
        }
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
