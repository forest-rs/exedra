// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-independent realization of retained analytic faces.
//!
//! Shared edges are sampled once. Recognized strips, rectangular grids and pole
//! patches keep their specialized bounded paths. Other elementary curved faces
//! require explicit face-local parameter trims: their chart polygons are
//! triangulated and refined by conforming interior-edge splits, preserving every
//! shared boundary chain. Surface Hessian bounds govern that refinement.
//! Planar trims are checked in emitted coordinates. Unsupported correspondence,
//! multi-period charts and invalid emitted arrangements are explicit refusals.
//! This path never heals geometry or certifies global solid validity.

use crate::body::{Body, BodyError, Coedge, EdgeId, Face, FaceId, ValidationPolicy, VertexId};
use crate::geometry::{Curve, GeometryError, Surface};
use alloc::{boxed::Box, vec::Vec};
use exedra_math::{Placement3, narrow};
use exedra_mesh::{BuildParams, Mesh};
use exedra_profile::{CircularEdgeConstraints, CircularSamplingError, circular_edge_count};
use exedra_triangulate::{PolygonInput, TriError, TriParams, triangulate_preserving_boundary};

mod grid;
mod poles;
mod strip;
mod trimmed;
use grid::GridPatch;
use poles::PolePatch;

/// Explicit accuracy and storage bounds for one body realization.
///
/// Planar faces use boundary-preserving constrained Delaunay triangulation.
/// It improves interior diagonals without adding vertices; a minimum triangle
/// angle is not guaranteed. Geometric deviation is controlled by
/// [`Self::tolerance`] independently of triangle shape.
/// Full spheres can use aligned latitude rings with fewer longitudes near the
/// poles when a directed radial-deviation bound certifies the schedule. Other
/// supported pole patches retain their rectangular/fan sampling. Pole positions
/// stay welded; face-varying normal limits and chart coordinates remain distinct.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RealizationPolicy {
    /// Requested geometric deviation in model units, including emitted precision.
    pub tolerance: f64,
    /// Allowed native endpoint/support agreement, separate from tessellation accuracy.
    pub geometry_gap: f64,
    /// Minimum segments for a circle, ellipse or NURBS edge; at least four.
    pub min_arc_segments: u32,
    /// Maximum segments for any circle, ellipse or NURBS edge in each attempt.
    pub max_edge_segments: u32,
    /// Maximum emitted vertices in each attempt.
    pub max_vertices: u32,
    /// Maximum edge-chain entries in each attempt, including repeated endpoints.
    pub max_boundary_samples: u32,
}
impl Default for RealizationPolicy {
    fn default() -> Self {
        Self {
            tolerance: 0.01,
            geometry_gap: 1e-9,
            min_arc_segments: 4,
            max_edge_segments: 4096,
            max_vertices: 1 << 20,
            max_boundary_samples: 1 << 22,
        }
    }
}

/// Native origin of an emitted vertex.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum VertexSource {
    /// An authoritative native vertex.
    Vertex(VertexId),
    /// An interior sample at a fraction of a bounded edge's parameter range.
    Edge {
        /// Sampled native edge.
        edge: EdgeId,
        /// Fraction in `(0,1)`.
        parameter: f64,
    },
    /// An interior sample on a native face, in supporting-surface parameters.
    Face {
        /// Sampled native face.
        face: FaceId,
        /// Exact parameters before placement and f32 emission.
        uv: [f64; 2],
    },
}

/// Native edge correspondence stored in two flat allocations.
#[derive(Clone, Debug)]
pub struct EdgeChains {
    vertices: Vec<exedra_mesh::VertexId>,
    offsets: Vec<usize>,
}
impl EdgeChains {
    /// Emitted vertices from the edge's start to end. Detached edges have empty chains.
    #[must_use]
    pub fn get(&self, edge: EdgeId) -> Option<&[exedra_mesh::VertexId]> {
        let start = *self.offsets.get(edge.index())?;
        let end = *self.offsets.get(edge.index() + 1)?;
        Some(&self.vertices[start..end])
    }
    /// Number of native edge entries, including detached edges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }
    /// Whether there are no native edge entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Work for completed mesh outputs, including a replaced refinement candidate.
///
/// A refused attempt can perform additional validation or planning that these
/// completed-output counters do not include. See the typed refinement outcome.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RealizationWork {
    /// Number of attempts, bounded to one initial attempt and one curve retry.
    pub attempts: u8,
    /// Edge-chain entries in completed outputs, including repeated endpoints.
    pub completed_boundary_samples: usize,
    /// Triangles in completed outputs, including any discarded candidate.
    pub completed_triangles: usize,
    /// Conforming explicit-chart refinement batches in completed outputs.
    pub trim_refinement_batches: usize,
    /// Triangle error tests during explicit-chart refinement, including each
    /// final accepted pass; excludes output orientation checks.
    pub trim_triangle_tests: usize,
    /// Interior vertices generated by explicit-chart refinement.
    pub trim_generated_vertices: usize,
    /// Analytic normal values attached to explicit-chart triangle corners.
    pub trim_normal_corners: usize,
    /// Peak temporary normal-table capacity bytes in a completed attempt,
    /// excluding allocator overhead and retained mesh attributes.
    pub trim_normal_scratch_bytes: usize,
}

/// Result of the optional bounded curve-sampling retry.
#[derive(Clone, Debug, PartialEq)]
pub enum CurveRefinementOutcome {
    /// The retry reduced the combined deviation and replaced the first output.
    /// Final target achievement is still reported by `within_tolerance()`.
    Improved,
    /// The completed retry did not reduce deviation; the first output is retained.
    NotImproved,
    /// The retry was refused; the first output and its honest accuracy remain.
    Refused(RealizationError),
}

/// Evidence for a single optional curve-sampling retry.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveRefinement {
    /// Combined bound for the original completed output.
    pub initial_deviation: f64,
    /// Smaller curve budget used only for this retry, in model units.
    pub curve_tolerance: f64,
    /// Whether the retry replaced the original output, or why it did not.
    pub outcome: CurveRefinementOutcome,
}

/// Accuracy and work evidence for one explicit conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct RealizationReport {
    /// Requested deviation in model units.
    pub tolerance: f64,
    /// Largest conservative native endpoint/support gap from validation.
    pub geometry_gap: f64,
    /// Largest conservative curve chord bound from actual sampled parameter
    /// intervals, before placement and narrowing.
    pub max_curve_deviation: f64,
    /// Largest conservative supporting-surface interpolation bound, including
    /// explicit-chart Hessian bounds or specialized grid alignment arithmetic,
    /// before placement and narrowing.
    pub max_surface_deviation: f64,
    /// Largest emitted vertex displacement, including placement and narrowing.
    pub max_vertex_displacement: f64,
    /// Largest emitted paired-sample residual from the ideal rotational ruling.
    /// Cylinder rulings translate; conical rulings also scale the reference radius.
    /// Includes parameter-grid mismatch and authoritative endpoint offsets.
    pub max_ruling_displacement: f64,
    /// Conservative geometric deviation bound: maximum curve/surface error, emitted and ruling
    /// displacement, and two native gaps (endpoint-to-curve and curve-to-support).
    pub max_deviation: f64,
    /// Physical curve-segment counts indexed by native edge; zero for detached
    /// edges and explicit collapsed boundaries, whose chain has one vertex.
    pub edge_segments: Vec<u32>,
    /// Attempt and completed-output work, including any discarded output.
    pub work: RealizationWork,
    /// A single curve retry, absent on the ordinary first-pass path.
    pub curve_refinement: Option<CurveRefinement>,
}
impl RealizationReport {
    /// Whether the emitted result meets the requested geometric deviation.
    #[must_use]
    pub fn within_tolerance(&self) -> bool {
        self.max_deviation <= self.tolerance
    }
}

/// Forward face parameters at an emitted corner whose authoritative native
/// vertex is a surface pole. These are supporting-surface coordinates, not
/// texture UVs or fictitious parameters of a collapsed edge.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PoleCorner {
    /// Mesh-local corner in this realization.
    pub corner: exedra_mesh::CornerId,
    /// Owning native face.
    pub face: FaceId,
    /// Authoritative native pole vertex; also retained in [`VertexSource::Vertex`].
    pub vertex: VertexId,
    /// Face-local U/V. Cone U selects a one-sided limiting normal; sphere normals
    /// are unique at their coordinate poles. Parameter units follow the surface.
    pub uv: [f64; 2],
}

/// Sparse immutable pole-corner correspondence, sorted by mesh corner index.
/// The empty default allocates nothing; mesh IDs remain realization-local.
#[derive(Clone, Debug, Default)]
pub struct PoleCorners {
    records: Box<[PoleCorner]>,
}
impl PoleCorners {
    /// Borrow all records without cloning geometry or correspondence.
    #[must_use]
    pub fn as_slice(&self) -> &[PoleCorner] {
        &self.records
    }
    /// Number of pole-corner records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }
    /// Whether this realization has no pole-corner records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    /// A mesh-local corner's forward pole parameters. Unknown/stale local IDs
    /// return `None`; IDs from a different realization must not be reused.
    #[must_use]
    pub fn get(&self, corner: exedra_mesh::CornerId) -> Option<&PoleCorner> {
        let i = self
            .records
            .binary_search_by_key(&corner.index(), |r| r.corner.index())
            .ok()?;
        (self.records[i].corner == corner).then_some(&self.records[i])
    }
}

/// A realized mesh with native geometry correspondence and accuracy evidence.
#[derive(Clone, Debug)]
pub struct RealizedBody {
    /// Placed f32 triangle mesh.
    pub mesh: Mesh,
    /// Native source of each mesh face, in mesh-face order.
    pub mesh_faces: Vec<(exedra_mesh::FaceId, FaceId)>,
    /// Native source of each emitted vertex, in vertex order.
    pub mesh_vertices: Vec<(exedra_mesh::VertexId, VertexSource)>,
    /// Shared sampled boundary chains in edge parameter order.
    pub edge_chains: EdgeChains,
    /// Forward face parameters for corners at coordinate singularities.
    pub pole_corners: PoleCorners,
    /// Geometric error and work evidence.
    pub report: RealizationReport,
}

