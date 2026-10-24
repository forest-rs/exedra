// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! UV projection and affine authoring in mesh coordinates.
//!
//! Projection resolves every selected face and pending UV before writing.
//! Invalid selections, parameters and nonfinite results leave the source untouched.
//! Writes use the caller's eager edit session and change sink; a kernel write
//! failure is returned explicitly and does not roll back earlier writes.
//!
//! A mesh supplied by a loader uses the same authoring path as a generated mesh:
//!
//! ```
//! use exedra_mesh_ops::uv::{map_uvs, UvFrame, UvMapping, UvMappingParams,
//!     UvProjection, UvScope, UvTransform};
//! # let mut mesh = exedra_mesh::Mesh::from_polygons(
//! #     &[[0.0,0.0,0.0], [1.0,0.0,0.0], [0.0,1.0,0.0]], &[&[0,1,2]])?;
//! let mut edit = mesh.edit();
//! let output = map_uvs(&mut edit, &UvMappingParams {
//!     scope: UvScope::WholeMesh,
//!     mapping: UvMapping {
//!         projection: UvProjection::Planar(UvFrame::XY),
//!         transform: UvTransform::rotate_offset_scale(0.3, [0.25, 0.0], [2.0, 1.0])?,
//!     },
//!     options: exedra_mesh::uv_mapping::UvMappingOptions::default(),
//!     write_missing_only: false,
//! })?;
//! let _: () = edit.finish();
//! assert_eq!(output.corners_written, 3);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

/// Differential UV measurements, acceptance bounds and bounded issue witnesses.
pub mod quality;

#[cfg(test)]
#[path = "uv_mapping_tests.rs"]
mod mapping_tests;

use crate::math::FloatExt;
use crate::selection::{FaceSet, canonicalize_face_set};
use alloc::vec::Vec;
pub use exedra_mesh::uv_mapping::{
    BoxUv, PerspectiveUv, UvAxis, UvFaceMapping, UvFrame, UvInput, UvMapping, UvMappingError,
    UvMappingOptions, UvMappingScratch, UvPolePolicy, UvProjection, UvSeamPolicy, UvSet,
    UvTransform,
};
use exedra_mesh::{
    ChangeSink, CornerId, DEFAULT_BOX_NORMAL_EPSILON, EditSession, FaceId, Mesh, op,
};

/// Parameters for [`map_uvs`]. Existing projection requires every selected
/// corner to have UVs unless it is skipped by `write_missing_only`.
#[derive(Clone, Debug, PartialEq)]
pub struct UvMappingParams {
    /// Face selection, canonicalized before execution.
    pub scope: UvScope,
    /// Projection and transform in mesh coordinates.
    pub mapping: UvMapping,
    /// Source/target channels and angular recovery policies.
    pub options: UvMappingOptions,
    /// Retain already authored UVs without transforming them. Angular modes
    /// still use their geometric neighbors for seam and pole recovery.
    pub write_missing_only: bool,
}

/// Authors the selected corner UV channel using a shared kernel mapping.
///
/// Box mapping uses an authored corner normal override when present, otherwise
/// the geometric face normal. Degenerate normals use positive Z and are
/// reported in `fallback_faces`. Mapping validates all pending values before
/// writing, so bad parameters, missing UVs and numeric failures leave the mesh
/// and sink untouched. Topology and other attributes are preserved.
pub fn map_uvs<S: ChangeSink>(
    edit: &mut EditSession<'_, S>,
    params: &UvMappingParams,
) -> Result<UvOutput, UvError> {
    map_uvs_with_scratch(edit, params, &mut UvAuthoringScratch::default())
}

