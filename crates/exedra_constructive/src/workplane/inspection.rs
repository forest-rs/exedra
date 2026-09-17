// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::string::ToString;

/// Work limit that prevented completion of a geometric query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SurfaceResource {
    /// Faces selected for one workplane.
    SelectedFaces,
    /// Face corners examined, including repeated shared vertices.
    Corners,
    /// Body faces scanned by an inventory.
    ScannedFaces,
    /// Distinct selectors retained by an inventory.
    Selectors,
}

/// Structured evidence explaining a workplane refusal.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WorkplaneEvidence {
    /// No additional evidence is available for this category.
    None,
    /// More than one reachable authored node uses this source label.
    DuplicateSource {
        /// Ambiguous opaque label; this is not a transient face identifier.
        source: String,
    },
    /// The selection consists of separate edge-connected patches.
    Disconnected {
        /// Lowest face ID of each patch, in ascending order, scoped to this body.
        representatives: Vec<FaceId>,
    },
    /// A region combines immediate Boolean operands (or non-Boolean faces).
    MixedOperands {
        /// Sorted distinct operands; `None` denotes non-Boolean attribution.
        operands: Vec<Option<u16>>,
    },
    /// Source corners exceed the allowed distance from the measured plane.
    PlaneDeviation {
        /// Maximum observed distance in body units.
        measured: f64,
        /// Authored allowable distance in body units.
        tolerance: f64,
    },
    /// Work stopped at an explicit limit; no successful result is implied.
    Budget {
        /// Exhausted work or output limit.
        resource: SurfaceResource,
        /// Work completed or output retained before the refused next item.
        completed: u64,
        /// Maximum allowed work or output count.
        limit: u64,
    },
}

/// Refusal with the exact requested selector and machine-readable evidence.
/// Face IDs in evidence belong to the queried body snapshot only.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkplaneFailure {
    /// Requested selection, preserved even when it matches no faces.
    pub selection: WorkplaneSelection,
    /// Stable coarse classification for callers that do not need detail.
    pub kind: WorkplaneError,
    /// Evidence for diagnosis without parsing the display string.
    pub evidence: WorkplaneEvidence,
}
impl core::fmt::Display for WorkplaneFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}: {}", self.selection, self.kind)?;
        match &self.evidence {
            WorkplaneEvidence::None => Ok(()),
            WorkplaneEvidence::DuplicateSource { source } => {
                write!(f, "; duplicate source {source:?}")
            }
            WorkplaneEvidence::Disconnected { representatives } => {
                write!(f, "; {} connected patches", representatives.len())
            }
            WorkplaneEvidence::MixedOperands { operands } => {
                write!(f, "; {} operand identities", operands.len())
            }
            WorkplaneEvidence::PlaneDeviation {
                measured,
                tolerance,
            } => write!(f, "; deviation {measured} exceeds {tolerance}"),
            WorkplaneEvidence::Budget {
                resource,
                completed,
                limit,
            } => write!(f, "; {resource:?}: {completed} completed, limit {limit}"),
        }
    }
}
impl core::error::Error for WorkplaneFailure {}

// Internal refusal before the public entry point supplies selection context.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Rejection(pub WorkplaneError, pub WorkplaneEvidence);
impl From<WorkplaneError> for Rejection {
    fn from(kind: WorkplaneError) -> Self {
        Self(kind, WorkplaneEvidence::None)
    }
}
impl Rejection {
    pub(super) fn for_selection(self, selection: WorkplaneSelection) -> WorkplaneFailure {
        WorkplaneFailure {
            selection,
            kind: self.0,
            evidence: self.1,
        }
    }
}
pub(super) fn budget(resource: SurfaceResource, completed: u64, limit: u64) -> Rejection {
    Rejection(
        WorkplaneError::BudgetExceeded,
        WorkplaneEvidence::Budget {
            resource,
            completed,
            limit,
        },
    )
}
pub(super) fn duplicate(source: &str) -> Rejection {
    Rejection(
        WorkplaneError::AmbiguousSelection,
        WorkplaneEvidence::DuplicateSource {
            source: source.into(),
        },
    )
}

pub use exedra_mesh_ops::workplane::SurfacePlane;

fn analyze_patch(
    mesh: &Mesh,
    faces: &[FaceId],
    tolerance: f64,
    corners: &mut u64,
    limit: u64,
) -> Result<SurfacePlane, Rejection> {
    exedra_mesh_ops::workplane::analyze_patch(mesh, faces, tolerance, corners, limit)
        .map_err(Rejection::from)
}

