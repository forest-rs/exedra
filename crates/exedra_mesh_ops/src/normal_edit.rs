// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Authored corner-normal editing with explicit geometric source checks.

use crate::selection::canonicalize_face_set;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use exedra_mesh::{CornerId, FaceId, HalfEdgeId, Mesh, MeshRevision, NormalParams, VertexId, op};

/// How to update authored corner-normal overrides on selected faces.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum NormalEdit {
    /// Remove authored overrides; extraction can derive normals again.
    Clear,
    /// Bake one geometric polygon normal on every selected face corner.
    Face,
    /// Bake the kernel's current derived normals, including adjacent faces.
    Derived(NormalParams),
    /// Derive normals with selected-patch boundaries temporarily hardened, then
    /// bake only selected corners. Source edge sharpness is unchanged.
    Smooth(NormalParams),
}

/// Selected faces and the authored-normal operation to perform.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalEditParams {
    /// Face IDs, sorted and deduplicated during preparation.
    pub faces: Vec<FaceId>,
    /// Normal source or clearing mode.
    pub mode: NormalEdit,
}

/// Refusal while preparing or writing corner normals.
#[derive(Clone, Debug, PartialEq)]
pub enum NormalEditError {
    /// Invalid or stale selected face topology.
    Selection(exedra_mesh::SelectedFacePatchError),
    /// Automatic sharp-angle threshold is nonfinite or outside `[0, 180]`.
    InvalidParameters,
    /// A selected polygon has no usable geometric normal.
    DegenerateFace {
        /// Offending selected face.
        face: FaceId,
    },
    /// Kernel derivation did not produce a usable normal for a selected corner.
    MissingDerivedNormal {
        /// Face owning the corner.
        face: FaceId,
        /// Corner whose normal is unavailable.
        corner: CornerId,
    },
    /// Source positions or sharpness used by normal derivation are nonfinite.
    NumericLimit,
    /// Source revision or captured geometric dependencies changed.
    StalePreparation,
    /// Kernel refused a temporary selection-boundary sharpness update.
    BoundarySharpness(op::SetEdgeSharpnessError),
    /// Kernel refused an authored-normal write; earlier eager writes may remain.
    Write(op::SetCornerNormalOverrideError),
}
impl core::fmt::Display for NormalEditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Selection(error) => write!(f, "normal selection: {error}"),
            Self::InvalidParameters => f.write_str("invalid normal derivation parameters"),
            Self::DegenerateFace { face } => write!(f, "face {face:?} has no usable normal"),
            Self::MissingDerivedNormal { face, corner } => {
                write!(f, "face {face:?} corner {corner:?} has no derived normal")
            }
            Self::NumericLimit => f.write_str("normal source exceeds numeric limits"),
            Self::StalePreparation => {
                f.write_str("normal preparation no longer matches its source")
            }
            Self::BoundarySharpness(error) => write!(f, "normal selection boundary: {error}"),
            Self::Write(error) => write!(f, "corner normal write: {error}"),
        }
    }
}
impl core::error::Error for NormalEditError {}