/// Reusable authoring storage. Retain it for repeated mappings to avoid
/// reallocating pending writes, per-face inputs, and angular sorting buffers.
#[derive(Debug, Default)]
pub struct UvAuthoringScratch {
    pending: Vec<(CornerId, [f32; 2])>,
    corners: Vec<CornerId>,
    inputs: Vec<UvInput>,
    projection: UvMappingScratch,
    face: UvFaceMapping,
}
impl UvAuthoringScratch {
    /// Capacity bytes retained by operation buffers; excludes the mesh, returned
    /// face lists and allocator overhead. This is capacity, not an allocation count.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.pending.capacity() * size_of::<(CornerId, [f32; 2])>()
            + self.corners.capacity() * size_of::<CornerId>()
            + self.inputs.capacity() * size_of::<UvInput>()
            + self.face.uvs.capacity() * size_of::<[f32; 2]>()
            + self.projection.retained_bytes()
    }
    /// Releases all retained operation storage.
    pub fn release(&mut self) {
        *self = Self::default();
    }
}

/// Finite bounds of mapped texture coordinates, in texture repeats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvBounds {
    /// Component-wise minimum.
    pub min: [f32; 2],
    /// Component-wise maximum.
    pub max: [f32; 2],
}
fn include_bounds(bounds: &mut Option<UvBounds>, uv: [f32; 2]) {
    if let Some(bounds) = bounds {
        for (i, value) in uv.into_iter().enumerate() {
            bounds.min[i] = bounds.min[i].min(value);
            bounds.max[i] = bounds.max[i].max(value);
        }
    } else {
        *bounds = Some(UvBounds { min: uv, max: uv });
    }
}

/// Reads a built-in UV channel from either supported storage representation.
#[must_use]
pub fn corner_uv(mesh: &Mesh, set: UvSet, corner: CornerId) -> Option<[f32; 2]> {
    mesh.attrs()
        .sparse(set.key())
        .and_then(|layer| layer.get(corner.into()))
        .or_else(|| {
            mesh.attrs()
                .dense(set.key())
                .and_then(|layer| layer.get(corner.into()))
        })
        .copied()
}
fn validate_set(mesh: &Mesh, set: UvSet) -> Result<(), UvError> {
    if mesh
        .attrs()
        .layer_kind(set.key().domain(), set.key().name())
        .is_some()
        && mesh.attrs().sparse(set.key()).is_none()
        && mesh.attrs().dense(set.key()).is_none()
    {
        return Err(UvError::IncompatibleLayer { set });
    }
    Ok(())
}
fn selected_faces(mesh: &Mesh, scope: &UvScope) -> (FaceSet, bool) {
    match scope {
        UvScope::WholeMesh => {
            let mut faces = Vec::with_capacity(mesh.faces().count());
            faces.extend(mesh.faces());
            (faces, false)
        }
        UvScope::FaceSet(faces) => {
            let mut faces = faces.clone();
            let changed = canonicalize_face_set(&mut faces);
            (faces, changed)
        }
    }
}
fn finish_mapping_batch(
    mesh: &Mesh,
    params: &UvMappingParams,
    scratch: &mut UvAuthoringScratch,
    output: &mut UvOutput,
    face: Option<FaceId>,
) -> Result<(), UvError> {
    params
        .mapping
        .map_face_into(
            &scratch.inputs,
            params.options.seams,
            params.options.poles,
            &mut scratch.projection,
            &mut scratch.face,
        )
        .map_err(|error| UvError::Mapping {
            corner: error
                .corner
                .and_then(|index| scratch.corners.get(index).copied()),
            error: error.error,
        })?;
    output.seam_corners_adjusted += scratch.face.seam_corners_adjusted;
    output.pole_corners_repaired += scratch.face.pole_corners_repaired;
    if let Some(face) = face.filter(|_| scratch.face.normal_fallback) {
        output.fallback_faces.push(face);
    }
    if let Some(face) = face.filter(|_| scratch.face.wide_periodic_axes.into_iter().any(|x| x)) {
        output.wide_periodic_faces.push(face);
    }
    for ((&corner, &uv), input) in scratch
        .corners
        .iter()
        .zip(&scratch.face.uvs)
        .zip(&scratch.inputs)
    {
        if params.write_missing_only && corner_uv(mesh, params.options.target, corner).is_some() {
            output.corners_skipped_existing += 1;
            continue;
        }
        include_bounds(&mut output.mapped_bounds, uv);
        if params.mapping.projection.uses_existing()
            && params.options.source == params.options.target
            && input
                .existing
                .is_some_and(|old| old.map(f32::to_bits) == uv.map(f32::to_bits))
        {
            output.corners_unchanged += 1;
        } else {
            scratch.pending.push((corner, uv));
        }
    }
    Ok(())
}