fn components(
    mesh: &Mesh,
    faces: &[FaceId],
    corners: &mut u64,
    limit: u64,
) -> Result<Vec<Vec<FaceId>>, Rejection> {
    exedra_mesh_ops::workplane::connected_patches(mesh, faces, corners, limit)
        .map_err(Rejection::from)
}

impl From<exedra_mesh_ops::workplane::FrameFailure> for Rejection {
    fn from(failure: exedra_mesh_ops::workplane::FrameFailure) -> Self {
        use exedra_mesh_ops::workplane::{FrameEvidence, FrameResource};
        let evidence = match failure.evidence {
            FrameEvidence::None => WorkplaneEvidence::None,
            FrameEvidence::Disconnected { representatives } => {
                WorkplaneEvidence::Disconnected { representatives }
            }
            FrameEvidence::PlaneDeviation {
                measured,
                tolerance,
            } => WorkplaneEvidence::PlaneDeviation {
                measured,
                tolerance,
            },
            FrameEvidence::Budget {
                resource,
                completed,
                limit,
            } => WorkplaneEvidence::Budget {
                resource: match resource {
                    FrameResource::SelectedFaces => SurfaceResource::SelectedFaces,
                    FrameResource::Corners => SurfaceResource::Corners,
                },
                completed,
                limit,
            },
        };
        Self(failure.kind, evidence)
    }
}

pub(super) fn selection_ambiguity(
    body: &TessellatedBody,
    selection: &WorkplaneSelection,
    faces: &[FaceId],
) -> Option<Rejection> {
    if let WorkplaneSelection::SourceStartCap(source) | WorkplaneSelection::SourceEndCap(source) =
        selection
        && body.source_map.source_is_ambiguous(source)
    {
        return Some(duplicate(source));
    }
    if matches!(selection, WorkplaneSelection::Region(_)) {
        let operands: BTreeSet<_> = faces
            .iter()
            .map(|&face| match body.source_map.face_feature(face) {
                Some(Feature::BooleanFace { operand }) => Some(operand),
                _ => None,
            })
            .collect();
        if operands.len() > 1 {
            return Some(Rejection(
                WorkplaneError::AmbiguousSelection,
                WorkplaneEvidence::MixedOperands {
                    operands: operands.into_iter().collect(),
                },
            ));
        }
    }
    None
}

/// Global limits for a complete surface inventory. No partial inventory is
/// returned on budget exhaustion. Geometry is measured in body coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SurfaceInventoryPolicy {
    /// Maximum body faces scanned, including faces with unknown ancestry.
    pub max_faces: u32,
    /// Maximum corner visits across all selectors' connectivity and geometry
    /// checks. A face appearing under several selectors is visited repeatedly.
    pub max_corners: u64,
    /// Maximum distinct supported selectors retained.
    pub max_selectors: u32,
    /// Positive finite planar distance tolerance, in body units.
    pub distance_tolerance: f64,
}
impl Default for SurfaceInventoryPolicy {
    fn default() -> Self {
        Self {
            max_faces: 65536,
            max_corners: 1_000_000,
            max_selectors: 16384,
            distance_tolerance: 1e-6,
        }
    }
}

/// Work actually performed while building an inventory.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceInventoryStats {
    /// Body faces scanned once for selector membership.
    pub faces_scanned: u32,
    /// Scanned faces with no surviving original surface record.
    pub unknown_origin_faces: u32,
    /// Scanned faces with no supported semantic cap or region selector.
    /// Individual transient face queries remain available separately.
    pub faces_without_selectors: u32,
    /// Corner visits across connectivity and geometry analysis of all selectors.
    pub corners_examined: u64,
}

/// A refused inventory, never an apparently complete partial result.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfaceInventoryError {
    /// Invalid limits or distance tolerance.
    InvalidPolicy,
    /// Source provenance is stale relative to the mesh.
    StaleSource,
    /// The next work item or output entry exceeded an explicit limit.
    Budget {
        /// Exhausted work or output limit.
        resource: SurfaceResource,
        /// Items completed before refusing the next one.
        completed: u64,
        /// Authored maximum.
        limit: u64,
    },
}
impl core::fmt::Display for SurfaceInventoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "surface inventory: {self:?}")
    }
}
impl core::error::Error for SurfaceInventoryError {}

/// One connected patch within a semantic selection. Faces are transient IDs in
/// the source body, not a persistent patch name or a serialized attachment.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfacePatch {
    /// Source faces in ascending ID order.
    pub faces: Vec<FaceId>,
    /// Measured plane or the geometric reason this patch cannot supply one.
    /// Success does not choose an origin/roll, certify a solid or test containment.
    pub plane: Result<SurfacePlane, WorkplaneFailure>,
}

