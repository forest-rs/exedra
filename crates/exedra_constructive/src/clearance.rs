// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Planar mounting-footprint clearance against evaluated face boundaries.
//!
//! This measures the polygonal boundary stored in the evaluated mesh, projected
//! into a checked workplane. It does not recover an analytic curve, account for
//! a different tessellation's error, measure wall thickness, or certify a solid.
//! A circular footprint is contained exactly when its center is inside material
//! and its distance to every boundary exceeds its radius (subject to arithmetic
//! and the caller's decision tolerance). Holes are material exclusions.
//! Polygon queries additionally inspect full boundary intervals and excluded
//! holes enclosed by the footprint, returning separate containment evidence.

use crate::{
    tessellate::{Feature, TessellatedBody},
    workplane::{Workplane, WorkplaneError},
};
use exedra_math::Placement3;
use exedra_mesh::{FaceId, HalfEdgeId};
use exedra_mesh_ops::clearance as geometry;
pub use geometry::{
    BoundaryPolicy, BoundaryStats, ClearanceDecision, FootprintWitness, PolygonClearanceError,
    PolygonClearancePolicy,
};
/// Filled polygon containment and distance with constructive boundary evidence.
pub type PolygonClearance = geometry::PolygonClearance<BoundarySource>;
/// A polygon containment violation with constructive boundary evidence.
pub type PolygonViolation = geometry::PolygonViolation<BoundarySource>;

/// Provenance of a boundary segment, scoped to the source body snapshot.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BoundarySource {
    /// Face on the selected side of this boundary.
    pub face: FaceId,
    /// Selected-side half-edge in the original evaluated mesh.
    pub edge: HalfEdgeId,
    /// Feature of that selected face.
    pub feature: Feature,
    /// Feature across the boundary, or `None` for an open mesh edge.
    pub adjacent_feature: Option<Feature>,
}

/// A failed boundary extraction or circular-footprint query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClearanceError {
    /// Invalid limits, nonfinite coordinates, negative radius, or invalid requirement.
    InvalidInput,
    /// The workplane does not match its source mesh revision.
    Workplane(WorkplaneError),
    /// Source features no longer match the mesh revision.
    StaleSource,
    /// The patch has no valid, simple material boundary with consistent nesting.
    InvalidBoundary,
    /// Two nonadjacent segments cross, touch, or violate minimum separation.
    BoundaryContact {
        /// First offending source segment.
        first: HalfEdgeId,
        /// Second offending source segment.
        second: HalfEdgeId,
    },
    /// The selected topology exceeds an explicit work budget.
    BudgetExceeded,
    /// Projected geometry or arithmetic cannot be represented reliably.
    NumericLimit,
}
impl core::fmt::Display for ClearanceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("invalid planar-clearance inputs or policy"),
            Self::Workplane(error) => write!(f, "clearance workplane: {error}"),
            Self::StaleSource => f.write_str("clearance source map is stale"),
            Self::InvalidBoundary => {
                f.write_str("patch boundary is empty, degenerate, or incorrectly nested")
            }
            Self::BoundaryContact { first, second } => write!(
                f,
                "patch boundary edges {first:?} and {second:?} cross or touch within tolerance"
            ),
            Self::BudgetExceeded => f.write_str("planar-clearance boundary work budget exceeded"),
            Self::NumericLimit => f.write_str("planar-clearance geometry exceeds numeric limits"),
        }
    }
}
impl core::error::Error for ClearanceError {}

impl From<geometry::ClearanceError> for ClearanceError {
    fn from(error: geometry::ClearanceError) -> Self {
        use geometry::ClearanceError as E;
        match error {
            E::InvalidInput => Self::InvalidInput,
            E::Workplane(error) => Self::Workplane(error),
            E::InvalidBoundary => Self::InvalidBoundary,
            E::BoundaryContact { first, second } => Self::BoundaryContact { first, second },
            E::BudgetExceeded => Self::BudgetExceeded,
            E::NumericLimit => Self::NumericLimit,
        }
    }
}

