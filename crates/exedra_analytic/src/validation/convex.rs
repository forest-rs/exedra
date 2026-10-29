// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    BoundaryValidationError, BoundaryValidationPolicy, BoundaryValidationReport, TrimProof,
    TrimValidation,
};
use crate::body::{Body, FaceId, VertexId};
use crate::geometry::Surface;
use alloc::{vec, vec::Vec};
use exedra_math::{cross, dot, norm, scale, sub};

/// A checked single convex planar shell within the explicit model-gap allowance.
///
/// Every face is a simple convex polygon with no holes, all used vertices lie
/// within every outward supporting halfspace, and an interior witness lies
/// strictly behind every plane. The boundary has a connected manifold vertex
/// link everywhere and sphere topology. This is a stronger, bounded subset than
/// general curved-solid validity; no meshing or sampled ray classification is used.
#[derive(Clone, Debug, PartialEq)]
pub struct ConvexSolidReport {
    /// Complete topology and trim-arrangement evidence.
    pub boundary: BoundaryValidationReport,
    /// Strictly interior point, the mean of the used native vertices.
    pub interior: [f64; 3],
    /// Smallest signed distance from that point to an outward supporting plane,
    /// after conservative arithmetic allowance.
    pub interior_clearance: f64,
    /// Largest positive vertex/halfspace disagreement, bounded by `max_gap`.
    pub max_halfspace_gap: f64,
    /// Positive signed volume from oriented planar triangle integrals.
    pub volume: f64,
    /// Used-vertex pairs checked for coincident geometric identities.
    pub vertex_pair_tests: usize,
    /// Used vertices checked against outward supporting halfspaces.
    pub vertex_halfspace_tests: usize,
}
/// Why the single convex planar solid subset could not be established.
#[derive(Clone, Debug, PartialEq)]
pub enum ConvexSolidError {
    /// Independent local boundary validation failed.
    Boundary(BoundaryValidationError),
    /// The body has more than one boundary shell, including possible cavities.
    ShellCount {
        /// Observed connected boundary components.
        found: usize,
    },
    /// A convex solid boundary must have Euler characteristic two.
    EulerCharacteristic {
        /// Observed characteristic.
        found: i64,
    },
    /// Only plane-supported simple polygon faces without inner loops are admitted.
    UnsupportedFace {
        /// Native face outside this certificate's subset.
        face: FaceId,
    },
    /// An oriented face polygon has a reflex corner.
    NonconvexFace {
        /// Native face.
        face: FaceId,
    },
    /// A native vertex lies beyond an outward supporting plane.
    Halfspace {
        /// Supporting face.
        face: FaceId,
        /// Native vertex outside it.
        vertex: VertexId,
        /// Signed distance including conservative arithmetic allowance.
        bound: f64,
        /// Caller model-gap allowance.
        allowed: f64,
    },
    /// Distinct native identities cannot establish distinct geometric vertices.
    CoincidentVertices {
        /// First used vertex.
        first: VertexId,
        /// Second used vertex.
        second: VertexId,
    },
    /// No strict interior witness or positive finite volume could be established.
    NoInterior,
}
impl core::fmt::Display for ConvexSolidError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "convex analytic solid validation: {self:?}")
    }
}
impl core::error::Error for ConvexSolidError {}