/// A bounded realization refusal; no input geometry is changed.
#[derive(Clone, Debug, PartialEq)]
pub enum RealizationError {
    /// A policy value or budget is invalid.
    InvalidPolicy,
    /// The placement is not a finite rigid frame.
    Frame(GeometryError),
    /// Native topology or support agreement is invalid.
    Boundary(BodyError),
    /// No faces were supplied.
    EmptyBoundary,
    /// A curved trim lacks a recognized bounded patch or complete supported
    /// explicit parameter correspondence, or its chart spans multiple periods.
    UnsupportedSurfaceTrim {
        /// Offending native face.
        face: FaceId,
    },
    /// Circular sampling exceeds the requested edge budget.
    Sampling {
        /// Offending native edge.
        edge: EdgeId,
        /// Sampling diagnosis.
        error: CircularSamplingError,
    },
    /// A doubly curved patch needs more intervals than the edge budget permits.
    SurfaceSampling {
        /// Offending native face.
        face: FaceId,
        /// Current U/V interval counts; these still exceed the error budget.
        intervals: [u32; 2],
        /// Maximum intervals in either direction.
        maximum: u32,
    },
    /// An explicit storage budget is exceeded.
    Budget {
        /// Budgeted storage unit.
        storage: &'static str,
        /// Required entries, or a lower bound when counting stopped early.
        required: usize,
        /// Allowed entries.
        maximum: u32,
    },
    /// A counted output table cannot fit the platform's container address space.
    AddressSpace {
        /// Output storage unit that cannot be addressed.
        storage: &'static str,
    },
    /// More than two face occurrences use an edge.
    NonManifold {
        /// Offending native edge.
        edge: EdgeId,
        /// Occurrence count.
        uses: usize,
    },
    /// Distinct native edges would become the same mesh edge.
    DuplicateEdge {
        /// First native edge.
        first: EdgeId,
        /// Conflicting native edge.
        second: EdgeId,
    },
    /// A sampled edge collapses or overflows after placement and narrowing.
    CollapsedEdge {
        /// Offending native edge.
        edge: EdgeId,
    },
    /// A placed native vertex is non-finite in f32.
    NonFiniteVertex {
        /// Offending native vertex.
        vertex: VertexId,
    },
    /// A generated surface sample is non-finite after placement and f32 storage.
    /// This is a numeric representation refusal, before triangle construction.
    NonFiniteFaceSample {
        /// Supporting native face.
        face: FaceId,
        /// Forward supporting-surface parameters of the sample.
        uv: [f64; 2],
    },
    /// Planar boundary triangulation was refused in emitted coordinates.
    Arrangement {
        /// Offending native face.
        face: FaceId,
        /// Planar diagnosis.
        error: TriError,
    },
    /// A generated triangle collapses or opposes the declared face orientation.
    DegenerateFace {
        /// Offending native face.
        face: FaceId,
    },
    /// The mesh builder refused the output.
    Build(exedra_mesh::BuildError),
}
impl core::fmt::Display for RealizationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "analytic realization: {self:?}")
    }
}
impl core::error::Error for RealizationError {}

#[derive(Copy, Clone, Default)]
struct Incidence {
    faces: [Option<FaceId>; 2],
    count: usize,
}
#[derive(Copy, Clone)]
struct Strip {
    a: Coedge,
    b: Coedge,
    translation: [f64; 3],
    variable_axial_rulings: bool,
    center: [f64; 3],
    radius_scale: f64,
    correspondence_gap: f64,
    sides: Option<strip::Sides>,
}
#[derive(Copy, Clone)]
enum Patch {
    Planar,
    Trimmed,
    Strip(Strip),
    Grid(GridPatch),
    Pole(PolePatch),
}

fn check_table_capacity<T>(storage: &'static str, count: usize) -> Result<(), RealizationError> {
    if (isize::MAX as usize)
        .checked_div(size_of::<T>())
        .is_some_and(|maximum| count > maximum)
    {
        return Err(RealizationError::AddressSpace { storage });
    }
    Ok(())
}

struct Chains {
    vertices: Vec<u32>,
    offsets: Vec<usize>,
}
impl Chains {
    fn get(&self, edge: EdgeId) -> &[u32] {
        &self.vertices[self.offsets[edge.index()]..self.offsets[edge.index() + 1]]
    }
}

// Union by rank and path compression keep paired strip counts coherent in one pass.
struct Groups {
    parent: Vec<usize>,
    rank: Vec<u8>,
}
impl Groups {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: alloc::vec![0;n],
        }
    }
    fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }
    fn join(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.rank[a] < self.rank[b] {
            core::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        if self.rank[a] == self.rank[b] {
            self.rank[a] += 1;
        }
    }
    fn synchronize(&mut self, counts: &mut [u32]) {
        let mut maximum = alloc::vec![0;counts.len()];
        for (i, &n) in counts.iter().enumerate() {
            let root = self.root(i);
            maximum[root] = maximum[root].max(n);
        }
        for (i, n) in counts.iter_mut().enumerate() {
            *n = maximum[self.root(i)];
        }
    }
}

impl<F, E> Body<F, E> {
    /// Realizes supported faces under an explicit rigid placement and policy.
    /// Regions are native face indices; source metadata is never interpreted.
    ///
    /// # Errors
    /// Returns [`RealizationError`] for unsupported trims, invalid native data,
    /// exhausted budgets or an invalid arrangement after f32 narrowing.
    pub fn realize(
        &self,
        placement: Placement3,
        policy: &RealizationPolicy,
    ) -> Result<RealizedBody, RealizationError> {
        self.realize_with_regions(placement, policy, |id, _| id.0)
    }

    /// Realizes the body and maps each native face to a caller region.
    ///
    /// Native correspondence remains per-face even when regions are shared.
    /// The region function is evaluated once per native face. If a completed
    /// output misses the target because curve sampling consumed its headroom,
    /// with other error below one quarter of the target, one retry halves only
    /// the curve budget. Placement, geometry policy, grid policy and requested
    /// target remain unchanged. The original output is kept on retry refusal
    /// or non-improvement. Ordinary successful outputs take the original path.
    ///
    /// Storage limits apply to each attempt. During a retry both bounded outputs
    /// can be live; peak storage can therefore approach twice one output's limits.
    ///
    /// # Errors
    /// Returns the same bounded refusals as [`Self::realize`].
    pub fn realize_with_regions(
        &self,
        placement: Placement3,
        policy: &RealizationPolicy,
        region: impl Fn(FaceId, &Face<F>) -> u32,
    ) -> Result<RealizedBody, RealizationError> {
        let first = self.realize_once(placement, policy, policy.tolerance, region)?;
        if first.report.within_tolerance() {
            Ok(first)
        } else {
            Ok(self.refine_curves(first, placement, policy))
        }
    }

    #[cold]
    fn refine_curves(
        &self,
        mut first: RealizedBody,
        placement: Placement3,
        policy: &RealizationPolicy,
    ) -> RealizedBody {
        let report = &first.report;
        let other = report.max_vertex_displacement
            + report.max_ruling_displacement
            + 2.0 * report.geometry_gap;
        let curve_tolerance = 0.5 * policy.tolerance;
        if report.max_curve_deviation <= 0.0
            || report.max_surface_deviation > report.max_curve_deviation
            || other >= 0.25 * policy.tolerance
            || curve_tolerance <= 0.0
        {
            return first;
        }
        // Recover semantic values from the first output only on this rare path.
        // Reusing native correspondence avoids invoking caller code a second time.
        let mut face_regions = alloc::vec![None; self.faces().len()];
        let values = first.mesh.attrs().dense(exedra_mesh::attr::FACE_REGION);
        for &(mesh_face, native_face) in &first.mesh_faces {
            face_regions[native_face.index()] = Some(
                values
                    .and_then(|layer| layer.get(mesh_face.as_id()))
                    .copied()
                    .unwrap_or(0),
            );
        }
        if face_regions.iter().any(Option::is_none) {
            return first;
        }
        let initial_deviation = first.report.max_deviation;
        let mut work = first.report.work;
        work.attempts = 2;
        let retry = self.realize_once(placement, policy, curve_tolerance, |id, _| {
            face_regions[id.index()].expect("complete first-output region mapping")
        });
        let outcome = match retry {
            Ok(mut candidate) => {
                work.completed_boundary_samples += candidate.report.work.completed_boundary_samples;
                work.completed_triangles += candidate.report.work.completed_triangles;
                work.trim_refinement_batches += candidate.report.work.trim_refinement_batches;
                work.trim_triangle_tests += candidate.report.work.trim_triangle_tests;
                work.trim_generated_vertices += candidate.report.work.trim_generated_vertices;
                work.trim_normal_corners += candidate.report.work.trim_normal_corners;
                work.trim_normal_scratch_bytes = work
                    .trim_normal_scratch_bytes
                    .max(candidate.report.work.trim_normal_scratch_bytes);
                if candidate.report.max_deviation < first.report.max_deviation {
                    candidate.report.work = work;
                    candidate.report.curve_refinement = Some(CurveRefinement {
                        initial_deviation,
                        curve_tolerance,
                        outcome: CurveRefinementOutcome::Improved,
                    });
                    return candidate;
                }
                CurveRefinementOutcome::NotImproved
            }
            Err(error) => CurveRefinementOutcome::Refused(error),
        };
        first.report.work = work;
        first.report.curve_refinement = Some(CurveRefinement {
            initial_deviation,
            curve_tolerance,
            outcome,
        });
        first
    }

