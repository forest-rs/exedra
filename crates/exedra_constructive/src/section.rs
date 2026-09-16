// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Plane sections and capped cuts of evaluated, closed meshes.
//!
//! Cuts follow robust face triangulation, including the chosen diagonals of
//! nonplanar faces. They do not reconstruct an analytic surface. Planes use the
//! body's coordinates. Contacts within tolerance are refused rather than snapped.
//! Section frames are deterministic and right-handed; local +Z is the plane normal.
//! Omitted collinear face-boundary samples are restored only when collinear in
//! 3D, preserving the triangulated surface. Both original and f32-realized
//! section loops must retain separation, winding, and nesting.
//!
//! ```
//! use exedra_constructive::{
//!     builders::rect,
//!     ir::{CapMode, Placement3, Plane3},
//!     section::{CutCap, SectionPolicy, split_body},
//!     tessellate::{EvalPolicy, tessellate_extrude},
//! };
//! let body = tessellate_extrude(&rect(2.0, 3.0)?, &Placement3::IDENTITY,
//!     4.0, CapMode::Both, &EvalPolicy::default())?;
//! let cut = split_body(&body,
//!     Plane3 { normal: [0.3, -0.2, 1.0], distance: 1.71 },
//!     &SectionPolicy::default(), CutCap { region: 100, material: None })?;
//! assert!(cut.negative.is_some() && cut.positive.is_some());
//! assert_eq!(cut.section.regions.len(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod emit;
mod prepare;
mod triangulate;
use emit::{cap_triangles, emit};
use prepare::prepare;

use crate::ir::{Placement3, Plane3, SlotId};
use crate::tessellate::{Feature, TessellatedBody};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use exedra_math::{cross, dot, narrow, normalize, scale, sub};
use exedra_mesh::{FaceBuildAttrs, FaceTriangulation, MeshBuilder};

/// Accuracy and finite work limits for plane operations.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SectionPolicy {
    /// Body-space distance: vertices this close to the plane are ambiguous.
    /// Emitted cut vertices must also remain within this distance after f32 storage.
    pub distance_tolerance: f64,
    /// Maximum input triangles, before clipping.
    pub max_triangles: u32,
    /// Maximum distinct section vertices, including triangulation-diagonal crossings.
    pub max_section_vertices: u32,
    /// Maximum budgeted segment-pair and containment work, shared by original
    /// and f32-realized section validation.
    pub max_pair_checks: u64,
}
impl Default for SectionPolicy {
    fn default() -> Self {
        Self {
            distance_tolerance: 1e-6,
            max_triangles: 1_000_000,
            max_section_vertices: 8192,
            max_pair_checks: 16_000_000,
        }
    }
}

/// Authored attributes of newly generated cap faces.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CutCap {
    /// Region assigned to every cap triangle. Choose a value distinct from input
    /// regions when region-boundary selection should identify the rim.
    pub region: u32,
    /// Optional material-slot override. `None` inherits the body's occurrence material.
    pub material: Option<SlotId>,
}

