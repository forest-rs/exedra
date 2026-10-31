// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Independent checks beyond local curve/support agreement.
//!
//! [`Body::validate_boundary`] checks oriented edge pairing, unique loop ownership
//! and connected vertex links, then checks supported face trim arrangements.
//! Its report distinguishes unsupported arrangements from checked ones. Neither
//! this report nor closed incidence proves that distant faces do not intersect,
//! or that nested shells bound correctly oriented material. No mesh is created.

mod convex;
#[cfg(test)]
mod tests;
mod topology;
mod trims;
pub use convex::{ConvexSolidError, ConvexSolidReport};

use crate::body::{
    Body, BodyError, EdgeId, FaceId, LoopId, ValidationPolicy, ValidationReport, VertexId,
};
use alloc::vec::Vec;

/// Geometric agreement, chart separation and work limits for boundary inspection.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoundaryValidationPolicy {
    /// Maximum local 3D support/correspondence gap, in model units.
    pub max_gap: f64,
    /// Strict minimum separation in native chart units for checked arrangements.
    /// Cylinder U is radians and V is a model length; no texture UVs are involved.
    pub chart_separation: f64,
    /// Maximum edge occurrences admitted before allocating topology scratch.
    /// The combined native-table scratch count is also limited to four times
    /// this value, including detached entries that retain historic identities.
    pub max_coedges: usize,
    /// Maximum pair work per phase, admitted before arrangement checks or the
    /// convex certificate's vertex-pair/halfspace checks.
    pub max_pair_tests: usize,
}
impl Default for BoundaryValidationPolicy {
    fn default() -> Self {
        Self {
            max_gap: 1e-9,
            chart_separation: 1e-8,
            max_coedges: 1_000_000,
            max_pair_tests: 10_000_000,
        }
    }
}

/// A continuous face-arrangement proof used by this inspection.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrimProof {
    /// Straight chart loops checked for winding, crossings, separation and nesting.
    Polygon,
    /// Planar polygons/full ellipses checked using whole-curve support bounds.
    PlanarEllipses,
    /// One cylinder chart bounded by two separated harmonic graphs and two seams.
    CylindricalGraphs,
}

/// Why a valid local boundary did not receive an arrangement certificate.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedTrim {
    /// Nonplanar face has no complete explicit chart correspondence.
    MissingChart,
    /// The chart curve family or loop combination is outside the checked subset.
    CurveFamily,
    /// Bounds could not prove strict separation; this does not establish a crossing.
    SeparationUnresolved,
    /// Finite arithmetic cannot resolve the requested strict separation.
    NumericLimit,
}

/// Arrangement evidence for one face; unsupported never means valid.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrimValidation {
    /// Complete arrangement check succeeded using a continuous proof.
    Certified(TrimProof),
    /// Local support validation succeeded, but arrangement validity remains unknown.
    Unsupported(UnsupportedTrim),
}

/// One connected component of the topological boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryShell {
    /// Native face identities in this component.
    pub faces: Vec<FaceId>,
    /// Distinct used native vertices.
    pub vertices: usize,
    /// Used noncollapsed edges; closed curves count as one edge.
    pub regular_edges: usize,
    /// Euler characteristic including inner face loops and collapsed poles.
    pub euler_characteristic: i64,
}

/// Checked topology and face-by-face arrangement evidence, never a global solid claim.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryValidationReport {
    /// Existing local incidence and complete support/correspondence evidence.
    pub geometry: ValidationReport,
    /// Connected boundary components, in first-face order.
    pub shells: Vec<BoundaryShell>,
    /// Arrangement status indexed by native face ID.
    pub face_trims: Vec<TrimValidation>,
    /// Actual used regular and collapsed edge occurrences.
    pub coedges: usize,
    /// Conservative trim-pair count admitted before checking any arrangement.
    pub pair_test_bound: usize,
}
impl BoundaryValidationReport {
    /// Whether every face received an arrangement certificate.
    /// This still does not certify distant face intersections or shell containment.
    #[must_use]
    pub fn all_trims_checked(&self) -> bool {
        self.face_trims
            .iter()
            .all(|status| matches!(status, TrimValidation::Certified(_)))
    }
}