/// Oriented checked boundary carrying constructive feature evidence.
pub type PatchLoop = geometry::PatchLoop<BoundarySource>;
/// Nearest boundary location with constructive feature evidence.
pub type BoundaryWitness = geometry::BoundaryWitness<BoundarySource>;
/// Signed circular-footprint margin with constructive feature evidence.
/// Classification returns a geometric [`geometry::ClearanceError`].
pub type CircleClearance = geometry::CircleClearance<BoundarySource>;

/// Immutable checked planar domain with snapshot-scoped feature provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanarPatch {
    geometry: geometry::PlanarPatch<BoundarySource>,
}
impl PlanarPatch {
    /// Extracts the selected boundary and binds its original face features.
    ///
    /// The frame and body must belong to the same logical source. Revision
    /// counters cannot identify unrelated meshes. The result owns its geometry
    /// and remains queryable independently of subsequent edits to the body.
    pub fn from_workplane(
        body: &TessellatedBody,
        workplane: &Workplane,
        policy: &BoundaryPolicy,
    ) -> Result<Self, ClearanceError> {
        workplane
            .check(&body.mesh)
            .map_err(ClearanceError::Workplane)?;
        body.source_map
            .check(&body.mesh)
            .map_err(|_| ClearanceError::StaleSource)?;
        let geometry = geometry::PlanarPatch::from_workplane(&body.mesh, workplane, policy)?
            .try_map_sources(|source| {
                Ok::<_, ClearanceError>(BoundarySource {
                    face: source.face,
                    edge: source.edge,
                    feature: body
                        .source_map
                        .face_feature(source.face)
                        .ok_or(ClearanceError::InvalidBoundary)?,
                    adjacent_feature: source
                        .adjacent_face
                        .and_then(|face| body.source_map.face_feature(face)),
                })
            })?;
        Ok(Self { geometry })
    }
    /// Workplane-to-body placement for interpreting measurements and witnesses.
    #[must_use]
    pub fn frame(&self) -> Placement3 {
        self.geometry.frame()
    }
    /// Outer boundary first, followed by holes in deterministic order.
    #[must_use]
    pub fn loops(&self) -> &[PatchLoop] {
        self.geometry.loops()
    }
    /// Boundary construction work, separate from subsequent query work.
    #[must_use]
    pub fn stats(&self) -> BoundaryStats {
        self.geometry.stats()
    }
    /// Maximum corner deviation from the workplane before projection.
    #[must_use]
    pub fn max_plane_deviation(&self) -> f64 {
        self.geometry.max_plane_deviation()
    }
    /// Measures projected area, perimeter, centroid and bounds without traversing
    /// mesh topology or allocating. Holes subtract area and add perimeter.
    pub fn measure(
        &self,
    ) -> Result<crate::measure::PlanarMeasurements, crate::measure::MeasurementError> {
        self.geometry.measure()
    }
    /// Measures a filled polygon against this snapshot, including concave edges
    /// and enclosed support holes. Points and witnesses use patch-local units.
    /// See [`geometry::PlanarPatch::polygon_clearance`] for contact and budget semantics.
    pub fn polygon_clearance(
        &self,
        footprint: &[[f64; 2]],
        policy: &PolygonClearancePolicy,
    ) -> Result<PolygonClearance, PolygonClearanceError> {
        self.geometry.polygon_clearance(footprint, policy)
    }

    /// Measures a circular footprint against every boundary, including holes.
    /// Coordinates and radius use patch-local units. Work is linear in boundary
    /// edges, without allocation or tessellation. Invalid inputs are refused.
    pub fn circle_clearance(
        &self,
        center: [f64; 2],
        radius: f64,
    ) -> Result<CircleClearance, ClearanceError> {
        self.geometry
            .circle_clearance(center, radius)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