    fn realize_once(
        &self,
        placement: Placement3,
        policy: &RealizationPolicy,
        curve_tolerance: f64,
        region: impl Fn(FaceId, &Face<F>) -> u32,
    ) -> Result<RealizedBody, RealizationError> {
        if !policy.tolerance.is_finite()
            || policy.tolerance <= 0.0
            || policy.min_arc_segments < 4
            || policy.min_arc_segments > policy.max_edge_segments
            || policy.max_vertices == 0
            || policy.max_boundary_samples == 0
        {
            return Err(RealizationError::InvalidPolicy);
        }
        crate::geometry::rigid(placement).map_err(RealizationError::Frame)?;
        let validation = self
            .validate(&ValidationPolicy {
                max_gap: policy.geometry_gap,
            })
            .map_err(RealizationError::Boundary)?;
        if self.faces().is_empty() {
            return Err(RealizationError::EmptyBoundary);
        }
        let mut incidence = alloc::vec![Incidence::default();self.edges().len()];
        let mut patches = Vec::with_capacity(self.faces().len());
        let mut groups = Groups::new(self.edges().len());
        let mut pole_output = false;
        for (i, face) in self.faces().iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            let mut has_pole = false;
            for &loop_id in &face.loops {
                for coedge in &self.boundary_loop(loop_id).expect("validated loop").coedges {
                    has_pole |= self.edges()[coedge.edge.index()].is_collapsed();
                    let item = &mut incidence[coedge.edge.index()];
                    if item.count == 2 {
                        return Err(RealizationError::NonManifold {
                            edge: coedge.edge,
                            uses: 3,
                        });
                    }
                    item.faces[item.count] = Some(id);
                    item.count += 1;
                }
            }
            let explicit = self.has_explicit_trims(id);
            let patch = if has_pole {
                pole_output = true;
                Patch::Pole(PolePatch::prepare(self, id, policy)?)
            } else if matches!(face.surface, Surface::Sphere(_)) {
                if explicit {
                    Patch::Trimmed
                } else {
                    return Err(RealizationError::UnsupportedSurfaceTrim { face: id });
                }
            } else if matches!(face.surface, Surface::Torus(_) | Surface::Ruled(_)) {
                match GridPatch::prepare(self, id) {
                    Ok(patch) => Patch::Grid(patch),
                    Err(_) if explicit => Patch::Trimmed,
                    Err(error) => return Err(error),
                }
            } else if matches!(face.surface, Surface::Plane(_)) {
                // Coaxial annuli/sectors retain angular rulings, which are
                // also the construction chart's nondegenerate coordinate net.
                self.rotational_patch(id, policy.geometry_gap)
                    .map_or(Patch::Planar, Patch::Strip)
            } else {
                match self.rotational_patch(id, policy.geometry_gap) {
                    Ok(patch) => Patch::Strip(patch),
                    Err(_) if explicit => Patch::Trimmed,
                    Err(error) => return Err(error),
                }
            };
            match patch {
                Patch::Strip(p) => groups.join(p.a.edge.index(), p.b.edge.index()),
                Patch::Grid(p) => p.join(&mut groups),
                Patch::Trimmed => {
                    if policy.tolerance * 0.25 == 0.0 {
                        return Err(RealizationError::InvalidPolicy);
                    }
                }
                Patch::Planar | Patch::Pole(_) => {}
            }
            patches.push(patch);
        }
        let mut counts = Vec::with_capacity(self.edges().len());
        for (i, edge) in self.edges().iter().enumerate() {
            let n = if incidence[i].count == 0 {
                0
            } else {
                if let Some((curve, range)) = edge.regular() {
                    match curve {
                        Curve::Line(_) => 1,
                        Curve::Nurbs(nurbs) => {
                            let edge = EdgeId(crate::usize_to_u32(i));
                            let required = nurbs
                                .edge_count(range, curve_tolerance)
                                .ok_or(RealizationError::Sampling {
                                    edge,
                                    error: CircularSamplingError::EdgeCountOverflow,
                                })?
                                .max(policy.min_arc_segments);
                            if required > policy.max_edge_segments {
                                return Err(RealizationError::Sampling {
                                    edge,
                                    error: CircularSamplingError::ToleranceBudgetExceeded {
                                        required,
                                        maximum: policy.max_edge_segments,
                                    },
                                });
                            }
                            required
                        }
                        curve => circular_edge_count(
                            curve.sampling_radius().expect("periodic curve"),
                            range.span(),
                            curve_tolerance,
                            CircularEdgeConstraints::new(
                                policy.min_arc_segments,
                                policy.max_edge_segments,
                            ),
                        )
                        .map_err(|error| RealizationError::Sampling {
                            edge: EdgeId(crate::usize_to_u32(i)),
                            error,
                        })?,
                    }
                } else {
                    0
                }
            };
            counts.push(n);
        }
        trimmed::prepare_counts(self, &patches, &mut counts, policy)?;
        for patch in &patches {
            if let Patch::Pole(patch) = patch {
                patch.synchronize(&mut counts);
            }
        }
        groups.synchronize(&mut counts);
        grid::refine(self, &patches, &mut counts, &mut groups, policy)?;
        for (patch, face) in patches.iter_mut().zip(self.faces()) {
            if let Patch::Pole(patch) = patch {
                patch.prepare_radial_sampling(face, &counts, policy)?;
            }
        }
        let mut active_vertices = alloc::vec![false;self.vertices().len()];
        let mut samples = 0_usize;
        let mut interior = 0_usize;
        for (i, (edge, &n)) in self.edges().iter().zip(&counts).enumerate() {
            if incidence[i].count == 0 {
                continue;
            }
            if edge.is_collapsed() {
                active_vertices[edge.start.index()] = true;
                samples = samples.checked_add(1).ok_or(RealizationError::Budget {
                    storage: "boundary samples",
                    required: usize::MAX,
                    maximum: policy.max_boundary_samples,
                })?;
                continue;
            }
            active_vertices[edge.start.index()] = true;
            active_vertices[edge.end.index()] = true;
            samples = samples
                .checked_add(n as usize)
                .and_then(|total| total.checked_add(1))
                .ok_or(RealizationError::Budget {
                    storage: "boundary samples",
                    required: usize::MAX,
                    maximum: policy.max_boundary_samples,
                })?;
            interior = interior
                .checked_add(n as usize - 1)
                .ok_or(RealizationError::Budget {
                    storage: "vertices",
                    required: usize::MAX,
                    maximum: policy.max_vertices,
                })?;
        }
        let vertex_count = active_vertices
            .iter()
            .filter(|v| **v)
            .count()
            .saturating_add(interior)
            .saturating_add(
                patches
                    .iter()
                    .filter_map(|p| match p {
                        Patch::Grid(p) => Some(p.interior_count(&counts)),
                        Patch::Pole(p) => Some(p.interior_count(&counts)),
                        _ => None,
                    })
                    .fold(0_usize, usize::saturating_add),
            );
        for (storage, required, maximum) in [
            ("boundary samples", samples, policy.max_boundary_samples),
            ("vertices", vertex_count, policy.max_vertices),
        ] {
            if required > maximum as usize {
                return Err(RealizationError::Budget {
                    storage,
                    required,
                    maximum,
                });
            }
        }
        // Counts and trims already determine output cardinality. Reserve the
        // three parallel arrays once instead of repeatedly growing each while
        // emitting strips. A planar polygon contributes N + 2H - 2 triangles.
        let mut pole_corner_count = 0_usize;
        let triangle_count = self
            .faces()
            .iter()
            .enumerate()
            .map(|(i, face)| match patches[i] {
                Patch::Planar | Patch::Trimmed => face
                    .loops
                    .iter()
                    .flat_map(|id| &self.boundary_loop(*id).expect("validated loop").coedges)
                    .map(|coedge| counts[coedge.edge.index()] as usize)
                    .fold(0_usize, usize::saturating_add)
                    .saturating_add(face.loops.len().saturating_sub(1).saturating_mul(2))
                    .saturating_sub(2),
                Patch::Strip(patch) => (counts[patch.a.edge.index()] as usize)
                    .saturating_mul(2)
                    .saturating_add(patch.sides.map_or(0, |s| s.extra_samples(self, &counts))),
                Patch::Grid(patch) => patch.triangle_count(&counts),
                Patch::Pole(patch) => {
                    pole_corner_count =
                        pole_corner_count.saturating_add(patch.pole_corner_count(&counts));
                    patch.triangle_count(&counts)
                }
            })
            .try_fold(0_usize, |total, count| {
                let required = total
                    .checked_add(count)
                    .ok_or(RealizationError::AddressSpace {
                        storage: "triangles",
                    })?;
                if required > u32::MAX as usize {
                    Err(RealizationError::Budget {
                        storage: "triangles",
                        required,
                        maximum: u32::MAX,
                    })
                } else {
                    Ok(required)
                }
            })?;
        // Saturated counts must not equal a caller's u32::MAX budget on a
        // 32-bit target and then reach Vec::with_capacity. Check every primary
        // output table even when a pole planner fell back to a regular grid.
        check_table_capacity::<[f32; 3]>("vertices", vertex_count)?;
        check_table_capacity::<VertexSource>("vertex sources", vertex_count)?;
        check_table_capacity::<(exedra_mesh::VertexId, VertexSource)>(
            "vertex correspondence",
            vertex_count,
        )?;
        check_table_capacity::<[u32; 3]>("triangles", triangle_count)?;
        check_table_capacity::<(exedra_mesh::FaceId, FaceId)>(
            "face correspondence",
            triangle_count,
        )?;
        check_table_capacity::<u32>("boundary samples", samples)?;
        check_table_capacity::<poles::PoleSeed>("pole corner preparation", pole_corner_count)?;
        check_table_capacity::<PoleCorner>("pole corner correspondence", pole_corner_count)?;
        let mut emitted = Vec::with_capacity(vertex_count);
        let mut sources = Vec::with_capacity(vertex_count);
        let mut vertex_map = alloc::vec![None;self.vertices().len()];
        let mut displacement = 0.0_f64;
        for (i, (&point, &active)) in self.vertices().iter().zip(&active_vertices).enumerate() {
            if !active {
                continue;
            }
            let exact = placement.transform_point(point);
            let p = narrow(exact);
            if !p.iter().all(|v| v.is_finite()) {
                return Err(RealizationError::NonFiniteVertex {
                    vertex: VertexId(crate::usize_to_u32(i)),
                });
            }
            displacement = displacement.max(
                distance(p.map(f64::from), exact)
                    + if pole_output {
                        placement_roundoff(placement, point)
                    } else {
                        0.0
                    },
            );
            vertex_map[i] = Some(crate::usize_to_u32(emitted.len()));
            emitted.push(p);
            sources.push(VertexSource::Vertex(VertexId(crate::usize_to_u32(i))));
        }
        let mut chains = Chains {
            vertices: Vec::with_capacity(samples),
            offsets: Vec::with_capacity(self.edges().len() + 1),
        };
        let mut curve_deviation = 0.0_f64;
        // Consecutive circles commonly share the same angular schedule. Keep
        // one reusable phase grid, bounded by the checked boundary-sample
        // budget, rather than evaluating sin/cos again for every profile ring.
        let mut phase_key = None;
        let mut phases = Vec::new();
        for (i, (edge, &n)) in self.edges().iter().zip(&counts).enumerate() {
            chains.offsets.push(chains.vertices.len());
            if edge.is_collapsed() && incidence[i].count > 0 {
                chains
                    .vertices
                    .push(vertex_map[edge.start.index()].expect("active pole vertex"));
                continue;
            }
            if n == 0 {
                continue;
            }
            let id = EdgeId(crate::usize_to_u32(i));
            let (curve, range) = edge.regular().expect("sampled edge is regular");
            chains
                .vertices
                .push(vertex_map[edge.start.index()].expect("active vertex"));
            let [min, max] = range.bounds();
            let key = (min.to_bits(), max.to_bits(), n);
            let reuse_phases = phase_key == Some(key);
            if curve.harmonic().is_some() && !reuse_phases {
                phases.clear();
                phases.reserve(n as usize - 1);
                phase_key = Some(key);
            }
            let mut previous_parameter = min;
            let mut max_parameter_step = 0.0_f64;
            for k in 1..n {
                let t = f64::from(k) / f64::from(n);
                let parameter = range.at(t).expect("bounded sample fraction");
                max_parameter_step = max_parameter_step.max((parameter - previous_parameter).abs());
                previous_parameter = parameter;
                let point = match &curve {
                    curve if curve.harmonic().is_some() => {
                        let [sin, cos] = if reuse_phases {
                            phases[k as usize - 1]
                        } else {
                            let phase = [libm::sin(parameter), libm::cos(parameter)];
                            phases.push(phase);
                            phase
                        };
                        let (frame, [a, b]) = curve.harmonic().expect("harmonic curve");
                        frame.transform_point([a * cos, b * sin, 0.0])
                    }
                    _ => {
                        curve
                            .evaluate(parameter)
                            .ok_or(RealizationError::CollapsedEdge { edge: id })?
                            .point
                    }
                };
                let exact = placement.transform_point(point);
                let p = narrow(exact);
                if !p.iter().all(|v| v.is_finite()) {
                    return Err(RealizationError::CollapsedEdge { edge: id });
                }
                displacement = displacement.max(
                    distance(p.map(f64::from), exact)
                        + if pole_output {
                            placement_roundoff(placement, point)
                        } else {
                            0.0
                        },
                );
                chains.vertices.push(crate::usize_to_u32(emitted.len()));
                emitted.push(p);
                sources.push(VertexSource::Edge {
                    edge: id,
                    parameter: t,
                });
            }
            chains
                .vertices
                .push(vertex_map[edge.end.index()].expect("active vertex"));
            if chains
                .get_partial(i)
                .windows(2)
                .any(|p| emitted[p[0] as usize] == emitted[p[1] as usize])
            {
                return Err(RealizationError::CollapsedEdge { edge: id });
            }
            if let Curve::Nurbs(nurbs) = curve {
                max_parameter_step = max_parameter_step.max((max - previous_parameter).abs());
                curve_deviation = curve_deviation.max(nurbs.chord_bound(range, max_parameter_step));
            }
            if let Some(radius) = curve.sampling_radius() {
                // Absolute parameter origins can make nominally equal steps
                // round differently. Bound the intervals actually evaluated.
                max_parameter_step = max_parameter_step.max((max - previous_parameter).abs());
                let sin = libm::sin(max_parameter_step / 4.0);
                let deviation = 2.0 * radius * sin * sin;
                let deviation = if pole_output {
                    (deviation * (1.0 + 4.0 * crate::geometry::FRAME_TOLERANCE).next_up()).next_up()
                } else {
                    deviation
                };
                curve_deviation = curve_deviation.max(deviation);
            }
        }
        chains.offsets.push(chains.vertices.len());
        let mut sharp = Vec::with_capacity(
            samples
                - self
                    .edges()
                    .iter()
                    .zip(&counts)
                    .filter(|(_, n)| **n > 0)
                    .count(),
        );
        for (i, &n) in counts.iter().enumerate() {
            if n == 0 {
                continue;
            }
            let id = EdgeId(crate::usize_to_u32(i));
            let value = self.edge_sharpness(id, incidence[i]);
            for pair in chains.get(id).windows(2) {
                let (a, b) = ordered(pair[0], pair[1]);
                sharp.push((a, b, value, id));
            }
        }
        sharp.sort_unstable_by_key(|&(a, b, _, _)| (a, b));
        for pair in sharp.windows(2) {
            if (pair[0].0, pair[0].1) == (pair[1].0, pair[1].1) && pair[0].3 != pair[1].3 {
                return Err(RealizationError::DuplicateEdge {
                    first: pair[0].3,
                    second: pair[1].3,
                });
            }
        }
        // The sorted native boundary table is sparse in emitted vertex pairs.
        // Index its rows once so each mesh edge searches only the boundary
        // neighbors of its lower endpoint, rather than the entire table.
        let mut sharp_offsets = Vec::with_capacity(vertex_count + 1);
        let mut next = 0;
        for vertex in 0..vertex_count {
            sharp_offsets.push(next);
            while next < sharp.len() && sharp[next].0 as usize == vertex {
                next += 1;
            }
        }
        sharp_offsets.push(next);
        let mut triangles = Vec::with_capacity(triangle_count);
        let mut regions = Vec::with_capacity(triangle_count);
        let mut native_faces = Vec::with_capacity(triangle_count);
        let mut ruling_displacement = 0.0_f64;
        let mut surface_deviation = 0.0_f64;
        let mut trim_work = trimmed::Work::default();
        let mut trim_normals = Vec::new();
        let mut grid_scratch = grid::GridScratch::default();
        let mut pole_scratch = poles::Scratch::default();
        let mut pole_corners = Vec::with_capacity(pole_corner_count);
        for (i, face) in self.faces().iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            let face_region = region(id, face);
            if let Patch::Pole(patch) = patches[i] {
                surface_deviation = surface_deviation.max(patch.deviation(face, &counts));
                patch.emit(
                    self,
                    id,
                    placement,
                    &counts,
                    &chains,
                    &mut pole_scratch,
                    &mut emitted,
                    &mut sources,
                    &mut displacement,
                    face_region,
                    &mut triangles,
                    &mut regions,
                    &mut native_faces,
                    &mut pole_corners,
                )?;
            } else if let Patch::Grid(patch) = patches[i] {
                surface_deviation = surface_deviation.max(patch.deviation(face, &counts));
                patch.emit(
                    self,
                    id,
                    placement,
                    &counts,
                    &chains,
                    &mut grid_scratch,
                    &mut emitted,
                    &mut sources,
                    &mut displacement,
                    face_region,
                    &mut triangles,
                    &mut regions,
                    &mut native_faces,
                )?;
            } else if matches!(patches[i], Patch::Trimmed) {
                surface_deviation = surface_deviation.max(trimmed::emit_face(
                    self,
                    id,
                    placement,
                    policy,
                    &chains,
                    &mut emitted,
                    &mut sources,
                    &mut displacement,
                    face_region,
                    &mut triangles,
                    &mut regions,
                    &mut native_faces,
                    &mut trim_normals,
                    &mut trim_work,
                )?);
            } else if matches!(patches[i], Patch::Planar) {
                let frame = placement * self.planar_frame(id, [0.0; 2]).expect("planar support");
                let mut rings = Vec::with_capacity(face.loops.len());
                let mut indices = Vec::new();
                for &loop_id in &face.loops {
                    let mut points = Vec::new();
                    for &coedge in &self.boundary_loop(loop_id).expect("validated loop").coedges {
                        let chain = chains.get(coedge.edge);
                        for index in 0..chain.len() - 1 {
                            let index = chain[if coedge.reversed {
                                chain.len() - 1 - index
                            } else {
                                index
                            }];
                            points.push(project(frame, emitted[index as usize].map(f64::from)));
                            indices.push(index);
                        }
                    }
                    rings.push(points);
                }
                let holes: Vec<&[[f64; 2]]> = rings.iter().skip(1).map(Vec::as_slice).collect();
                let input = PolygonInput {
                    outer: &rings[0],
                    holes: &holes,
                };
                let triangulated =
                    triangulate_preserving_boundary(&input, &TriParams::constrained_delaunay())
                        .map_err(|error| RealizationError::Arrangement { face: id, error })?;
                for triangle in triangulated.triangles {
                    emit(
                        &mut triangles,
                        triangle.map(|j| indices[j as usize]),
                        id,
                        face_region,
                        &emitted,
                        &mut regions,
                        &mut native_faces,
                    )?;
                }
            } else {
                let Patch::Strip(patch) = patches[i] else {
                    unreachable!()
                };
                let (a, b) = (chains.get(patch.a.edge), chains.get(patch.b.edge));
                let a_at = |j: usize| a[if patch.a.reversed { a.len() - 1 - j } else { j }];
                let b_at = |j: usize| b[if patch.b.reversed { j } else { b.len() - 1 - j }];
                // Replace each upper sample by its paired lower sample plus
                // the ideal axial/radial map. Barycentric interpolation moves
                // every triangle point by at most the largest such residual.
                // Use the emitted chains themselves, including authoritative
                // endpoints. The reference chord's sagitta and narrowing error
                // are already bounded; the upper map is contractive because
                // radius_scale <= 1. This residual also includes upper narrowing.
                let center = placement.transform_point(patch.center);
                let translation = crate::geometry::vector(placement, patch.translation);
                if !center.iter().chain(&translation).all(|v| v.is_finite()) {
                    return Err(RealizationError::DegenerateFace { face: id });
                }
                let mut patch_ruling = 0.0_f64;
                for j in 0..a.len() {
                    let lower = emitted[a_at(j) as usize].map(f64::from);
                    let upper = emitted[b_at(j) as usize].map(f64::from);
                    let radial_change =
                        exedra_math::sub(lower, center).map(|v| v * (patch.radius_scale - 1.0));
                    let expected_delta = if patch.variable_axial_rulings {
                        // Complete radial coefficient agreement was established
                        // by cylindrical_band. Unequal axial rim harmonics keep
                        // the same exact cylinder generatrices; they do not need
                        // interior chart refinement along an otherwise straight
                        // ruling. Compare emitted samples to both exact rims.
                        let fraction = j as f64 / (a.len() - 1) as f64;
                        let af = if patch.a.reversed {
                            1.0 - fraction
                        } else {
                            fraction
                        };
                        let bf = if patch.b.reversed {
                            fraction
                        } else {
                            1.0 - fraction
                        };
                        exedra_math::sub(
                            placement.transform_point(
                                self.edge_point(patch.b.edge, bf)
                                    .ok_or(RealizationError::DegenerateFace { face: id })?,
                            ),
                            placement.transform_point(
                                self.edge_point(patch.a.edge, af)
                                    .ok_or(RealizationError::DegenerateFace { face: id })?,
                            ),
                        )
                    } else {
                        exedra_math::add(translation, radial_change)
                    };
                    let residual = distance(exedra_math::sub(upper, lower), expected_delta);
                    if !residual.is_finite() {
                        return Err(RealizationError::DegenerateFace { face: id });
                    }
                    patch_ruling = patch_ruling.max(patch.correspondence_gap + residual);
                }
                let sides = patch
                    .sides
                    .map(|s| {
                        s.samples(
                            self,
                            id,
                            &chains,
                            &sources,
                            &emitted,
                            placement,
                            [a_at(0), b_at(0)],
                            [a_at(a.len() - 1), b_at(a.len() - 1)],
                            &mut patch_ruling,
                        )
                    })
                    .transpose()?;
                ruling_displacement = ruling_displacement.max(patch_ruling);
                // Check orientation once, at the first strip's midpoint. The
                // midpoint of a full periodic edge faces the opposite direction.
                let step = 1.0 / f64::from(counts[patch.a.edge.index()]);
                let start = if patch.a.reversed { 1.0 } else { 0.0 };
                let next = start + if patch.a.reversed { -step } else { step };
                let point = |edge, t| {
                    self.edge_point(edge, t)
                        .ok_or(RealizationError::DegenerateFace { face: id })
                };
                let uv = face
                    .surface
                    .parameters(point(patch.a.edge, (start + next) * 0.5)?)
                    .ok_or(RealizationError::UnsupportedSurfaceTrim { face: id })?;
                let normal = self
                    .evaluate_support(id, uv)
                    .ok_or(RealizationError::UnsupportedSurfaceTrim { face: id })?
                    .normal;
                let native_a = point(patch.a.edge, start)?;
                let native_next = point(patch.a.edge, next)?;
                let native_b = point(patch.b.edge, if patch.b.reversed { 0.0 } else { 1.0 })?;
                let native_normal = exedra_math::cross(
                    exedra_math::sub(native_next, native_a),
                    exedra_math::sub(native_b, native_a),
                );
                if exedra_math::dot(native_normal, normal) <= 0.0 {
                    return Err(RealizationError::DegenerateFace { face: id });
                }
                for j in 0..a.len() - 1 {
                    let corners = [a_at(j), a_at(j + 1), b_at(j + 1), b_at(j)];
                    // A native strip can survive while its emitted triangle
                    // reverses after narrowing. Check in the placed frame.
                    let fraction = (j as f64 + 0.5) / f64::from(counts[patch.a.edge.index()]);
                    let t = if patch.a.reversed {
                        1.0 - fraction
                    } else {
                        fraction
                    };
                    let uv = face
                        .surface
                        .parameters(point(patch.a.edge, t)?)
                        .ok_or(RealizationError::UnsupportedSurfaceTrim { face: id })?;
                    let normal = self
                        .evaluate_support(id, uv)
                        .ok_or(RealizationError::UnsupportedSurfaceTrim { face: id })?
                        .normal;
                    if let Some(sides) = &sides
                        && (j == 0 || j + 2 == a.len())
                    {
                        let left = [
                            strip::Sample {
                                vertex: corners[0],
                                fraction: 0.0,
                            },
                            strip::Sample {
                                vertex: corners[3],
                                fraction: 1.0,
                            },
                        ];
                        let right = [
                            strip::Sample {
                                vertex: corners[1],
                                fraction: 0.0,
                            },
                            strip::Sample {
                                vertex: corners[2],
                                fraction: 1.0,
                            },
                        ];
                        strip::emit_panel(
                            if j == 0 { &sides[0] } else { &left },
                            if j + 2 == a.len() { &sides[1] } else { &right },
                            id,
                            face_region,
                            crate::geometry::vector(placement, normal),
                            &emitted,
                            &mut triangles,
                            &mut regions,
                            &mut native_faces,
                        )?;
                        continue;
                    }
                    for triangle in [
                        [corners[0], corners[1], corners[2]],
                        [corners[0], corners[2], corners[3]],
                    ] {
                        let p = triangle.map(|k| emitted[k as usize].map(f64::from));
                        let emitted_normal = exedra_math::cross(
                            exedra_math::sub(p[1], p[0]),
                            exedra_math::sub(p[2], p[0]),
                        );
                        if exedra_math::dot(
                            emitted_normal,
                            crate::geometry::vector(placement, normal),
                        ) <= 0.0
                        {
                            return Err(RealizationError::DegenerateFace { face: id });
                        }
                    }
                    emit(
                        &mut triangles,
                        [corners[0], corners[1], corners[2]],
                        id,
                        face_region,
                        &emitted,
                        &mut regions,
                        &mut native_faces,
                    )?;
                    emit(
                        &mut triangles,
                        [corners[0], corners[2], corners[3]],
                        id,
                        face_region,
                        &emitted,
                        &mut regions,
                        &mut native_faces,
                    )?;
                }
            }
        }
        // The indexed builder consumes flat triangles without owning one
        // corner/attribute allocation per face. No welding is requested: native
        // vertex identity and edge sample order remain authoritative.
        let mut mesh = Mesh::from_indexed_triangles(&emitted, &triangles, &BuildParams::default())
            .map_err(RealizationError::Build)?;
        let vertex_ids: Vec<_> = mesh.vertices().collect();
        let face_ids: Vec<_> = mesh.faces().collect();
        let mut pole_records = Vec::with_capacity(pole_corners.len());
        for seed in pole_corners {
            let face = face_ids[seed.triangle];
            let vertex = vertex_ids[seed.vertex as usize];
            let corner = mesh
                .face_loop(face)
                .find(|c| mesh.to_vertex(*c) == Some(vertex))
                .ok_or(RealizationError::DegenerateFace { face: seed.face })?;
            pole_records.push(PoleCorner {
                corner,
                face: seed.face,
                vertex: seed.native_vertex,
                uv: seed.uv,
            });
        }
        pole_records.sort_unstable_by_key(|record| record.corner.index());

        let normal_count = trim_normals
            .len()
            .checked_mul(3)
            .ok_or(RealizationError::Budget {
                storage: "trim corner normals",
                required: usize::MAX,
                maximum: u32::MAX,
            })?;
        if normal_count > u32::MAX as usize {
            return Err(RealizationError::Budget {
                storage: "trim corner normals",
                required: normal_count,
                maximum: u32::MAX,
            });
        }
        let normal_storage_count = normal_count.saturating_add(if normal_count == 0 {
            0
        } else {
            pole_records.len()
        });
        check_table_capacity::<(exedra_mesh::CornerId, [f32; 3])>(
            "trim corner normals",
            normal_storage_count,
        )?;
        let mut trim_corner_normals = Vec::with_capacity(normal_storage_count);
        for record in &trim_normals {
            let triangle = record.triangle as usize;
            let corners = triangles[triangle].map(|i| vertex_ids[i as usize]);
            for corner in mesh.face_loop(face_ids[triangle]) {
                let vertex = mesh.to_vertex(corner).expect("new triangle corner");
                let index = corners
                    .iter()
                    .position(|&v| v == vertex)
                    .expect("triangle vertex preserved by indexed builder");
                trim_corner_normals.push((corner, record.normals[index]));
            }
        }
        if normal_count > 0 {
            // Mix pole overrides into the same sorted writes, so a mixed body
            // never repeatedly inserts pole entries into a large trim layer.
            for record in &pole_records {
                let normal =
                    poles::limit_normal(&self.faces()[record.face.index()], record.uv, placement)
                        .ok_or(RealizationError::DegenerateFace { face: record.face })?;
                trim_corner_normals.push((record.corner, narrow(normal)));
            }
        }
        let trim_normal_scratch_bytes = trim_work.normal_scratch_bytes.max(
            trim_normals.capacity() * size_of::<trimmed::TriangleNormals>()
                + trim_corner_normals.capacity() * size_of::<(exedra_mesh::CornerId, [f32; 3])>(),
        );
        drop(trim_normals);
        // Sparse writes in ascending corner order append rather than shifting
        // an already populated layer for every triangle corner.
        trim_corner_normals.sort_unstable_by_key(|(corner, _)| corner.index());

        let edge_updates: Vec<_> = mesh
            .half_edges()
            .filter_map(|edge| {
                if mesh.canonical_edge(edge) != Some(edge) {
                    return None;
                }
                let a = mesh.from_vertex(edge)?.index();
                let b = mesh.to_vertex(edge)?.index();
                let (a, b) = ordered(a, b);
                // Generic chart refinement appends interior vertices after
                // native boundary sharpness was indexed; they carry no sharp edge.
                let (&start, &end) = (
                    sharp_offsets.get(a as usize)?,
                    sharp_offsets.get(a as usize + 1)?,
                );
                let row = &sharp[start..end];
                row.binary_search_by_key(&b, |&(_, b, _, _)| b)
                    .ok()
                    .map(|i| (edge, row[i].2))
            })
            .collect();
        let mut edit = mesh.edit();
        for (&face, &region) in face_ids.iter().zip(&regions) {
            exedra_mesh::op::set_face_region(&mut edit, face, region)
                .expect("newly emitted face is live");
        }
        for (edge, value) in edge_updates {
            exedra_mesh::op::set_edge_sharpness(&mut edit, edge, value)
                .expect("newly emitted edge and twin are live");
        }
        for (corner, normal) in trim_corner_normals {
            exedra_mesh::op::set_corner_normal_override(&mut edit, corner, Some(normal))
                .expect("newly emitted trim corner is live");
        }
        for record in pole_records.iter().filter(|_| normal_count == 0) {
            let normal =
                poles::limit_normal(&self.faces()[record.face.index()], record.uv, placement)
                    .ok_or(RealizationError::DegenerateFace { face: record.face })?;
            exedra_mesh::op::set_corner_normal_override(
                &mut edit,
                record.corner,
                Some(narrow(normal)),
            )
            .expect("newly emitted pole corner is live");
        }
        let _: () = edit.finish();
        let edge_chains = EdgeChains {
            vertices: chains
                .vertices
                .into_iter()
                .map(|i| vertex_ids[i as usize])
                .collect(),
            offsets: chains.offsets,
        };
        let work = RealizationWork {
            attempts: 1,
            completed_boundary_samples: edge_chains.vertices.len(),
            completed_triangles: triangles.len(),
            trim_refinement_batches: trim_work.batches,
            trim_triangle_tests: trim_work.triangle_tests,
            trim_generated_vertices: trim_work.vertices,
            trim_normal_corners: normal_count,
            trim_normal_scratch_bytes,
        };
        Ok(RealizedBody {
            mesh,
            mesh_faces: face_ids.into_iter().zip(native_faces).collect(),
            mesh_vertices: vertex_ids.into_iter().zip(sources).collect(),
            edge_chains,
            pole_corners: PoleCorners {
                records: pole_records.into_boxed_slice(),
            },
            report: RealizationReport {
                tolerance: policy.tolerance,
                geometry_gap: validation.max_gap,
                max_curve_deviation: curve_deviation,
                max_surface_deviation: surface_deviation,
                max_vertex_displacement: displacement,
                max_ruling_displacement: ruling_displacement,
                max_deviation: curve_deviation.max(surface_deviation)
                    + displacement
                    + ruling_displacement
                    + 2.0 * validation.max_gap,
                edge_segments: counts,
                work,
                curve_refinement: None,
            },
        })
    }

    fn rotational_patch(&self, id: FaceId, gap: f64) -> Result<Strip, RealizationError> {
        let face = self.face(id).expect("known face");
        let refuse = || RealizationError::UnsupportedSurfaceTrim { face: id };
        if matches!(face.surface, Surface::Cylinder(_))
            && face
                .loops
                .iter()
                .filter_map(|&id| self.boundary_loop(id))
                .flat_map(|loop_| &loop_.coedges)
                .any(|c| {
                    matches!(
                        self.edges()[c.edge.index()].regular(),
                        Some((Curve::Ellipse(_), _))
                    )
                })
        {
            let band = self
                .cylindrical_band(id, &ValidationPolicy { max_gap: gap })
                .map_err(|_| refuse())?;
            return Ok(Strip {
                a: band.pair[0],
                b: band.pair[1],
                translation: band.translation,
                variable_axial_rulings: band.variable_axial_rulings,
                center: band.center,
                radius_scale: 1.0,
                correspondence_gap: band.max_gap(),
                sides: None,
            });
        }

        let (mut a, mut b) = if matches!(face.surface, Surface::Plane(_)) && face.loops.len() == 2 {
            let rings = [
                self.boundary_loop(face.loops[0]).expect("validated loop"),
                self.boundary_loop(face.loops[1]).expect("validated loop"),
            ];
            if rings.iter().any(|r| r.coedges.len() != 1) {
                return Err(refuse());
            }
            let (a, b) = (rings[0].coedges[0], rings[1].coedges[0]);
            if [a, b].iter().any(|c| {
                let edge = &self.edges()[c.edge.index()];
                !matches!(edge.regular(), Some((Curve::Circle(_), _))) || edge.start != edge.end
            }) {
                return Err(refuse());
            }
            let circle = |coedge: Coedge| {
                let Some((Curve::Circle(circle), _)) = self.edges()[coedge.edge.index()].regular()
                else {
                    unreachable!()
                };
                circle
            };
            let normal = face.surface.frame().rows.map(|r| r[2] * face.sense.sign());
            let winding = |coedge: Coedge| {
                exedra_math::dot(circle(coedge).frame().rows.map(|r| r[2]), normal)
                    * if coedge.reversed { -1.0 } else { 1.0 }
            };
            // Loop zero is the authored outer boundary. Choosing a reference
            // ring must never reorder outer/hole semantics or repair winding.
            if circle(a).radius() <= circle(b).radius()
                || winding(a) <= 0.0
                || winding(b) >= 0.0
                || [a, b].iter().any(|c| {
                    (self.edges()[c.edge.index()]
                        .regular()
                        .expect("checked circle")
                        .1
                        .span()
                        - core::f64::consts::TAU)
                        .abs()
                        > 64.0 * f64::EPSILON
                })
            {
                return Err(refuse());
            }
            (a, b)
        } else {
            if face.loops.len() != 1 {
                return Err(refuse());
            }
            let coedges = &self
                .boundary_loop(face.loops[0])
                .expect("validated loop")
                .coedges;
            if coedges.len() < 4 {
                return Err(refuse());
            }
            let first = coedges
                .iter()
                .position(|c| {
                    matches!(
                        self.edges()[c.edge.index()].regular(),
                        Some((Curve::Circle(_), _))
                    )
                })
                .ok_or_else(refuse)?;
            let mut second = None;
            for (i, c) in coedges.iter().enumerate().filter(|(i, _)| *i != first) {
                match self.edges()[c.edge.index()].regular() {
                    Some((Curve::Circle(_), _)) if second.is_none() => second = Some(i),
                    Some((Curve::Line(_), _)) => {}
                    _ => return Err(refuse()),
                }
            }
            let second = second.ok_or_else(refuse)?;
            let between = (second + coedges.len() - first) % coedges.len();
            if between < 2
                || coedges.len() - between < 2
                || coedges.iter().enumerate().any(|(i, c)| {
                    i != first
                        && i != second
                        && !matches!(
                            self.edges()[c.edge.index()].regular(),
                            Some((Curve::Line(_), _))
                        )
                })
            {
                return Err(refuse());
            }
            let (a, b) = (coedges[first], coedges[second]);
            (a, b)
        };
        let axis = face.surface.frame().rows.map(|r| r[2]);
        let axis_length = distance(axis, [0.0; 3]);
        let axis = axis.map(|v| v / axis_length);
        let signed = |coedge: Coedge| {
            let edge = &self.edges()[coedge.edge.index()];
            let Some((Curve::Circle(circle), range)) = edge.regular() else {
                unreachable!();
            };
            exedra_math::dot(circle.frame().rows.map(|r| r[2]), axis).signum()
                * range.span()
                * if coedge.reversed { -1.0 } else { 1.0 }
        };
        let circle = |coedge: Coedge| {
            let Some((Curve::Circle(circle), _)) = self.edges()[coedge.edge.index()].regular()
            else {
                unreachable!()
            };
            circle
        };
        // Use the larger reference ring for cones. The radial map then
        // contracts, so its reference-boundary error is never amplified.
        let conical = !matches!(face.surface, Surface::Cylinder(_));
        if conical && circle(a).radius() < circle(b).radius() {
            core::mem::swap(&mut a, &mut b);
        }
        let (lower, upper) = (circle(a), circle(b));
        let radius = lower.radius().max(upper.radius());
        if (signed(a) + signed(b)).abs() * radius > gap.max(64.0 * f64::EPSILON * radius) {
            return Err(refuse());
        }
        let radius_scale = if conical {
            upper.radius() / lower.radius()
        } else {
            1.0
        };
        let lower_axis = lower.frame().rows.map(|r| r[2]);
        let upper_axis = upper.frame().rows.map(|r| r[2]);
        let height = exedra_math::dot(
            exedra_math::sub(
                upper.frame().rows.map(|r| r[3]),
                lower.frame().rows.map(|r| r[3]),
            ),
            axis,
        );
        let translation = axis.map(|v| v * height);
        // Rectangular eligibility is independent of caller support-gap slack.
        // Parallel circle planes and axial boundary rulings are required;
        // the measured residual still contributes to the report, rather than
        // being silently excused by this numerical comparison.
        let angular_slack = 64.0 * crate::geometry::FRAME_TOLERANCE;
        let linear_slack = 64.0 * f64::EPSILON * (radius + height.abs());
        if distance(
            exedra_math::sub(
                upper.frame().rows.map(|r| r[3]),
                lower.frame().rows.map(|r| r[3]),
            ),
            translation,
        ) > linear_slack
            || (!conical && (upper.radius() - lower.radius()).abs() > linear_slack)
        {
            return Err(refuse());
        }
        if distance(exedra_math::cross(lower_axis, axis), [0.0; 3]) > angular_slack
            || distance(exedra_math::cross(upper_axis, axis), [0.0; 3]) > angular_slack
        {
            return Err(refuse());
        }
        for fraction in [0.0, 1.0] {
            let lower = self
                .edge_point(a.edge, if a.reversed { 1.0 - fraction } else { fraction })
                .ok_or_else(refuse)?;
            let upper = self
                .edge_point(b.edge, if b.reversed { fraction } else { 1.0 - fraction })
                .ok_or_else(refuse)?;
            let radial_change = exedra_math::sub(lower, circle(a).frame().rows.map(|r| r[3]))
                .map(|v| v * (radius_scale - 1.0));
            if distance(
                exedra_math::sub(upper, lower),
                exedra_math::add(translation, radial_change),
            ) > linear_slack
            {
                return Err(refuse());
            }
        }
        Ok(Strip {
            a,
            b,
            translation,
            variable_axial_rulings: false,
            center: lower.frame().rows.map(|r| r[3]),
            radius_scale,
            correspondence_gap: 0.0,
            sides: strip::Sides::prepare(self, id, a, b, linear_slack)?,
        })
    }

    fn edge_sharpness(&self, id: EdgeId, incidence: Incidence) -> f32 {
        let [Some(a), Some(b)] = incidence.faces else {
            return 1.0;
        };
        if a == b {
            return 0.0;
        }
        let Some(point) = self.edge_point(id, 0.5) else {
            return 1.0;
        };
        let id_edge = id;
        let normal = |id| {
            self.face(id)
                .and_then(|f| {
                    if let Surface::Ruled(ruled) = &f.surface {
                        let edge = self.edge(id_edge)?;
                        let (curve, range) = edge.regular()?;
                        let slack = ruled
                            .curves()
                            .iter()
                            .map(crate::geometry::CurveSpan::magnitude)
                            .fold(0.0, f64::max)
                            * 128.0
                            * crate::geometry::FRAME_TOLERANCE;
                        for reversed in [false, true] {
                            let actual =
                                crate::geometry::CurveSpan::new(curve.clone(), range, reversed)
                                    .ok()?;
                            for side in 0..4 {
                                if ruled.boundary(side)?.alignment(&actual) <= slack {
                                    return Some(match side {
                                        0 => [0.5, 0.0],
                                        1 => [1.0, 0.5],
                                        2 => [0.5, 1.0],
                                        _ => [0.0, 0.5],
                                    });
                                }
                            }
                        }
                        None
                    } else {
                        f.surface.parameters(point)
                    }
                })
                .and_then(|uv| self.evaluate_support(id, uv))
                .map(|e| e.normal)
        };
        match (normal(a), normal(b)) {
            (Some(a), Some(b)) if exedra_math::dot(a, b) > 1.0 - 1e-12 => 0.0,
            _ => 1.0,
        }
    }
}