/// One oriented section boundary in section-frame XY coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionLoop {
    /// Cyclic points, without a duplicate closing point; CCW outer, CW hole.
    pub points: Vec<[f64; 2]>,
    /// Source face feature for each edge from point `i` to point `(i+1) % len`.
    pub edge_features: Vec<Feature>,
}
/// One connected filled section, with its directly enclosed holes.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionRegion {
    /// Counter-clockwise outer boundary.
    pub outer: SectionLoop,
    /// Clockwise hole boundaries.
    pub holes: Vec<SectionLoop>,
}
/// Deterministic work and realization measurements.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SectionStats {
    /// Number of input triangles inspected.
    pub input_triangles: u32,
    /// Input triangles straddling the plane.
    pub split_triangles: u32,
    /// Distinct edge/plane intersections.
    pub section_vertices: u32,
    /// Closed section boundaries, including holes.
    pub section_loops: u32,
    /// Triangles on one cap; zero for section-only queries.
    pub cap_triangles: u32,
    /// Maximum plane distance after narrowing cut vertices to f32.
    pub max_plane_deviation: f64,
}
/// A section of the triangulated input surface; empty when the plane misses it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaneSection {
    /// Maps section-local XY coordinates into body coordinates.
    pub frame: Placement3,
    /// Disconnected filled regions in deterministic boundary order.
    pub regions: Vec<SectionRegion>,
    /// Work and numerical realization evidence, not a self-intersection certificate.
    pub stats: SectionStats,
}
/// Both closed sides of a plane cut, sharing the same realized section vertices.
#[derive(Debug)]
pub struct PlaneSplit {
    /// `dot(normal, point) < distance`, capped toward the positive side.
    /// `None` when this half contains no input geometry.
    pub negative: Option<TessellatedBody>,
    /// `dot(normal, point) > distance`, capped toward the negative side.
    /// `None` when this half contains no input geometry.
    pub positive: Option<TessellatedBody>,
    /// Section geometry and construction evidence.
    pub section: PlaneSection,
}
/// Explicit refusal of a plane operation; no partial result is returned.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SectionError {
    /// Invalid plane, tolerance, or zero work budget.
    InvalidPolicy,
    /// The source map no longer describes the mesh.
    StaleSourceMap,
    /// Input topology is empty, invalid, or open.
    InvalidMesh,
    /// A stored input vertex lies within the contact tolerance of the plane.
    AmbiguousContact,
    /// Robust face triangulation cannot represent an input face.
    Triangulation,
    /// A configured work budget would be exceeded.
    BudgetExceeded,
    /// Section boundaries branch, touch, intersect, or have inconsistent winding.
    InvalidSection,
    /// Geometry cannot retain finite coordinates, orientation, or plane accuracy.
    NumericLimit,
    /// Generated topology could not be closed and validated.
    BuildFailed,
}
impl core::fmt::Display for SectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidPolicy => "invalid plane-section policy or plane",
            Self::StaleSourceMap => "plane-section source map is stale",
            Self::InvalidMesh => "plane section requires a valid closed nonempty mesh",
            Self::AmbiguousContact => "plane contacts an input vertex within tolerance",
            Self::Triangulation => "plane section could not triangulate an input face",
            Self::BudgetExceeded => "plane-section work budget exceeded",
            Self::InvalidSection => "plane section has unsupported boundary topology or winding",
            Self::NumericLimit => "plane section exceeds numeric realization limits",
            Self::BuildFailed => "plane cut could not construct closed valid topology",
        })
    }
}
impl core::error::Error for SectionError {}

/// Extracts oriented section regions from an evaluated body's triangulated mesh.
///
/// # Errors
/// Refuses invalid/open input, contacts within tolerance, unsupported section
/// topology, exhausted budgets, and unrepresentable geometry. The source map must
/// still match the mesh. Distant self-intersection is not checked.
pub fn section_body(
    source: &TessellatedBody,
    plane: Plane3,
    policy: &SectionPolicy,
) -> Result<PlaneSection, SectionError> {
    Ok(prepare(source, plane, policy)?.section)
}

/// Splits an evaluated body into two closed, capped halves.
///
/// Surviving surface triangles retain source features, regions, material slots,
/// corner UVs/normals, and original edge seams/sharpness. Caps use `cap` and
/// section-frame XY UVs; cap rims are sharp. New faces/vertices use
/// [`Feature::PlaneCutCap`]/[`Feature::PlaneCutSeam`]. Derived bodies clear source
/// sampling and realization evidence. No solid-validity certificate is implied.
///
/// # Errors
/// Returns [`SectionError`] for the same failures as [`section_body`], cap
/// triangulation failure, or collapsed/open output. Returns no partial halves.
pub fn split_body(
    source: &TessellatedBody,
    plane: Plane3,
    policy: &SectionPolicy,
    cap: CutCap,
) -> Result<PlaneSplit, SectionError> {
    let mut prepared = prepare(source, plane, policy)?;
    let triangles = cap_triangles(&prepared)?;
    prepared.section.stats.cap_triangles =
        u32::try_from(triangles.len()).map_err(|_| SectionError::BudgetExceeded)?;
    let negative = emit(
        source,
        &prepared,
        &prepared.negative,
        &triangles,
        cap,
        false,
    )?;
    let positive = emit(source, &prepared, &prepared.positive, &triangles, cap, true)?;
    Ok(PlaneSplit {
        negative,
        positive,
        section: prepared.section,
    })
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum PointKey {
    Original(u32),
    Cut(u32, u32),
}
#[derive(Copy, Clone)]
struct Point {
    position: [f64; 3],
    key: PointKey,
    feature: Feature,
    sharpness: Option<f32>,
}
#[derive(Copy, Clone)]
struct Corner {
    point: u32,
    uv: Option<[f64; 2]>,
    normal: Option<[f32; 3]>,
}
struct Polygon {
    corners: Vec<Corner>,
    feature: Feature,
    region: u32,
    material: Option<SlotId>,
}
struct Boundary {
    ids: Vec<u32>,
    features: Vec<Feature>,
    xy: Vec<[f64; 2]>,
    area: f64,
}
struct Prepared {
    points: Vec<Point>,
    negative: Vec<Polygon>,
    positive: Vec<Polygon>,
    boundaries: Vec<Boundary>,
    groups: Vec<(usize, Vec<usize>)>,
    section: PlaneSection,
}

#[cfg(test)]
mod tests;