fn prepare_mapping(
    mesh: &Mesh,
    params: &UvMappingParams,
    scratch: &mut UvAuthoringScratch,
) -> Result<UvOutput, UvError> {
    params
        .mapping
        .validate()
        .map_err(|error| UvError::Mapping {
            corner: None,
            error,
        })?;
    validate_set(mesh, params.options.target)?;
    if params.mapping.projection.uses_existing() {
        validate_set(mesh, params.options.source)?;
    }
    scratch.pending.clear();
    let (faces, selections_canonicalized) = selected_faces(mesh, &params.scope);
    let mut output = UvOutput {
        faces,
        selections_canonicalized,
        ..UvOutput::default()
    };
    // Quads are the common case. Higher-valence faces grow this buffer normally.
    scratch
        .pending
        .reserve(output.faces.len().saturating_mul(4));
    let angular_context = params
        .mapping
        .projection
        .periodic_axes()
        .into_iter()
        .any(|axis| axis);
    let batchable = !params.mapping.projection.uses_normals() && !angular_context;
    scratch.corners.clear();
    scratch.inputs.clear();
    for face_index in 0..output.faces.len() {
        let face = output.faces[face_index];
        if face == FaceId::OUTSIDE || mesh.face_edge(face).is_none() {
            return Err(UvError::InvalidFace { face });
        }
        if angular_context
            && params.write_missing_only
            && mesh
                .face_loop(face)
                .all(|corner| corner_uv(mesh, params.options.target, corner).is_some())
        {
            output.corners_skipped_existing += mesh.face_loop(face).count() as u64;
            continue;
        }
        if !batchable {
            scratch.corners.clear();
            scratch.inputs.clear();
        }
        let normal = if params.mapping.projection.uses_normals() {
            for corner in mesh.face_loop(face) {
                corner_position(mesh, corner)?;
            }
            exedra_mesh::geometric_face_normal(mesh, face).unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        for corner in mesh.face_loop(face) {
            if params.write_missing_only
                && !angular_context
                && corner_uv(mesh, params.options.target, corner).is_some()
            {
                output.corners_skipped_existing += 1;
                continue;
            }
            let position = if params.mapping.projection.uses_existing() {
                [0.0; 3]
            } else {
                corner_position(mesh, corner)?
            };
            let normal = if matches!(params.mapping.projection, UvProjection::Box(_)) {
                mesh.attrs()
                    .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                    .and_then(|layer| layer.get(corner.into()))
                    .map_or(normal, |value| value.map(f64::from))
            } else {
                normal
            };
            scratch.corners.push(corner);
            scratch.inputs.push(UvInput {
                position,
                normal,
                existing: if params.mapping.projection.uses_existing() {
                    corner_uv(mesh, params.options.source, corner)
                } else {
                    None
                },
            });
        }
        // Linear modes can cross face boundaries. Bound gathering to a small
        // cache-resident batch; angular correction and normal reports stay per face.
        if !batchable || scratch.inputs.len() >= 256 {
            finish_mapping_batch(
                mesh,
                params,
                scratch,
                &mut output,
                (!batchable).then_some(face),
            )?;
            scratch.corners.clear();
            scratch.inputs.clear();
        }
    }
    if batchable && !scratch.inputs.is_empty() {
        finish_mapping_batch(mesh, params, scratch, &mut output, None)?;
    }
    // Monotone sparse writes avoid inserting repeatedly into the middle of a
    // sorted attribute layer on imported meshes with arbitrary corner ordering.
    scratch
        .pending
        .sort_unstable_by_key(|&(corner, _)| corner.index());
    if !(params.mapping.projection.uses_existing()
        && params.options.source == params.options.target)
    {
        let sparse = mesh.attrs().sparse(params.options.target.key());
        let dense = mesh.attrs().dense(params.options.target.key());
        // Keep a selected-face edit proportional to the selection. Large
        // batches scan the sorted layer; small batches use indexed lookups.
        let mut existing = sparse
            .filter(|layer| scratch.pending.len() > layer.entries().len() / 64)
            .map(|layer| layer.entries().peekable());
        scratch.pending.retain(|&(corner, uv)| {
            let previous = if let Some(entries) = &mut existing {
                while entries
                    .peek()
                    .is_some_and(|&(index, _)| index < corner.index())
                {
                    entries.next();
                }
                entries
                    .peek()
                    .filter(|&&(index, _)| index == corner.index())
                    .map(|&(_, value)| *value)
            } else {
                sparse
                    .and_then(|layer| layer.get(corner.into()))
                    .or_else(|| dense.and_then(|layer| layer.get(corner.into())))
                    .copied()
            };
            let equal = previous.is_some_and(|old| old.map(f32::to_bits) == uv.map(f32::to_bits));
            output.corners_unchanged += u64::from(equal);
            !equal
        });
    }
    output.corners_written = scratch.pending.len() as u64;
    output.scratch_bytes = scratch.retained_bytes() as u64;
    Ok(output)
}

/// Authors UVs using reusable scratch. Validation and all angular recovery
/// complete before the first write. Unchanged values are not written or dirtied.
/// Writes occur in ascending corner-ID order. Finish the caller's edit session
/// to advance revision and obtain change output.
pub fn map_uvs_with_scratch<S: ChangeSink>(
    edit: &mut EditSession<'_, S>,
    params: &UvMappingParams,
    scratch: &mut UvAuthoringScratch,
) -> Result<UvOutput, UvError> {
    let output = prepare_mapping(edit.mesh(), params, scratch)?;
    op::set_corner_uvs(edit, params.options.target, &scratch.pending)
        .map_err(UvError::BulkWrite)?;
    Ok(output)
}

/// Validates and previews exactly the same mapping as authoring, without
/// changing a mesh or sink. `corners_written` predicts changed values and
/// `mapped_bounds` can feed `UvTransform::fit_bounds`. The preview owns no
/// replayable edit; authoring revalidates the current mesh when called.
pub fn preview_uvs(
    mesh: &Mesh,
    params: &UvMappingParams,
    scratch: &mut UvAuthoringScratch,
) -> Result<UvOutput, UvError> {
    prepare_mapping(mesh, params, scratch)
}

/// Authored coverage and finite bounds for a selected UV channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvInspection {
    /// Selected faces in canonical order.
    pub faces: FaceSet,
    /// Number of selected face corners.
    pub corners: u64,
    /// Corners carrying a UV value, including nonfinite values.
    pub authored_corners: u64,
    /// Corners without a UV value.
    pub missing_corners: u64,
    /// Authored corners with nonfinite values, in face-loop order.
    pub nonfinite_corners: Vec<CornerId>,
    /// Bounds of finite authored coordinates only.
    pub bounds: Option<UvBounds>,
}

