// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Plane sections and capped cuts of closed polygonal meshes.
//!
//! Planes use mesh coordinates. Cuts follow robust face triangulation, including
//! the chosen diagonals of nonplanar faces, without analytic reconstruction.
//! Contacts within tolerance are refused. Section frames are deterministic and
//! right-handed, with local +Z along the plane normal. Both original and
//! f32-realized loops must preserve separation, winding and nesting.

mod emit;
#[cfg(test)]
mod far_tests;
mod prepare;
mod triangulate;
use emit::{cap_triangles, emit};
use prepare::prepare;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use exedra_math::{Placement3, Plane3};
use exedra_math::{cross, dot, narrow, normalize, scale, sub};
use exedra_mesh::{FaceBuildAttrs, FaceTriangulation, MeshBuilder};
use exedra_mesh::{FaceId, Mesh, VertexId};

/// Accuracy and finite work limits for plane operations.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SectionPolicy {
    /// Body-space distance: vertices this close to the plane are ambiguous.
    /// Emitted cut vertices must also remain within this distance after f32 storage.
    ///
    /// The tolerance is absolute. Storage places a cut vertex up to half an f32
    /// ulp of its coordinates off the plane, so bodies far from the origin need
    /// a tolerance covering their coordinate magnitude (the default suits
    /// coordinates up to about 16); otherwise the cut is refused as
    /// [`SectionError::NumericLimit`].
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

/// One oriented section boundary in section-frame XY coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionLoop {
    /// Cyclic points, without a duplicate closing point; CCW outer, CW hole.
    pub points: Vec<[f64; 2]>,
    /// Input face for each edge from point `i` to point `(i+1) % len`.
    pub edge_faces: Vec<FaceId>,
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
impl PlaneSection {
    /// Measures all filled regions in this section's local XY frame.
    ///
    /// Holes subtract area and add perimeter; disconnected regions contribute
    /// together. An empty section has zero area/perimeter and no centroid/bounds.
    /// Work is linear in the stored boundary edges without allocation or renewed
    /// tessellation. The section policy bounds generated boundaries.
    ///
    /// Measures the original intersection coordinates stored here, before any
    /// f32 narrowing of a split body's cap. This is not an analytic measurement.
    ///
    /// Because section fields are public, callers editing them must preserve
    /// simple, nonoverlapping regions and correctly nested holes. This query
    /// checks finite arithmetic, degeneracy and winding, not crossings/nesting.
    /// Source faces and the frame remain on this section.
    ///
    /// # Errors
    /// Rejects invalid oriented loops, nonpositive net area for nonempty input,
    /// and unrepresentable arithmetic.
    pub fn measure(
        &self,
    ) -> Result<crate::measure::PlanarMeasurements, crate::measure::MeasurementError> {
        crate::measure::measure(self.regions.iter().flat_map(|region| {
            core::iter::once(crate::measure::PlanarBoundary {
                points: &region.outer.points,
                is_hole: false,
            })
            .chain(
                region
                    .holes
                    .iter()
                    .map(|hole| crate::measure::PlanarBoundary {
                        points: &hole.points,
                        is_hole: true,
                    }),
            )
        }))
    }
}
/// Both closed sides of a plane cut, sharing the same realized section vertices.
#[derive(Debug)]
pub struct PlaneSplit {
    /// `dot(normal, point) < distance`, capped toward the positive side.
    /// `None` when this half contains no input geometry.
    pub negative: Option<CutMesh>,
    /// `dot(normal, point) > distance`, capped toward the negative side.
    /// `None` when this half contains no input geometry.
    pub positive: Option<CutMesh>,
    /// Section geometry and construction evidence.
    pub section: PlaneSection,
}
/// Explicit refusal of a plane operation; no partial result is returned.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SectionError {
    /// Invalid plane, tolerance, or zero work budget.
    InvalidPolicy,
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

/// Extracts oriented section regions from a plain triangulated mesh.
///
/// # Errors
/// Refuses invalid/open input, contacts within tolerance, unsupported section
/// topology, exhausted budgets, and unrepresentable geometry. Distant
/// self-intersection is not checked.
pub fn section_mesh(
    source: &Mesh,
    plane: Plane3,
    policy: &SectionPolicy,
) -> Result<PlaneSection, SectionError> {
    Ok(prepare(source, plane, policy)?.section)
}

/// Splits a plain mesh into two closed, capped halves.
///
/// Surviving surface triangles retain regions, corner UVs/normals and original
/// edge seams/sharpness. Caps use `cap_region` and section-frame XY UVs; cap rims
/// are sharp. Results identify original source faces/vertices and generated cap
/// faces/intersection vertices. Caller-defined layers keep their registration
/// and [`Propagation`](exedra_mesh::attributes::Propagation) rule: surviving
/// faces take their source face's values, corners take the source face's
/// corners weighted by position (exact at source vertices, interpolated at
/// intersections), and intersection vertices interpolate their source edge.
/// Caps have no source face, so their caller values start empty. No
/// solid-validity certificate is implied.
///
/// # Errors
/// Returns [`SectionError`] for the same failures as [`section_mesh`], cap
/// triangulation failure, or collapsed/open output. Returns no partial halves.
pub fn split_mesh(
    source: &Mesh,
    plane: Plane3,
    policy: &SectionPolicy,
    cap_region: u32,
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
        cap_region,
        false,
    )?;
    let positive = emit(
        source,
        &prepared,
        &prepared.positive,
        &triangles,
        cap_region,
        true,
    )?;
    Ok(PlaneSplit {
        negative,
        positive,
        section: prepared.section,
    })
}

/// Input face responsible for a cut face, or a newly generated cap.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CutFaceSource {
    /// Surviving or subdivided input face.
    Original(FaceId),
    /// Newly triangulated plane cap.
    Cap,
}

/// Input geometry responsible for a cut vertex.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CutVertexSource {
    /// An unchanged input vertex.
    Original(VertexId),
    /// Intersection with an input triangle edge. This may be a face's
    /// triangulation diagonal rather than a topological mesh edge.
    Intersection {
        /// Directed interpolation endpoints in the input mesh.
        vertices: [VertexId; 2],
        /// Interpolation fraction measured from the first endpoint.
        parameter: f64,
    },
}

/// One closed half and its source correspondence at the time of emission.
#[derive(Debug)]
pub struct CutMesh {
    /// Generated mesh in the input mesh's coordinate system.
    pub mesh: Mesh,
    /// Source of every generated face, keyed by its output ID.
    pub face_sources: BTreeMap<FaceId, CutFaceSource>,
    /// Source of every generated vertex, keyed by its output ID.
    pub vertex_sources: BTreeMap<VertexId, CutVertexSource>,
    /// Caller-defined attribute values whose layer has no
    /// [`Propagation`](exedra_mesh::attributes::Propagation) rule, so the
    /// split could not carry them.
    pub unpropagated_attribute_values: u64,
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
    provenance: CutVertexSource,
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
    provenance: CutFaceSource,
    region: u32,
}
struct Boundary {
    ids: Vec<u32>,
    faces: Vec<FaceId>,
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