/// Completed normal writes and canonical source selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalEditOutput {
    /// Edited source faces; no topology is generated or removed.
    pub faces: Vec<FaceId>,
    /// Authored corner-normal writes, including removals of absent overrides.
    pub corners_written: u64,
    /// Preparation sorted or deduplicated the selection.
    pub selections_canonicalized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CornerSource {
    corner: CornerId,
    vertex: VertexId,
    twin: HalfEdgeId,
    adjacent: FaceId,
    position: Option<[u32; 3]>,
    sharpness: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct FaceSource {
    face: FaceId,
    corners: Vec<CornerSource>,
}

/// Inspectable authored-normal writes bound to their geometric dependencies.
///
/// Derived and smoothed normals capture all faces incident to selected vertices,
/// since a change across the selection boundary may change a selected normal.
/// UVs, regions and existing normal overrides do not affect this calculation.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalEditPlan {
    faces: Vec<FaceId>,
    mode: NormalEdit,
    writes: Vec<(CornerId, Option<[f32; 3]>)>,
    selections_canonicalized: bool,
    revision: MeshRevision,
    sources: Vec<FaceSource>,
}
impl NormalEditPlan {
    /// Computes intended normal writes without editing the source.
    pub fn prepare(mesh: &Mesh, params: &NormalEditParams) -> Result<Self, NormalEditError> {
        if let NormalEdit::Derived(normals) | NormalEdit::Smooth(normals) = params.mode
            && normals
                .auto_sharp_angle_degrees
                .is_some_and(|v| !v.is_finite() || !(0.0..=180.0).contains(&v))
        {
            return Err(NormalEditError::InvalidParameters);
        }
        let mut faces = params.faces.clone();
        let selections_canonicalized = canonicalize_face_set(&mut faces);
        let sources = capture_sources(mesh, &faces, params.mode)?;
        let mut writes = Vec::new();
        match params.mode {
            NormalEdit::Clear => {
                writes.extend(
                    faces
                        .iter()
                        .flat_map(|&face| mesh.face_loop(face).map(|corner| (corner, None))),
                );
            }
            NormalEdit::Face => {
                for &face in &faces {
                    let corners: Vec<_> = mesh.face_loop(face).collect();
                    let vertices: Vec<_> = corners
                        .iter()
                        .map(|&corner| mesh.to_vertex(corner).expect("captured corner has vertex"))
                        .collect();
                    let normal = crate::patch::geom::normalized_face_normal(mesh, &vertices)
                        .ok_or(NormalEditError::DegenerateFace { face })?;
                    writes.extend(corners.into_iter().map(|corner| (corner, Some(normal))));
                }
            }
            NormalEdit::Derived(normals) | NormalEdit::Smooth(normals) => {
                let derived = if matches!(params.mode, NormalEdit::Smooth(_)) {
                    derive_normals_with_selection_boundary_hardened(mesh, &faces, &normals)
                        .map_err(NormalEditError::BoundarySharpness)?
                } else {
                    mesh.derive_corner_normals(&normals)
                };
                for &face in &faces {
                    for corner in mesh.face_loop(face) {
                        let normal = derived
                            .get(corner)
                            .filter(|normal| exedra_math::finite(*normal))
                            .ok_or(NormalEditError::MissingDerivedNormal { face, corner })?;
                        writes.push((corner, Some(normal)));
                    }
                }
            }
        }
        Ok(Self {
            faces,
            mode: params.mode,
            writes,
            selections_canonicalized,
            revision: mesh.revision(),
            sources,
        })
    }
    /// Canonical selected face IDs.
    #[must_use]
    pub fn faces(&self) -> &[FaceId] {
        &self.faces
    }
    /// Prepared corner writes, in selected-face/corner order.
    #[must_use]
    pub fn writes(&self) -> &[(CornerId, Option<[f32; 3]>)] {
        &self.writes
    }
    /// Whether preparation sorted or deduplicated the selection.
    #[must_use]
    pub fn selections_canonicalized(&self) -> bool {
        self.selections_canonicalized
    }
    /// Applies writes after checking source revision and exact geometric state,
    /// including unfinished edits. Equivalent clones are accepted. Kernel write
    /// failures can leave partial edits in the eager session; finish the caller's
    /// change sink after success or failure. No topology or other layers change.
    pub fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
    ) -> Result<NormalEditOutput, NormalEditError> {
        if txn.mesh().revision() != self.revision
            || capture_sources(txn.mesh(), &self.faces, self.mode)
                .ok()
                .as_ref()
                != Some(&self.sources)
        {
            return Err(NormalEditError::StalePreparation);
        }
        for &(corner, normal) in &self.writes {
            op::set_corner_normal_override(txn, corner, normal).map_err(NormalEditError::Write)?;
        }
        Ok(NormalEditOutput {
            faces: self.faces.clone(),
            corners_written: self.writes.len() as u64,
            selections_canonicalized: self.selections_canonicalized,
        })
    }
}