/// Inspects authored UV coverage without projection, mutation or render extraction.
pub fn inspect_uvs(mesh: &Mesh, scope: &UvScope, set: UvSet) -> Result<UvInspection, UvError> {
    validate_set(mesh, set)?;
    let (faces, _) = selected_faces(mesh, scope);
    let mut result = UvInspection {
        faces,
        ..UvInspection::default()
    };
    for &face in &result.faces {
        if face == FaceId::OUTSIDE || mesh.face_edge(face).is_none() {
            return Err(UvError::InvalidFace { face });
        }
        for corner in mesh.face_loop(face) {
            result.corners += 1;
            match corner_uv(mesh, set, corner) {
                None => result.missing_corners += 1,
                Some(uv) => {
                    result.authored_corners += 1;
                    if uv.into_iter().all(f32::is_finite) {
                        include_bounds(&mut result.bounds, uv);
                    } else {
                        result.nonfinite_corners.push(corner);
                    }
                }
            }
        }
    }
    Ok(result)
}

/// Face selection scope for UV projection.
#[derive(Clone, Debug, PartialEq)]
pub enum UvScope {
    /// Process all faces in arena order.
    WholeMesh,
    /// Process an explicit face set (canonicalized before execution).
    FaceSet(FaceSet),
}

/// Projection plane mode.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UvPlane {
    /// Project from mesh-coordinate XY.
    WorldXY,
    /// Project from mesh-coordinate XZ.
    WorldXZ,
    /// Project from mesh-coordinate YZ.
    WorldYZ,
    /// Choose per-face dominant axis from geometry.
    PerFaceFromGeometry,
}