/// Available evidence for one supported semantic selector on this body.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceEntry {
    /// Reusable intent. This selector must be resolved afresh after edits.
    pub selector: SurfaceSelector,
    /// Distinct original surface roles and authored labels, in sorted order.
    pub origins: Vec<crate::source_map::SurfaceOrigin>,
    /// Faces for which no original surface record survived.
    pub unknown_origin_faces: u32,
    /// Connected patches, ordered by lowest face ID. Empty when topology cannot
    /// be traversed; the selection status then explains the refusal.
    pub patches: Vec<SurfacePatch>,
    /// Whether this selector identifies one geometrically valid planar patch
    /// under the inventory's tolerance. Authored frame inputs and workplane
    /// policy still require validation when the caller resolves an attachment.
    pub status: Result<(), WorkplaneFailure>,
}

/// Complete inventory of supported cap and region selectors with surviving
/// faces. Missing selectors are not invented; absence alone cannot distinguish
/// a destroyed surface from an unauthored or unpreserved one.
#[derive(Clone, Debug)]
pub struct SurfaceInventory {
    entries: Vec<SurfaceEntry>,
    stats: SurfaceInventoryStats,
    revision: MeshRevision,
    policy: SurfaceInventoryPolicy,
}
impl SurfaceInventory {
    /// Entries in deterministic selector order. Several selectors may overlap.
    #[must_use]
    pub fn entries(&self) -> &[SurfaceEntry] {
        &self.entries
    }
    /// Effective limits and planarity tolerance used for this inventory.
    #[must_use]
    pub fn policy(&self) -> SurfaceInventoryPolicy {
        self.policy
    }
    /// Measured query work.
    #[must_use]
    pub fn stats(&self) -> SurfaceInventoryStats {
        self.stats
    }
    /// Checks for edits to the same logical body. Revision equality alone does
    /// not identify unrelated meshes; snapshot inventories enforce ownership.
    pub fn check(&self, mesh: &Mesh) -> Result<(), WorkplaneError> {
        if self.revision == mesh.revision() {
            Ok(())
        } else {
            Err(WorkplaneError::StaleSource)
        }
    }
}