/// Prepares and applies authored-normal edits directly in an eager mesh session.
/// See [`NormalEditPlan::apply`] for source checks and failure semantics.
pub fn edit_normals<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    params: &NormalEditParams,
) -> Result<NormalEditOutput, NormalEditError> {
    NormalEditPlan::prepare(txn.mesh(), params)?.apply(txn)
}

fn capture_sources(
    mesh: &Mesh,
    faces: &[FaceId],
    mode: NormalEdit,
) -> Result<Vec<FaceSource>, NormalEditError> {
    mesh.selected_face_patch_topology(faces)
        .map_err(NormalEditError::Selection)?;
    let derived = matches!(mode, NormalEdit::Derived(_) | NormalEdit::Smooth(_));
    let geometry = !matches!(mode, NormalEdit::Clear);
    let mut dependencies: BTreeSet<_> = faces.iter().copied().collect();
    if derived {
        let vertices: BTreeSet<_> = faces
            .iter()
            .flat_map(|&face| {
                mesh.face_loop(face)
                    .filter_map(|corner| mesh.to_vertex(corner))
            })
            .collect();
        // Scan all faces so disconnected fans sharing a vertex are included too;
        // one vertex-star walk need not visit every fan of a nonmanifold vertex.
        for face in mesh.faces() {
            if mesh.face_loop(face).any(|corner| {
                mesh.to_vertex(corner)
                    .is_some_and(|vertex| vertices.contains(&vertex))
            }) {
                dependencies.insert(face);
            }
        }
    }
    let mut sources = Vec::with_capacity(dependencies.len());
    for face in dependencies {
        let invalid = || {
            NormalEditError::Selection(exedra_mesh::SelectedFacePatchError::InvalidFaceLoop {
                face,
            })
        };
        let mut corners = Vec::new();
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).ok_or_else(invalid)?;
            let twin = mesh.twin(corner).ok_or_else(invalid)?;
            let adjacent = mesh.face(twin).ok_or_else(invalid)?;
            let position = if geometry {
                let point = mesh.vertex_position(vertex).ok_or_else(invalid)?;
                if !exedra_math::finite(*point) {
                    return Err(NormalEditError::NumericLimit);
                }
                Some(point.map(f32::to_bits))
            } else {
                None
            };
            let sharpness = if derived {
                mesh.edge_sharpness(corner)
            } else {
                None
            };
            if sharpness.is_some_and(|v| !v.is_finite()) {
                return Err(NormalEditError::NumericLimit);
            }
            corners.push(CornerSource {
                corner,
                vertex,
                twin,
                adjacent,
                position,
                sharpness: sharpness.map(f32::to_bits),
            });
        }
        if corners.len() < 3 {
            return Err(invalid());
        }
        sources.push(FaceSource { face, corners });
    }
    Ok(sources)
}
fn derive_normals_with_selection_boundary_hardened(
    mesh: &Mesh,
    faces: &[FaceId],
    params: &NormalParams,
) -> Result<exedra_mesh::DerivedCornerNormals, op::SetEdgeSharpnessError> {
    let mut working = mesh.clone();
    let mut boundary_edges = Vec::new();
    for &face in faces {
        for corner in working.face_loop(face) {
            let Some(twin) = working.twin(corner) else {
                continue;
            };
            let Some(twin_face) = working.face(twin) else {
                continue;
            };
            if twin_face == FaceId::OUTSIDE || faces.binary_search(&twin_face).is_err() {
                boundary_edges.push(corner);
            }
        }
    }
    let mut edit = working.edit();
    for corner in boundary_edges {
        op::set_edge_sharpness(&mut edit, corner, 1.0)?;
    }
    let _: () = edit.finish();
    Ok(working.derive_corner_normals(params))
}

#[cfg(test)]
mod tests;