impl Chains {
    fn get_partial(&self, i: usize) -> &[u32] {
        &self.vertices[self.offsets[i]..]
    }
}
fn placement_roundoff(frame: Placement3, point: [f64; 3]) -> f64 {
    frame
        .rows
        .into_iter()
        .map(|r| {
            32.0 * f64::EPSILON
                * (r[3].abs()
                    + r[0].abs() * point[0].abs()
                    + r[1].abs() * point[1].abs()
                    + r[2].abs() * point[2].abs())
                + 32.0 * f64::from_bits(1)
        })
        .fold(0.0, f64::max)
}

fn ordered(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = exedra_math::sub(a, b);
    libm::hypot(libm::hypot(d[0], d[1]), d[2])
}
fn project(frame: Placement3, p: [f64; 3]) -> [f64; 2] {
    let d = exedra_math::sub(p, frame.rows.map(|r| r[3]));
    [
        exedra_math::dot(d, frame.rows.map(|r| r[0])),
        exedra_math::dot(d, frame.rows.map(|r| r[1])),
    ]
}
fn emit(
    triangles: &mut Vec<[u32; 3]>,
    corners: [u32; 3],
    native: FaceId,
    region: u32,
    positions: &[[f32; 3]],
    regions: &mut Vec<u32>,
    faces: &mut Vec<FaceId>,
) -> Result<(), RealizationError> {
    let p = corners.map(|i| positions[i as usize].map(f64::from));
    let normal = exedra_math::cross(exedra_math::sub(p[1], p[0]), exedra_math::sub(p[2], p[0]));
    if normal == [0.0; 3] {
        return Err(RealizationError::DegenerateFace { face: native });
    }
    triangles.push(corners);
    regions.push(region);
    faces.push(native);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::tests::cylinder_parts;
    use crate::{AnalyticShellBuilder, RegionId};
    use alloc::vec;
    use core::cell::Cell;

    fn cylinder() -> Body {
        Body::try_from_parts(cylinder_parts(), &ValidationPolicy { max_gap: 1e-12 }).unwrap()
    }

    fn strip(start: f64, span: f64, offset: f64, gap: f64) -> Body {
        use crate::body::{BodyParts, BoundaryLoop, Edge, LoopId, Sense};
        use crate::geometry::{ParameterDomain, ParameterRange};
        let lower = Curve::circle(Placement3::IDENTITY, 1.0).unwrap();
        let upper = Curve::circle(Placement3::translate(0.0, 0.0, 1.0), 1.0).unwrap();
        let a = ParameterRange::new(start, start + span).unwrap();
        let b = ParameterRange::new(start + offset, start + offset + span).unwrap();
        let vertices = vec![
            lower.evaluate(a.bounds()[0]).unwrap().point,
            lower.evaluate(a.bounds()[1]).unwrap().point,
            upper.evaluate(b.bounds()[0]).unwrap().point,
            upper.evaluate(b.bounds()[1]).unwrap().point,
        ];
        let line = |from: usize, to: usize| Edge {
            geometry: crate::body::EdgeGeometry::Curve {
                curve: Curve::line(
                    vertices[from],
                    exedra_math::sub(vertices[to], vertices[from]),
                )
                .unwrap(),
                range: ParameterRange::UNIT,
            },
            start: VertexId(u32::try_from(from).unwrap()),
            end: VertexId(u32::try_from(to).unwrap()),
            role: (),
        };
        let edges = vec![
            Edge {
                geometry: crate::body::EdgeGeometry::Curve {
                    curve: lower,
                    range: a,
                },
                start: VertexId(0),
                end: VertexId(1),
                role: (),
            },
            Edge {
                geometry: crate::body::EdgeGeometry::Curve {
                    curve: upper,
                    range: b,
                },
                start: VertexId(2),
                end: VertexId(3),
                role: (),
            },
            line(1, 3),
            line(0, 2),
        ];
        Body::try_from_parts(
            BodyParts {
                trims: Vec::new(),
                vertices,
                edges,
                loops: vec![BoundaryLoop {
                    coedges: vec![
                        Coedge {
                            edge: EdgeId(0),
                            reversed: false,
                        },
                        Coedge {
                            edge: EdgeId(2),
                            reversed: false,
                        },
                        Coedge {
                            edge: EdgeId(1),
                            reversed: true,
                        },
                        Coedge {
                            edge: EdgeId(3),
                            reversed: true,
                        },
                    ],
                }],
                faces: vec![Face {
                    surface: Surface::cylinder(Placement3::IDENTITY, 1.0).unwrap(),
                    domain: ParameterDomain {
                        u: a,
                        v: ParameterRange::UNIT,
                    },
                    sense: Sense::Forward,
                    loops: vec![LoopId(0)],
                    role: (),
                }],
            },
            &ValidationPolicy { max_gap: gap },
        )
        .unwrap()
    }

    #[test]
    fn planar_annulus_fast_path_preserves_authored_outer_and_hole_roles() {
        use crate::body::{BoundaryLoop, Coedge, Face, LoopId, Sense};
        use crate::geometry::{ParameterDomain, ParameterRange};
        let mut parts = cylinder_parts();
        parts.vertices[1] = [1.0, 0.0, 0.0];
        parts.edges.truncate(2);
        *parts.edges[1].regular_mut().unwrap().0 =
            Curve::circle(Placement3::IDENTITY, 1.0).unwrap();
        parts.loops = vec![
            BoundaryLoop {
                coedges: vec![Coedge {
                    edge: EdgeId(1),
                    reversed: false,
                }],
            },
            BoundaryLoop {
                coedges: vec![Coedge {
                    edge: EdgeId(0),
                    reversed: true,
                }],
            },
        ];
        parts.faces = vec![Face {
            surface: Surface::plane(Placement3::IDENTITY).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(-2.0, 2.0).unwrap(),
                v: ParameterRange::new(-2.0, 2.0).unwrap(),
            },
            sense: Sense::Reverse,
            loops: vec![LoopId(0), LoopId(1)],
            role: (),
        }];
        let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-12 }).unwrap();
        assert!(
            matches!(
                body.realize(Placement3::IDENTITY, &RealizationPolicy::default()),
                Err(RealizationError::Arrangement {
                    face: FaceId(0),
                    ..
                })
            ),
            "an outside hole and reversed winding must not be reinterpreted by radius sorting"
        );
    }

    #[test]
    fn non_axial_rulings_do_not_pass_as_rectangular_strips_under_a_loose_gap() {
        let body = strip(0.0, 1.0, 0.01, 0.000013);
        let result = body.realize(
            Placement3::IDENTITY,
            &RealizationPolicy {
                tolerance: 0.008,
                geometry_gap: 0.000013,
                ..RealizationPolicy::default()
            },
        );
        assert!(matches!(
            result,
            Err(RealizationError::UnsupportedSurfaceTrim { face: FaceId(0) })
        ));
    }

    #[test]
    fn reused_and_changed_phase_grids_match_independent_native_evaluation() {
        use crate::geometry::ParameterRange;
        for phase in [0.0, 0.125] {
            let mut parts = cylinder_parts();
            *parts.edges[1].regular_mut().unwrap().0 = Curve::circle(
                Placement3::rotate_z_then_translate(-phase, 0.0, 0.0, 3.0),
                2.0,
            )
            .unwrap();
            *parts.edges[1].regular_mut().unwrap().1 =
                ParameterRange::new(phase, phase + core::f64::consts::TAU).unwrap();
            let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-12 }).unwrap();
            let placement = Placement3::rotate_y_then_translate(0.7, 3.0, -2.0, 1.0);
            let realized = body
                .realize(
                    placement,
                    &RealizationPolicy {
                        min_arc_segments: 64,
                        max_edge_segments: 64,
                        ..RealizationPolicy::default()
                    },
                )
                .unwrap();
            for &(vertex, source) in &realized.mesh_vertices {
                let native = match source {
                    VertexSource::Face { face, uv } => {
                        body.evaluate_support(face, uv).unwrap().point
                    }
                    VertexSource::Vertex(id) => body.vertices()[id.index()],
                    VertexSource::Edge { edge, parameter } => {
                        let curve = &body.edges()[edge.index()];
                        curve
                            .regular()
                            .unwrap()
                            .0
                            .evaluate(curve.regular().unwrap().1.at(parameter).unwrap())
                            .unwrap()
                            .point
                    }
                };
                assert_eq!(
                    realized.mesh.vertex_position(vertex).copied(),
                    Some(narrow(placement.transform_point(native))),
                );
            }
        }
    }

    #[test]
    fn large_angle_origins_report_actual_sample_spacing_after_parameter_rounding() {
        let body = strip(1e15, 1.125, 0.0, 1e-12);
        for placement in [
            Placement3::IDENTITY,
            Placement3::rotate_y_then_translate(0.7, 3.0, -2.0, 1.0),
        ] {
            let policy = RealizationPolicy {
                tolerance: 0.011,
                ..Default::default()
            };
            let initial = body
                .realize_once(placement, &policy, policy.tolerance, |id, _| id.0)
                .unwrap();
            assert!(!initial.report.within_tolerance());
            assert!(initial.report.max_curve_deviation >= 0.0175);
            let result = body.realize(placement, &policy).unwrap();
            assert!(result.report.max_deviation <= initial.report.max_deviation);
            assert_eq!(result.report.work.attempts, 2);
            assert_eq!(
                result
                    .report
                    .curve_refinement
                    .as_ref()
                    .unwrap()
                    .initial_deviation,
                initial.report.max_deviation
            );
            for edge in result.mesh.half_edges() {
                let a = result
                    .mesh
                    .vertex_position(result.mesh.from_vertex(edge).unwrap())
                    .unwrap();
                let b = result
                    .mesh
                    .vertex_position(result.mesh.to_vertex(edge).unwrap())
                    .unwrap();
                let midpoint = core::array::from_fn(|i| (f64::from(a[i]) + f64::from(b[i])) * 0.5);
                let xy = project(placement, midpoint);
                let error = (1.0 - libm::hypot(xy[0], xy[1])).abs();
                assert!(
                    error <= result.report.max_deviation,
                    "{error} exceeds {:?}",
                    result.report
                );
            }
        }
    }

    #[test]
    fn paired_parameter_grids_and_reversed_face_traversal_contribute_ruling_error() {
        use crate::body::Sense;
        use crate::geometry::ParameterRange;
        let mut parts = strip(1e15, 1.125, 0.0, 1e-12).into_parts();
        *parts.edges[0].regular_mut().unwrap().0 = Curve::circle(
            Placement3::rotate_z_then_translate(1e15, 0.0, 0.0, 0.0),
            1.0,
        )
        .unwrap();
        *parts.edges[0].regular_mut().unwrap().1 = ParameterRange::new(0.0, 1.125).unwrap();
        for reverse in [false, true] {
            let mut parts = parts.clone();
            if reverse {
                parts.faces[0].sense = Sense::Reverse;
                parts.loops[0].coedges.reverse();
                for coedge in &mut parts.loops[0].coedges {
                    coedge.reversed = !coedge.reversed;
                }
            }
            let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-12 }).unwrap();
            let result = body
                .realize(
                    Placement3::IDENTITY,
                    &RealizationPolicy {
                        tolerance: 0.011,
                        ..RealizationPolicy::default()
                    },
                )
                .unwrap();
            assert!(result.report.max_ruling_displacement > 0.06);
            assert!(!result.report.within_tolerance());
            for edge in result.mesh.half_edges() {
                let a = result
                    .mesh
                    .vertex_position(result.mesh.from_vertex(edge).unwrap())
                    .unwrap();
                let b = result
                    .mesh
                    .vertex_position(result.mesh.to_vertex(edge).unwrap())
                    .unwrap();
                let x = (f64::from(a[0]) + f64::from(b[0])) * 0.5;
                let y = (f64::from(a[1]) + f64::from(b[1])) * 0.5;
                assert!((1.0 - libm::hypot(x, y)).abs() <= result.report.max_deviation);
            }
        }
    }

    fn volume(mesh: &Mesh) -> f64 {
        mesh.faces()
            .map(|face| {
                let points: Vec<_> = mesh
                    .face_loop(face)
                    .map(|c| {
                        mesh.vertex_position(mesh.from_vertex(c).unwrap())
                            .unwrap()
                            .map(f64::from)
                    })
                    .collect();
                assert_eq!(points.len(), 3);
                exedra_math::dot(points[0], exedra_math::cross(points[1], points[2])) / 6.0
            })
            .sum()
    }

    #[test]
    fn periodic_cylinder_meshes_shared_seam_and_both_caps_without_a_construction_record() {
        let body = cylinder();
        let calls = Cell::new(0);
        let realized = body
            .realize_with_regions(
                Placement3::IDENTITY,
                &RealizationPolicy::default(),
                |_, _| {
                    calls.set(calls.get() + 1);
                    9
                },
            )
            .unwrap();
        assert_eq!(calls.get(), 3, "one semantic lookup per native face");
        assert!(realized.mesh.boundary_loops().unwrap().is_empty());
        assert!(realized.report.within_tolerance());
        let n = realized.report.edge_segments[0];
        assert_eq!(realized.report.edge_segments, vec![n, n, 1]);
        let regions = realized
            .mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .unwrap();
        assert!(
            realized
                .mesh
                .faces()
                .all(|id| regions.get(id.as_id()) == Some(&9))
        );
        let sharp = realized
            .mesh
            .half_edges()
            .filter(|&edge| {
                realized.mesh.canonical_edge(edge) == Some(edge)
                    && realized.mesh.edge_sharpness(edge) == Some(1.0)
            })
            .count();
        assert_eq!(
            sharp,
            2 * n as usize,
            "only cap rims are sharp; seam and diagonals remain smooth"
        );
        assert_eq!(realized.mesh_vertices.len(), 2 * n as usize);
        assert_eq!(realized.mesh_faces.len(), 4 * n as usize - 4);
        let expected =
            0.5 * f64::from(n) * 4.0 * libm::sin(core::f64::consts::TAU / f64::from(n)) * 3.0;
        assert!((volume(&realized.mesh) - expected).abs() < 1e-5);
        assert!(volume(&realized.mesh) < 12.0 * core::f64::consts::PI);
        for index in [0, 1] {
            let ring = realized.edge_chains.get(EdgeId(index)).unwrap();
            assert_eq!(ring.first(), ring.last());
            assert_eq!(ring.len(), n as usize + 1);
        }
        assert_eq!(realized.edge_chains.get(EdgeId(2)).unwrap().len(), 2);
        assert_eq!(realized.mesh_faces.len(), realized.mesh.faces().count());
        for (_, source) in &realized.mesh_vertices {
            assert!(match source {
                VertexSource::Face { face, uv } => body.evaluate_support(*face, *uv).is_some(),
                VertexSource::Vertex(id) => body.vertices().get(id.index()).is_some(),
                VertexSource::Edge { edge, parameter } =>
                    body.edge_point(*edge, *parameter).is_some(),
            });
        }
    }

    #[test]
    fn profile_holes_and_both_pocket_directions_realize_from_the_retained_body() {
        use crate::extrusion::{End, Extrusion, Pocket};
        use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
        let policy = ProfilePolicy::new(1e-9);
        let outer = Loop::new(vec![
            Segment::line([10.0, 0.0]),
            Segment::line([10.0, 6.0]),
            Segment::line([0.0, 6.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        let circle = |x, y, r, bulge| {
            Loop::new(vec![
                Segment::arc([x - r, y], bulge),
                Segment::arc([x + r, y], bulge),
            ])
            .unwrap()
        };
        let profile = Profile::new(outer, vec![circle(7.0, 3.0, 1.0, -1.0)], &policy).unwrap();
        let mut solid = Extrusion::new(profile, Placement3::IDENTITY, 3.0).unwrap();
        for (end, x) in [(End::Start, 2.0), (End::End, 4.0)] {
            let region = Profile::new(circle(x, 3.0, 0.5, 1.0), vec![], &policy).unwrap();
            solid = solid
                .with_pocket(Pocket::from_profile(&region, end, 1.0).unwrap(), &policy)
                .unwrap();
        }
        let expected = solid.volume();
        let body = solid.body().clone();
        drop(solid);
        let realized = body
            .realize(
                Placement3::IDENTITY,
                &RealizationPolicy {
                    tolerance: 0.001,
                    ..RealizationPolicy::default()
                },
            )
            .unwrap();
        assert!(realized.mesh.boundary_loops().unwrap().is_empty());
        assert!(realized.report.within_tolerance());
        assert!((volume(&realized.mesh) - expected).abs() < 0.02);
    }

    #[test]
    fn placement_narrowing_is_in_the_report_even_when_more_arc_samples_cannot_help() {
        let body = cylinder();
        let placement = Placement3::translate(0.0, 0.0, 1_000_000.03);
        let policy = RealizationPolicy {
            tolerance: 0.02,
            ..RealizationPolicy::default()
        };
        let realized = body.realize(placement, &policy).unwrap();
        assert!(!realized.report.within_tolerance());
        assert!((realized.report.max_vertex_displacement - 0.03).abs() < 1e-9);
        let ring = realized.edge_chains.get(EdgeId(0)).unwrap();
        let n = ring.len() - 1;
        for (i, pair) in ring.windows(2).enumerate() {
            let a = realized.mesh.vertex_position(pair[0]).unwrap();
            let b = realized.mesh.vertex_position(pair[1]).unwrap();
            let midpoint =
                core::array::from_fn(|axis| (f64::from(a[axis]) + f64::from(b[axis])) * 0.5);
            let native = placement.transform_point(
                body.edge_point(EdgeId(0), (i as f64 + 0.5) / n as f64)
                    .unwrap(),
            );
            assert!(distance(midpoint, native) <= realized.report.max_deviation);
        }
    }

    #[test]
    fn native_planar_builder_realizes_with_regions_and_refuses_precision_collapse() {
        let mut builder = AnalyticShellBuilder::new();
        let ids = [
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ]
        .map(|p| builder.push_vertex(p));
        builder.add_planar_face(&ids, RegionId(27)).unwrap();
        let shell = builder.build().unwrap();
        let result = shell
            .body()
            .realize_with_regions(
                Placement3::IDENTITY,
                &RealizationPolicy::default(),
                |_, face| face.role.0,
            )
            .unwrap();
        assert_eq!(result.mesh.faces().count(), 2);
        assert!(
            result
                .mesh_faces
                .iter()
                .all(|(_, native)| *native == FaceId(0))
        );
        let mut builder = AnalyticShellBuilder::new();
        let ids = [
            [1e6, 0.0, 0.0],
            [1e6 + 0.001, 0.0, 0.0],
            [1e6 + 0.001, 2.0, 0.0],
            [1e6, 2.0, 0.0],
        ]
        .map(|p| builder.push_vertex(p));
        builder.add_planar_face(&ids, RegionId(0)).unwrap();
        let shell = builder.build().unwrap();
        assert!(matches!(
            shell
                .body()
                .realize(Placement3::IDENTITY, &RealizationPolicy::default()),
            Err(RealizationError::CollapsedEdge { .. })
        ));
    }

    #[test]
    fn budgets_and_unsupported_trim_are_explicit_refusals() {
        let body = cylinder();
        assert!(matches!(
            body.realize(
                Placement3::IDENTITY,
                &RealizationPolicy {
                    max_vertices: 2,
                    ..RealizationPolicy::default()
                }
            ),
            Err(RealizationError::Budget {
                storage: "vertices",
                ..
            })
        ));
        assert!(matches!(
            body.realize(
                Placement3::IDENTITY,
                &RealizationPolicy {
                    tolerance: 1e-9,
                    max_edge_segments: 4,
                    ..RealizationPolicy::default()
                }
            ),
            Err(RealizationError::Sampling { .. })
        ));
        assert!(matches!(
            body.realize(
                Placement3::IDENTITY,
                &RealizationPolicy {
                    min_arc_segments: u32::MAX,
                    max_edge_segments: u32::MAX,
                    max_vertices: u32::MAX,
                    max_boundary_samples: u32::MAX,
                    ..RealizationPolicy::default()
                }
            ),
            Err(RealizationError::Budget {
                storage: "boundary samples",
                ..
            })
        ));
        let mut parts = cylinder_parts();
        parts.faces[0].loops = vec![crate::body::LoopId(1)];
        let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-12 }).unwrap();
        assert_eq!(
            body.realize(Placement3::IDENTITY, &RealizationPolicy::default())
                .unwrap_err(),
            RealizationError::UnsupportedSurfaceTrim { face: FaceId(0) }
        );
    }
    fn translated_bore() -> crate::bore::BoredPlate {
        use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
        let size = 1e-6;
        let outline = Loop::new(vec![
            Segment::line([35.0 * size, -25.0 * size]),
            Segment::line([35.0 * size, 25.0 * size]),
            Segment::line([-35.0 * size, 25.0 * size]),
            Segment::line([-35.0 * size, -25.0 * size]),
        ])
        .unwrap();
        let source = crate::extrusion::Extrusion::new(
            Profile::new(outline, vec![], &ProfilePolicy::new(1e-8 * size)).unwrap(),
            Placement3::rotate_y_then_translate(0.35, 1000.0 * size, -700.0 * size, 300.0 * size),
            20.0 * size,
        )
        .unwrap();
        let angle = core::f64::consts::PI / 6.0;
        source
            .with_through_bore(
                crate::bore::Bore::new(
                    [-10.0 * size * libm::tan(angle), 0.0, 0.0],
                    [libm::sin(angle), 0.0, libm::cos(angle)],
                    6.0 * size,
                )
                .unwrap(),
                &crate::bore::BorePolicy {
                    separation: 0.1 * size,
                    geometry_gap: 1e-8 * size,
                    min_axis_dot: 0.1,
                },
            )
            .unwrap()
    }
    #[test]
    fn curve_budget_refines_small_error_without_repeating_regions() {
        let model = translated_bore();
        let policy = RealizationPolicy {
            tolerance: 2.5e-8,
            geometry_gap: 1e-14,
            ..Default::default()
        };
        let initial = model
            .body()
            .realize_once(model.frame(), &policy, policy.tolerance, |id, _| 100 + id.0)
            .unwrap();
        assert!(!initial.report.within_tolerance());
        let calls = Cell::new(0);
        let output = model
            .body()
            .realize_with_regions(model.frame(), &policy, |id, _| {
                calls.set(calls.get() + 1);
                100 + id.0
            })
            .unwrap();
        assert!(output.report.within_tolerance(), "{:?}", output.report);
        assert_eq!(output.report.work.attempts, 2);
        let refinement = output.report.curve_refinement.as_ref().unwrap();
        assert!(refinement.initial_deviation > policy.tolerance);
        assert_eq!(refinement.curve_tolerance, 0.5 * policy.tolerance);
        assert_eq!(refinement.outcome, CurveRefinementOutcome::Improved);
        assert_eq!(output.report.tolerance, policy.tolerance);
        assert_eq!(
            output.report.work.completed_boundary_samples,
            initial.edge_chains.vertices.len() + output.edge_chains.vertices.len()
        );
        assert_eq!(
            output.report.work.completed_triangles,
            initial.mesh_faces.len() + output.mesh_faces.len()
        );
        assert!(output.report.work.completed_triangles > output.mesh_faces.len());
        assert!(output.report.work.completed_boundary_samples > output.edge_chains.vertices.len());
        assert_eq!(calls.get(), model.body().faces().len());
        assert!(output.mesh.validate_deep().is_empty());
        assert!(output.mesh.boundary_loops().unwrap().is_empty());
        let regions = output
            .mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .unwrap();
        for &(mesh_face, native_face) in &output.mesh_faces {
            assert_eq!(regions.get(mesh_face.as_id()), Some(&(100 + native_face.0)));
        }
    }
    fn assert_same_geometry(a: &RealizedBody, b: &RealizedBody) {
        assert_eq!(a.mesh_faces, b.mesh_faces);
        assert_eq!(a.mesh_vertices, b.mesh_vertices);
        assert_eq!(a.edge_chains.vertices, b.edge_chains.vertices);
        assert_eq!(a.edge_chains.offsets, b.edge_chains.offsets);
        for (a_id, b_id) in a.mesh.vertices().zip(b.mesh.vertices()) {
            assert_eq!(a.mesh.vertex_position(a_id), b.mesh.vertex_position(b_id));
        }
        for (a_id, b_id) in a.mesh.faces().zip(b.mesh.faces()) {
            let corners = |mesh: &Mesh, id| {
                mesh.face_loop(id)
                    .map(|c| mesh.from_vertex(c))
                    .collect::<Vec<_>>()
            };
            assert_eq!(corners(&a.mesh, a_id), corners(&b.mesh, b_id));
            let region = |mesh: &Mesh, id: exedra_mesh::FaceId| {
                mesh.attrs()
                    .dense(exedra_mesh::attr::FACE_REGION)
                    .and_then(|v| v.get(id.as_id()))
                    .copied()
                    .unwrap_or(0)
            };
            assert_eq!(region(&a.mesh, a_id), region(&b.mesh, b_id));
        }
        for (a_id, b_id) in a.mesh.half_edges().zip(b.mesh.half_edges()) {
            assert_eq!(a.mesh.edge_sharpness(a_id), b.mesh.edge_sharpness(b_id));
        }
    }
    #[test]
    fn refinement_budget_refusal_preserves_original_output_and_region_calls() {
        let model = translated_bore();
        let mut policy = RealizationPolicy {
            tolerance: 2.5e-8,
            geometry_gap: 1e-14,
            ..Default::default()
        };
        let original = model
            .body()
            .realize_once(model.frame(), &policy, policy.tolerance, |id, _| 100 + id.0)
            .unwrap();
        assert!(!original.report.within_tolerance());
        policy.max_edge_segments = *original.report.edge_segments.iter().max().unwrap();
        let calls = Cell::new(0);
        let output = model
            .body()
            .realize_with_regions(model.frame(), &policy, |id, _| {
                calls.set(calls.get() + 1);
                100 + id.0
            })
            .unwrap();
        assert_same_geometry(&original, &output);
        assert_eq!(output.report.max_deviation, original.report.max_deviation);
        assert_eq!(output.report.tolerance, policy.tolerance);
        assert_eq!(output.report.work.attempts, 2);
        assert_eq!(
            output.report.work.completed_boundary_samples,
            original.report.work.completed_boundary_samples
        );
        assert_eq!(
            output.report.work.completed_triangles,
            original.report.work.completed_triangles
        );
        assert!(matches!(
            output.report.curve_refinement.unwrap().outcome,
            CurveRefinementOutcome::Refused(RealizationError::Sampling { .. })
        ));
        assert_eq!(calls.get(), model.body().faces().len());
    }
    #[test]
    fn ordinary_curve_outputs_keep_first_pass_geometry_and_work() {
        let body = cylinder();
        for tolerance in [0.1, 0.01, 0.001] {
            let policy = RealizationPolicy {
                tolerance,
                ..Default::default()
            };
            let original = body
                .realize_once(Placement3::IDENTITY, &policy, policy.tolerance, |id, _| {
                    id.0
                })
                .unwrap();
            assert!(original.report.within_tolerance());
            let output = body.realize(Placement3::IDENTITY, &policy).unwrap();
            assert_same_geometry(&original, &output);
            assert_eq!(original.report, output.report);
            assert_eq!(output.report.work.attempts, 1);
            assert!(output.report.curve_refinement.is_none());
        }
    }
}