/// Parameters for [`project_planar`].
#[derive(Clone, Debug, PartialEq)]
pub struct UvPlanarParams {
    /// Face scope.
    pub scope: UvScope,
    /// Projection plane mode.
    pub plane: UvPlane,
    /// Uniform UV scale multiplier.
    pub scale: f32,
    /// UV offset after scale.
    pub offset: [f32; 2],
    /// When true, only writes missing corner UV values.
    pub write_missing_only: bool,
    /// Nonnegative, finite tolerance for the unnormalized face-normal sum.
    /// Unlike box projection, this legacy planar mode depends on face area.
    /// Equal dominant components prefer X, then Y, then Z.
    pub normal_epsilon: f32,
}

impl Default for UvPlanarParams {
    fn default() -> Self {
        Self {
            scope: UvScope::WholeMesh,
            plane: UvPlane::WorldXY,
            scale: 1.0,
            offset: [0.0, 0.0],
            write_missing_only: false,
            normal_epsilon: 1.0e-6,
        }
    }
}

/// Parameters for [`project_box`].
#[derive(Clone, Debug, PartialEq)]
pub struct UvBoxParams {
    /// Face scope.
    pub scope: UvScope,
    /// Uniform UV scale multiplier.
    pub scale: f32,
    /// UV offset after scale.
    pub offset: [f32; 2],
    /// When true, only writes missing corner UV values.
    pub write_missing_only: bool,
    /// Epsilon used by dominant-axis tie-breaking.
    ///
    /// Compared between components of the unit face normal, so plane
    /// selection does not depend on face size. Defaults to
    /// [`DEFAULT_BOX_NORMAL_EPSILON`], the value render extraction uses under
    /// `UvSource::CustomOrBoxProjected`.
    pub normal_epsilon: f32,
}

impl Default for UvBoxParams {
    fn default() -> Self {
        Self {
            scope: UvScope::WholeMesh,
            scale: 1.0,
            offset: [0.0, 0.0],
            write_missing_only: false,
            normal_epsilon: DEFAULT_BOX_NORMAL_EPSILON,
        }
    }
}

/// Cylinder axis selection.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CylinderAxis {
    /// Cylinder axis aligned with +X/-X.
    X,
    /// Cylinder axis aligned with +Y/-Y.
    Y,
    /// Cylinder axis aligned with +Z/-Z.
    Z,
}

/// Parameters for [`project_cylinder`].
#[derive(Clone, Debug, PartialEq)]
pub struct UvCylinderParams {
    /// Face scope.
    pub scope: UvScope,
    /// Cylinder axis.
    pub axis: CylinderAxis,
    /// Seam angle offset in radians.
    pub seam_offset_radians: f32,
    /// UV scale.
    ///
    /// The V coordinate is raw mesh-coordinate distance along the selected axis before
    /// scale and offset (it is not normalized to `[0, 1]`).
    pub scale: [f32; 2],
    /// UV offset.
    pub offset: [f32; 2],
    /// When true, only writes missing corner UV values.
    pub write_missing_only: bool,
}