/// Inventories surviving semantic surfaces using the same geometric checks as
/// attachment resolution. No tessellation is repeated and no frame is guessed.
/// Budgets bound the full scan and repeated analysis across overlapping selectors.
///
/// # Errors
/// Rejects invalid policy, stale provenance and exhausted work/output budgets.
/// Individual geometric or selection refusals are evidence in successful entries.
///
/// ```
/// use exedra_constructive::{builders::rect, ir::{CapMode, Placement3},
///     tessellate::{tessellate_extrude, EvalPolicy},
///     workplane::{inspect_surfaces, SurfaceInventoryPolicy, SurfaceSelector,
///         WorkplaneAttachment, WorkplanePolicy}};
/// let body = tessellate_extrude(&rect(4.0, 3.0)?, &Placement3::IDENTITY,
///     2.0, CapMode::Both, &EvalPolicy::default())?;
/// let inventory = inspect_surfaces(&body, &SurfaceInventoryPolicy::default())?;
/// let cap = inventory.entries().iter()
///     .find(|entry| entry.selector == SurfaceSelector::EndCap).unwrap();
/// assert!(cap.status.is_ok());
/// let plane = WorkplaneAttachment {
///     surface: cap.selector.clone(), anchor: [1.0, 1.0, 0.0],
///     projection: [0.0, 0.0, 1.0], x_direction: [1.0, 0.0, 0.0],
/// }.resolve(&body, &WorkplanePolicy::default())?;
/// assert_eq!(plane.to_body([0.0; 3]), [1.0, 1.0, 2.0]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn inspect_surfaces(
    body: &TessellatedBody,
    policy: &SurfaceInventoryPolicy,
) -> Result<SurfaceInventory, SurfaceInventoryError> {
    use SurfaceInventoryError as Error;
    use alloc::collections::BTreeMap;
    if policy.max_faces == 0
        || policy.max_corners == 0
        || policy.max_selectors == 0
        || !policy.distance_tolerance.is_finite()
        || policy.distance_tolerance <= 0.0
    {
        return Err(Error::InvalidPolicy);
    }
    body.source_map
        .check(&body.mesh)
        .map_err(|_| Error::StaleSource)?;
    let mut groups: BTreeMap<SurfaceSelector, Vec<FaceId>> = BTreeMap::new();
    let mut stats = SurfaceInventoryStats::default();
    let regions = body.mesh.attrs().dense(attr::FACE_REGION);
    for face in body.mesh.faces() {
        if stats.faces_scanned == policy.max_faces {
            return Err(Error::Budget {
                resource: SurfaceResource::ScannedFaces,
                completed: u64::from(stats.faces_scanned),
                limit: u64::from(policy.max_faces),
            });
        }
        stats.faces_scanned += 1;
        let mut selectors = Vec::new();
        if body.source_map.surface_origin(face).is_none() {
            stats.unknown_origin_faces += 1;
        }
        if let Some(origin) = body.source_map.surface_origin(face) {
            match origin.feature {
                Feature::CapStart => {
                    selectors.push(SurfaceSelector::StartCap);
                    if let Some(source) = &origin.source {
                        selectors.push(SurfaceSelector::SourceStartCap(source.to_string()));
                    }
                }
                Feature::CapEnd => {
                    selectors.push(SurfaceSelector::EndCap);
                    if let Some(source) = &origin.source {
                        selectors.push(SurfaceSelector::SourceEndCap(source.to_string()));
                    }
                }
                _ => {}
            }
        }
        if let Some(&region) = regions.and_then(|values| values.get(face.into())) {
            selectors.push(SurfaceSelector::Region(region));
            if let Some(Feature::BooleanFace { operand }) = body.source_map.face_feature(face) {
                selectors.push(SurfaceSelector::OperandRegion(OperandRegion {
                    operand,
                    region,
                }));
            }
        }
        if selectors.is_empty() {
            stats.faces_without_selectors += 1;
        }
        for selector in selectors {
            if !groups.contains_key(&selector) && groups.len() >= policy.max_selectors as usize {
                return Err(Error::Budget {
                    resource: SurfaceResource::Selectors,
                    completed: groups.len() as u64,
                    limit: u64::from(policy.max_selectors),
                });
            }
            groups.entry(selector).or_default().push(face);
        }
    }
    let mut entries = Vec::new();
    for (selector, faces) in groups {
        let selection = selector.selection();
        let mut origins = BTreeSet::new();
        let mut unknown_origin_faces = 0;
        for &face in &faces {
            if let Some(origin) = body.source_map.surface_origin(face) {
                origins.insert(origin.clone());
            } else {
                unknown_origin_faces += 1;
            }
        }
        let ambiguity = selection_ambiguity(body, &selection, &faces);
        let patch_faces = match components(
            &body.mesh,
            &faces,
            &mut stats.corners_examined,
            policy.max_corners,
        ) {
            Ok(patches) => patches,
            Err(reason) => {
                reject_inventory_budget(&reason)?;
                entries.push(SurfaceEntry {
                    selector,
                    origins: origins.into_iter().collect(),
                    unknown_origin_faces,
                    patches: Vec::new(),
                    status: Err(reason.for_selection(selection)),
                });
                continue;
            }
        };
        let mut patches = Vec::new();
        for faces in patch_faces {
            let plane = analyze_patch(
                &body.mesh,
                &faces,
                policy.distance_tolerance,
                &mut stats.corners_examined,
                policy.max_corners,
            );
            if let Err(reason) = &plane {
                reject_inventory_budget(reason)?;
            }
            patches.push(SurfacePatch {
                faces,
                plane: plane.map_err(|reason| reason.for_selection(selection.clone())),
            });
        }
        let status = if let Some(reason) = ambiguity {
            Err(reason.for_selection(selection))
        } else if patches.len() != 1 {
            Err(Rejection(
                WorkplaneError::AmbiguousSelection,
                WorkplaneEvidence::Disconnected {
                    representatives: patches.iter().map(|patch| patch.faces[0]).collect(),
                },
            )
            .for_selection(selection))
        } else {
            patches[0].plane.as_ref().map(|_| ()).map_err(Clone::clone)
        };
        entries.push(SurfaceEntry {
            selector,
            origins: origins.into_iter().collect(),
            unknown_origin_faces,
            patches,
            status,
        });
    }
    Ok(SurfaceInventory {
        entries,
        stats,
        revision: body.mesh.revision(),
        policy: *policy,
    })
}

fn reject_inventory_budget(reason: &Rejection) -> Result<(), SurfaceInventoryError> {
    if let WorkplaneEvidence::Budget {
        resource,
        completed,
        limit,
    } = &reason.1
    {
        Err(SurfaceInventoryError::Budget {
            resource: *resource,
            completed: *completed,
            limit: *limit,
        })
    } else {
        Ok(())
    }
}
