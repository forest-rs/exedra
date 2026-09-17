// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Direct planar, box and cylinder UV authoring in mesh coordinates.
//!
//! Projection resolves every selected face and pending UV before writing.
//! Invalid selections, parameters and nonfinite results leave the source untouched.
//! Writes use the caller's eager edit session and change sink; a kernel write
//! failure is returned explicitly and does not roll back earlier writes.

use crate::math::FloatExt;
use crate::selection::{FaceSet, canonicalize_face_set};
use alloc::vec::Vec;
use exedra_mesh::{
    ChangeSink, CornerId, DEFAULT_BOX_NORMAL_EPSILON, EditSession, FaceId, Mesh, op,
};

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UvOutput {
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