impl Default for UvCylinderParams {
    fn default() -> Self {
        Self {
            scope: UvScope::WholeMesh,
            axis: CylinderAxis::Y,
            seam_offset_radians: 0.0,
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
            write_missing_only: false,
        }
    }
}

/// Refusal from UV projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UvError {
    /// A built-in UV name exists with an incompatible value type.
    IncompatibleLayer {
        /// Rejected channel; no writes were performed.
        set: UvSet,
    },
    /// A sorted kernel UV batch was refused before mutation.
    BulkWrite(op::SetCornerUvsError),
    /// A shared kernel mapping refused a parameter or coordinate value.
    Mapping {
        /// Offending corner, absent when parameters themselves are invalid.
        corner: Option<CornerId>,
        /// Typed mapping failure.
        error: UvMappingError,
    },
    /// Scale, offset, seam angle or normal tolerance is invalid.
    InvalidParameters,
    /// The face is outside, stale or missing.
    InvalidFace {
        /// Rejected face.
        face: FaceId,
    },
    /// A face corner has no destination vertex or position.
    InvalidCorner {
        /// Rejected corner.
        corner: CornerId,
    },
    /// Source coordinates or computed UVs are not finite.
    NumericLimit,
    /// A kernel write failed. Earlier eager writes may remain.
    Write(op::SetCornerUvError),
}
impl core::fmt::Display for UvError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IncompatibleLayer { set } => write!(f, "incompatible UV layer for {set:?}"),
            Self::BulkWrite(error) => write!(f, "UV batch write: {error}"),
            Self::Mapping { corner, error } => write!(f, "UV mapping at {corner:?}: {error}"),
            Self::InvalidParameters => f.write_str("invalid UV projection parameters"),
            Self::InvalidFace { face } => write!(f, "invalid UV projection face {face:?}"),
            Self::InvalidCorner { corner } => write!(f, "invalid UV projection corner {corner:?}"),
            Self::NumericLimit => f.write_str("UV projection exceeds numeric limits"),
            Self::Write(error) => write!(f, "UV write: {error}"),
        }
    }
}
impl core::error::Error for UvError {}

/// Completed UV projection and geometric fallback evidence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvOutput {
    /// Mapped corners already equal to their target values; no writes issued.
    pub corners_unchanged: u64,
    /// Corners lifted by an integer turn to preserve angular continuity.
    /// Includes geometric context retained for missing-only angular mapping.
    pub seam_corners_adjusted: u64,
    /// Corners with undefined angles filled from this face's defined neighbors.
    /// Includes geometric context retained for missing-only angular mapping.
    pub pole_corners_repaired: u64,
    /// Coarse periodic faces spanning more than half a turn after correction.
    pub wide_periodic_faces: FaceSet,
    /// Finite bounds of mapped values, excluding corners skipped by missing-only.
    pub mapped_bounds: Option<UvBounds>,
    /// Operation buffer capacity bytes, excluding mesh/report/allocator overhead.
    pub scratch_bytes: u64,
    /// Processed faces in deterministic selection order.
    pub faces: FaceSet,
    /// Successfully authored corner UV values.
    pub corners_written: u64,
    /// Existing values retained by `write_missing_only`.
    pub corners_skipped_existing: u64,
    /// The explicit selection was sorted or deduplicated.
    pub selections_canonicalized: bool,
    /// Faces with degenerate normals that used the fallback projection.
    /// Planar projection falls back to XY; box projection to positive Z.
    pub fallback_faces: FaceSet,
}