impl<F, E> Body<F, E> {
    /// Validates one outward-oriented convex planar solid within `policy.max_gap`.
    ///
    /// All used vertices are checked against every supporting halfspace. Plane
    /// orientation, convex face turns, sphere topology, strict interior clearance
    /// and positive volume are checked independently of constructor history.
    /// The certificate permits the same bounded geometric gaps as the input
    /// boundary policy; it does not move, weld or heal the native boundary.
    ///
    /// # Errors
    /// Refuses curved/nonconvex/multiple-shell bodies and unresolved finite arithmetic.
    pub fn validate_convex_solid(
        &self,
        policy: &BoundaryValidationPolicy,
    ) -> Result<ConvexSolidReport, ConvexSolidError> {
        use ConvexSolidError as Error;
        let boundary = self.validate_boundary(policy).map_err(Error::Boundary)?;
        if boundary.shells.len() != 1 {
            return Err(Error::ShellCount {
                found: boundary.shells.len(),
            });
        }
        let shell = &boundary.shells[0];
        if shell.euler_characteristic != 2 {
            return Err(Error::EulerCharacteristic {
                found: shell.euler_characteristic,
            });
        }
        let mut used = vec![false; self.vertices().len()];
        let mut polygons = Vec::with_capacity(self.faces().len());
        for (i, f) in self.faces().iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            if !matches!(f.surface, Surface::Plane(_))
                || f.loops.len() != 1
                || boundary.face_trims[i] != TrimValidation::Certified(TrimProof::Polygon)
            {
                return Err(Error::UnsupportedFace { face: id });
            }
            let ring = &self.loops()[f.loops[0].index()];
            let polygon = ring
                .coedges
                .iter()
                .map(|co| {
                    let v = self.coedge_vertices(*co).expect("validated coedge")[0];
                    used[v.index()] = true;
                    self.vertices()[v.index()]
                })
                .collect::<Vec<_>>();
            polygons.push(polygon);
        }
        let used_vertices = used
            .iter()
            .enumerate()
            .filter_map(|(i, used)| used.then_some(i))
            .collect::<Vec<_>>();
        let count = used_vertices.len();
        let pair_tests = count.checked_mul(count.saturating_sub(1)).map(|n| n / 2);
        let halfspace_tests = count.checked_mul(self.faces().len());
        let work = pair_tests.and_then(|p| halfspace_tests.and_then(|h| p.checked_add(h)));
        if work.is_none_or(|n| n > policy.max_pair_tests) {
            return Err(Error::Boundary(BoundaryValidationError::WorkLimit {
                required: work,
                maximum: policy.max_pair_tests,
            }));
        }
        for (i, &a) in used_vertices.iter().enumerate() {
            for &b in &used_vertices[i + 1..] {
                let magnitude = self.vertices()[a]
                    .into_iter()
                    .chain(self.vertices()[b])
                    .map(f64::abs)
                    .fold(1.0, f64::max);
                if norm(sub(self.vertices()[a], self.vertices()[b]))
                    <= policy.max_gap + 512.0 * f64::EPSILON * magnitude
                {
                    return Err(Error::CoincidentVertices {
                        first: VertexId(crate::usize_to_u32(a)),
                        second: VertexId(crate::usize_to_u32(b)),
                    });
                }
            }
        }
        let origin = self.vertices()[used
            .iter()
            .position(|used| *used)
            .ok_or(Error::NoInterior)?];
        // Translation-relative accumulation avoids losing a thin solid's witness
        // when its world-space placement is much larger than its dimensions.
        let mut mean = [0.0; 3];
        let mut count = 0.0;
        for (p, used) in self.vertices().iter().zip(&used) {
            if *used {
                count += 1.0;
                let delta = sub(*p, origin);
                for axis in 0..3 {
                    mean[axis] += (delta[axis] - mean[axis]) / count;
                }
            }
        }
        let interior = exedra_math::add(origin, mean);
        let mut interior_clearance = f64::INFINITY;
        let mut max_halfspace_gap = 0.0_f64;
        let mut volume = 0.0;
        for (i, (f, polygon)) in self.faces().iter().zip(&polygons).enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            let frame = f.surface.frame();
            let n = cross(frame.rows.map(|r| r[0]), frame.rows.map(|r| r[1]));
            let normal = scale(n, f.sense.sign() / norm(n));
            let anchor = frame.rows.map(|r| r[3]);
            let magnitude = anchor
                .into_iter()
                .chain(interior)
                .map(f64::abs)
                .fold(1.0, f64::max);
            let error = 512.0 * f64::EPSILON * magnitude;
            let clearance = -dot(sub(interior, anchor), normal) - error;
            interior_clearance = interior_clearance.min(clearance);
            for (v, (p, used)) in self.vertices().iter().zip(&used).enumerate() {
                if !*used {
                    continue;
                }
                let bound = dot(sub(*p, anchor), normal) + error;
                max_halfspace_gap = max_halfspace_gap.max(bound.max(0.0));
                if !bound.is_finite() || bound > policy.max_gap {
                    return Err(Error::Halfspace {
                        face: id,
                        vertex: VertexId(crate::usize_to_u32(v)),
                        bound,
                        allowed: policy.max_gap,
                    });
                }
            }
            for corner in 0..polygon.len() {
                let a = polygon[corner];
                let b = polygon[(corner + 1) % polygon.len()];
                let c = polygon[(corner + 2) % polygon.len()];
                let turn = dot(cross(sub(b, a), sub(c, b)), normal);
                if !turn.is_finite() || turn < -error * (norm(sub(b, a)) + norm(sub(c, b))) {
                    return Err(Error::NonconvexFace { face: id });
                }
            }
            for j in 1..polygon.len() - 1 {
                volume += dot(
                    sub(polygon[0], interior),
                    cross(sub(polygon[j], interior), sub(polygon[j + 1], interior)),
                ) / 6.0;
            }
        }
        if !interior_clearance.is_finite()
            || interior_clearance <= policy.max_gap
            || !volume.is_finite()
            || volume <= 0.0
        {
            return Err(Error::NoInterior);
        }
        Ok(ConvexSolidReport {
            boundary,
            interior,
            interior_clearance,
            max_halfspace_gap,
            volume,
            vertex_pair_tests: pair_tests.expect("bounded work"),
            vertex_halfspace_tests: halfspace_tests.expect("bounded work"),
        })
    }
}
