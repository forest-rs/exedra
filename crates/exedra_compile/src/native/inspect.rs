// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Borrowed constructor queries, without source installation or hashing.

use super::{NativeFamily, NativeGeometryRef, NativeSelectionError};
use exedra_analytic::{
    End,
    body::{Body, FaceId},
};
use exedra_constructive::chart::SurfaceChartKind;
use exedra_math::Placement3;

impl NativeGeometryRef<'_> {
    /// Original construction family, preserving its typed metadata.
    #[must_use]
    pub const fn family(self) -> NativeFamily {
        match self {
            Self::Extrusion(_) => NativeFamily::Extrusion,
            Self::Revolution(_) => NativeFamily::Revolution,
            Self::Sweep(_) => NativeFamily::Sweep,
            Self::Loft(_) => NativeFamily::Loft,
            Self::BoredPlate(_) => NativeFamily::BoredPlate,
        }
    }
    /// Complete occurrence placement, separate from body-local geometry.
    #[must_use]
    pub fn frame(self) -> Placement3 {
        match self {
            Self::Extrusion(v) => v.frame(),
            Self::Revolution(v) => v.frame(),
            Self::Sweep(v) => v.frame(),
            Self::Loft(v) => v.frame(),
            Self::BoredPlate(v) => v.frame(),
        }
    }
    /// Structural chart operation available for this constructor, if any.
    /// Metric/transform preflight and actual coordinate representability remain
    /// separate checks. This query allocates and hashes nothing.
    #[must_use]
    pub const fn chart_kind(self) -> Option<SurfaceChartKind> {
        match self {
            Self::Revolution(_) => Some(SurfaceChartKind::Revolve),
            Self::Sweep(_) => Some(SurfaceChartKind::Sweep),
            Self::Loft(_) => Some(SurfaceChartKind::Loft),
            Self::Extrusion(_) | Self::BoredPlate(_) => None,
        }
    }
    /// Current construction cap, if present, without installing a native source.
    /// Returned IDs are local to this borrowed constructor; scoped selections
    /// require installation in a [`super::NativeSource`].
    #[must_use]
    pub fn cap(self, end: End) -> Option<FaceId> {
        match self {
            Self::Extrusion(v) => Some(v.cap(end)),
            Self::Revolution(v) => v.cap(end),
            Self::Sweep(v) => v.cap(end),
            Self::Loft(v) => v.cap(end),
            Self::BoredPlate(v) => Some(v.source().cap(end)),
        }
    }
    /// Exact oriented supporting-plane frame in occurrence coordinates.
    /// The datum is parameter zero; trims do not reposition it. This borrows
    /// native geometry directly, without hashing or meshing transient models.
    ///
    /// # Errors
    /// Returns typed unknown-face, curved-support or numeric-limit diagnoses.
    pub fn planar_frame(self, face: FaceId) -> Result<Placement3, NativeSelectionError> {
        fn local<F, E>(
            body: &Body<F, E>,
            face: FaceId,
        ) -> Result<Placement3, NativeSelectionError> {
            if !matches!(
                body.face(face)
                    .ok_or(NativeSelectionError::UnknownFace { face })?
                    .surface,
                exedra_analytic::geometry::Surface::Plane(_)
            ) {
                return Err(NativeSelectionError::NotPlanar { face });
            }
            body.planar_frame(face, [0.0; 2])
                .ok_or(NativeSelectionError::PlaneNumericLimit { face })
        }
        let plane = match self {
            NativeGeometryRef::Extrusion(v) => local(v.body(), face),
            NativeGeometryRef::Revolution(v) => local(v.body(), face),
            NativeGeometryRef::Sweep(v) => local(v.body(), face),
            NativeGeometryRef::Loft(v) => local(v.body(), face),
            NativeGeometryRef::BoredPlate(v) => local(v.body(), face),
        }?;
        let frame = self.frame() * plane;
        if frame.rows.iter().flatten().any(|v| !v.is_finite()) {
            return Err(NativeSelectionError::PlaneNumericLimit { face });
        }
        Ok(frame)
    }
}