/// Authors planar UVs, preserving existing values when requested.
///
/// `PerFaceFromGeometry` selects an unsigned coordinate plane from the
/// unnormalized triangle-fan sum. A sum smaller than `normal_epsilon` falls
/// back to XY and is identified in the output. Fixed-plane projection does not
/// inspect normals.
pub fn project_planar<S: ChangeSink>(
    edit: &mut EditSession<'_, S>,
    params: &UvPlanarParams,
) -> Result<UvOutput, UvError> {
    validate_scale_offset([params.scale; 2], params.offset)?;
    validate_epsilon(params.normal_epsilon)?;
    project(
        edit,
        &params.scope,
        params.write_missing_only,
        |mesh, face| {
            if params.plane == UvPlane::PerFaceFromGeometry {
                dominant_plane(mesh, face, params.normal_epsilon)
            } else {
                Ok((params.plane, false))
            }
        },
        |position, plane| {
            let base = match plane {
                UvPlane::WorldXY => [position[0], position[1]],
                UvPlane::WorldXZ => [position[0], position[2]],
                UvPlane::WorldYZ => [position[1], position[2]],
                UvPlane::PerFaceFromGeometry => unreachable!("resolved projection plane"),
            };
            [
                base[0] * params.scale + params.offset[0],
                base[1] * params.scale + params.offset[1],
            ]
        },
    )
}

/// Authors the kernel's signed box projection.
///
/// With no existing UVs, the same scale and zero offset produce the same render
/// buffers as `UvSource::CustomOrBoxProjected` extraction. Existing UVs are
/// overwritten unless `write_missing_only` is true. Plane selection uses the
/// unit normal and is independent of face size.
pub fn project_box<S: ChangeSink>(
    edit: &mut EditSession<'_, S>,
    params: &UvBoxParams,
) -> Result<UvOutput, UvError> {
    validate_scale_offset([params.scale; 2], params.offset)?;
    validate_epsilon(params.normal_epsilon)?;
    project(
        edit,
        &params.scope,
        params.write_missing_only,
        |mesh, face| {
            Ok(exedra_mesh::dominant_box_plane(
                mesh,
                face,
                params.normal_epsilon,
            ))
        },
        |position, plane| {
            exedra_mesh::project_box_position(position, plane, params.scale, params.offset)
        },
    )
}

/// Authors cylindrical UVs around the selected coordinate axis.
///
/// Before scale and offset, U is a wrapped turn in `[0, 1)` and V is the raw
/// axial coordinate. For Y, U grows from +X toward +Z; X uses +Z toward +Y,
/// and Z uses +Y toward +X. This preserves the existing UV mapping convention.
pub fn project_cylinder<S: ChangeSink>(
    edit: &mut EditSession<'_, S>,
    params: &UvCylinderParams,
) -> Result<UvOutput, UvError> {
    validate_scale_offset(params.scale, params.offset)?;
    if !params.seam_offset_radians.is_finite() {
        return Err(UvError::InvalidParameters);
    }
    project(
        edit,
        &params.scope,
        params.write_missing_only,
        |_, _| Ok(((), false)),
        |p, ()| {
            let (radial_a, radial_b, axial) = match params.axis {
                CylinderAxis::X => (p[1], p[2], p[0]),
                CylinderAxis::Y => (p[2], p[0], p[1]),
                CylinderAxis::Z => (p[0], p[1], p[2]),
            };
            let theta = radial_a.atan2_ext(radial_b) + params.seam_offset_radians;
            let u = (theta / (core::f32::consts::PI * 2.0)).rem_euclid_ext(1.0);
            [
                u * params.scale[0] + params.offset[0],
                axial * params.scale[1] + params.offset[1],
            ]
        },
    )
}

fn validate_scale_offset(scale: [f32; 2], offset: [f32; 2]) -> Result<(), UvError> {
    if scale.into_iter().chain(offset).all(f32::is_finite) {
        Ok(())
    } else {
        Err(UvError::InvalidParameters)
    }
}
fn validate_epsilon(epsilon: f32) -> Result<(), UvError> {
    if epsilon.is_finite() && epsilon >= 0.0 {
        Ok(())
    } else {
        Err(UvError::InvalidParameters)
    }
}