/// A demonstrated local boundary violation or explicit work refusal.
#[derive(Clone, Debug, PartialEq)]
pub enum BoundaryValidationError {
    /// Invalid nonfinite/nonpositive separation or zero work budget.
    InvalidPolicy,
    /// Existing local support, references or explicit correspondence failed.
    Geometry(BodyError),
    /// Empty bodies do not represent a closed boundary.
    Empty,
    /// Work was refused before the associated expensive checks.
    WorkLimit {
        /// Required work, absent when its count overflowed.
        required: Option<usize>,
        /// Caller limit.
        maximum: usize,
    },
    /// A used loop must have exactly one owning face.
    LoopOwnership {
        /// Native loop.
        loop_id: LoopId,
        /// Number of owning faces.
        owners: usize,
    },
    /// A used regular edge needs one occurrence in each direction.
    EdgeIncidence {
        /// Native edge.
        edge: EdgeId,
        /// Forward/reversed occurrence counts.
        uses: [usize; 2],
    },
    /// A vertex's local edge-end link has multiple disconnected cycles.
    DisconnectedVertexLink {
        /// Pinched native vertex.
        vertex: VertexId,
    },
    /// A loop consisting entirely of collapsed boundaries does not bound a face.
    CollapsedLoop {
        /// Owning face.
        face: FaceId,
        /// Degenerate loop.
        loop_id: LoopId,
    },
    /// A straight chart arrangement has wrong winding, contact or invalid nesting.
    PolygonArrangement {
        /// Native face.
        face: FaceId,
        /// Structured profile-arrangement diagnosis.
        error: exedra_curves::profile::ProfileError,
    },
    /// A chart loop has a collapsed or otherwise invalid affine segment chain.
    ChartLoop {
        /// Native face.
        face: FaceId,
        /// Structured loop diagnosis.
        error: exedra_curves::profile::LoopError,
    },
    /// An inferred legacy planar trim lies outside its declared face domain.
    ChartDomain {
        /// Native face.
        face: FaceId,
        /// Boundary loop excluded by the face domain.
        loop_id: LoopId,
    },
    /// A supported closed ellipse or polygon loop has the wrong traversal.
    Winding {
        /// Native face.
        face: FaceId,
        /// Offending loop.
        loop_id: LoopId,
    },
    /// A hole lies outside the outer boundary or inside another hole.
    HoleContainment {
        /// Native face.
        face: FaceId,
        /// Offending inner loop.
        loop_id: LoopId,
    },
    /// A whole harmonic cylinder band has no strictly positive axial width.
    CylinderWidth {
        /// Native face.
        face: FaceId,
        /// Conservative minimum upper-minus-lower height.
        minimum: f64,
    },
}
impl core::fmt::Display for BoundaryValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "analytic boundary validation: {self:?}")
    }
}
impl core::error::Error for BoundaryValidationError {}

impl<F, E> Body<F, E> {
    /// Checks a closed oriented 2-manifold boundary and supported face arrangements.
    ///
    /// Detached vertices/edges/loops remain permitted, as in [`Self::validate`].
    /// Used loops have one owner; used regular edges pair in opposite directions;
    /// each used vertex has one connected edge-end link. Periodic seams and
    /// certified collapsed poles retain their ordinary native identities.
    /// Unsupported arrangements are explicit in the returned report. No claim
    /// is made about global self-intersection, outward shell orientation or material.
    ///
    /// # Errors
    /// Returns a demonstrated topology/arrangement violation or a work refusal.
    pub fn validate_boundary(
        &self,
        policy: &BoundaryValidationPolicy,
    ) -> Result<BoundaryValidationReport, BoundaryValidationError> {
        use BoundaryValidationError as Error;
        if !policy.max_gap.is_finite()
            || policy.max_gap < 0.0
            || !policy.chart_separation.is_finite()
            || policy.chart_separation <= 0.0
            || policy.max_coedges == 0
            || policy.max_pair_tests == 0
        {
            return Err(Error::InvalidPolicy);
        }
        let coedges = self
            .faces()
            .iter()
            .flat_map(|f| &f.loops)
            .try_fold(0_usize, |n, id| {
                n.checked_add(self.boundary_loop(*id).map_or(0, |l| l.coedges.len()))
            });
        if coedges.is_none_or(|n| n > policy.max_coedges) {
            return Err(Error::WorkLimit {
                required: coedges,
                maximum: policy.max_coedges,
            });
        }
        let scratch = self
            .edges()
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(self.vertices().len()))
            .and_then(|n| n.checked_add(self.loops().len()))
            .and_then(|n| {
                self.faces()
                    .len()
                    .checked_mul(2)
                    .and_then(|f| n.checked_add(f))
            });
        let maximum = policy.max_coedges.saturating_mul(4);
        if scratch.is_none_or(|n| n > maximum) {
            return Err(Error::WorkLimit {
                required: scratch,
                maximum,
            });
        }
        let geometry = self
            .validate(&ValidationPolicy {
                max_gap: policy.max_gap,
            })
            .map_err(Error::Geometry)?;
        let shells = topology::check(self)?;
        let pair_test_bound = self.faces().iter().try_fold(0_usize, |sum, face| {
            let n = face
                .loops
                .iter()
                .map(|id| self.loops()[id.index()].coedges.len().max(4))
                .sum::<usize>();
            n.checked_mul(n).and_then(|n| sum.checked_add(n))
        });
        if pair_test_bound.is_none_or(|n| n > policy.max_pair_tests) {
            return Err(Error::WorkLimit {
                required: pair_test_bound,
                maximum: policy.max_pair_tests,
            });
        }
        // One indexed view avoids rescanning/cloning every explicit trim table
        // for each face and occurrence in patterned machined bodies.
        let mut trim_index = alloc::vec![None; self.loops().len()];
        for record in self.trims() {
            trim_index[record.loop_id.index()] = Some(record);
        }
        let face_trims = (0..self.faces().len())
            .map(|i| trims::check(self, FaceId(crate::usize_to_u32(i)), policy, &trim_index))
            .collect::<Result<_, _>>()?;
        Ok(BoundaryValidationReport {
            geometry,
            shells,
            face_trims,
            coedges: coedges.expect("bounded count"),
            pair_test_bound: pair_test_bound.expect("bounded count"),
        })
    }
}