fn project<S: ChangeSink, P: Copy>(
    edit: &mut EditSession<'_, S>,
    scope: &UvScope,
    missing_only: bool,
    resolve: impl Fn(&Mesh, FaceId) -> Result<(P, bool), UvError>,
    projection: impl Fn([f32; 3], P) -> [f32; 2],
) -> Result<UvOutput, UvError> {
    let mut output = UvOutput::default();
    match scope {
        UvScope::WholeMesh => output.faces = edit.mesh().faces().collect(),
        UvScope::FaceSet(faces) => {
            output.faces = faces.clone();
            output.selections_canonicalized = canonicalize_face_set(&mut output.faces);
        }
    }
    let mut pending = Vec::new();
    for &face in &output.faces {
        if face == FaceId::OUTSIDE || edit.mesh().face_edge(face).is_none() {
            return Err(UvError::InvalidFace { face });
        }
        // Normal resolution may consume even corners whose UVs will be kept.
        for corner in edit.mesh().face_loop(face) {
            corner_position(edit.mesh(), corner)?;
        }
        let (plane, fallback) = resolve(edit.mesh(), face)?;
        if fallback {
            output.fallback_faces.push(face);
        }
        for corner in edit.mesh().face_loop(face) {
            if missing_only && edit.corner_uv(corner).is_some() {
                output.corners_skipped_existing += 1;
                continue;
            }
            let uv = projection(corner_position(edit.mesh(), corner)?, plane);
            if !uv.into_iter().all(f32::is_finite) {
                return Err(UvError::NumericLimit);
            }
            pending.push((corner, uv));
        }
    }
    for (corner, uv) in pending {
        op::set_corner_uv(edit, corner, uv).map_err(UvError::Write)?;
        output.corners_written += 1;
    }
    Ok(output)
}

fn corner_position(mesh: &Mesh, corner: CornerId) -> Result<[f32; 3], UvError> {
    let position = mesh
        .to_vertex(corner)
        .and_then(|vertex| mesh.vertex_position(vertex))
        .copied()
        .ok_or(UvError::InvalidCorner { corner })?;
    if !exedra_math::finite(position) {
        return Err(UvError::NumericLimit);
    }
    Ok(position)
}

fn dominant_plane(mesh: &Mesh, face: FaceId, epsilon: f32) -> Result<(UvPlane, bool), UvError> {
    let mut corners = mesh.face_loop(face);
    let Some(first) = corners.next() else {
        return Ok((UvPlane::WorldXY, true));
    };
    let Some(second) = corners.next() else {
        return Ok((UvPlane::WorldXY, true));
    };
    let p0 = corner_position(mesh, first)?;
    let mut prev = corner_position(mesh, second)?;
    let mut seen = 2;
    let mut normal = [0.0_f32; 3];
    for corner in corners {
        seen += 1;
        let curr = corner_position(mesh, corner)?;
        let a = exedra_math::sub(prev, p0);
        let b = exedra_math::sub(curr, p0);
        normal[0] += a[1] * b[2] - a[2] * b[1];
        normal[1] += a[2] * b[0] - a[0] * b[2];
        normal[2] += a[0] * b[1] - a[1] * b[0];
        prev = curr;
    }
    if !exedra_math::finite(normal) {
        return Err(UvError::NumericLimit);
    }
    let [ax, ay, az] = normal.map(f32::abs);
    let max_axis = ax.max(ay).max(az);
    if seen < 3 || max_axis < epsilon {
        return Ok((UvPlane::WorldXY, true));
    }
    if ax + epsilon >= max_axis {
        return Ok((UvPlane::WorldYZ, false));
    }
    if ay + epsilon >= max_axis {
        return Ok((UvPlane::WorldXZ, false));
    }
    Ok((UvPlane::WorldXY, false))
}

#[cfg(test)]
mod tests;
