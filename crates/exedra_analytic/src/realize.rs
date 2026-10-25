// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-independent realization of planar and rotational strip faces.
//!
//! Shared edges are sampled once. Planar trims are checked in emitted coordinates;
//! cylindrical and conical faces use paired boundary strips, so a generic planar
//! triangulation cannot introduce long chords across a curved face. Other
//! curved trims are explicit refusals. Tighten sampling when an emitted
//! arrangement is refused; this path does not heal or refine trim contacts.

use crate::body::{Body, BodyError, Coedge, EdgeId, Face, FaceId, ValidationPolicy, VertexId};
use crate::geometry::{Curve, GeometryError, Surface};
use alloc::vec::Vec;
use exedra_math::{Placement3, narrow};
use exedra_mesh::{BuildParams, Mesh};
use exedra_profile::{CircularEdgeConstraints, CircularSamplingError, circular_edge_count};
use exedra_triangulate::{PolygonInput, TriError, TriParams, triangulate_preserving_boundary};

/// Explicit accuracy and storage bounds for one body realization.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RealizationPolicy {
    /// Requested geometric deviation in model units, including emitted precision.
    pub tolerance: f64,
    /// Allowed native endpoint/support agreement, separate from tessellation accuracy.
    pub geometry_gap: f64,
    /// Minimum segments for a circular edge; at least four.
    pub min_arc_segments: u32,
    /// Maximum segments for any circular edge.
    pub max_edge_segments: u32,
    /// Maximum emitted vertices.
    pub max_vertices: u32,
    /// Maximum total edge-chain entries, including repeated shared endpoints.
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

/// Accuracy and work evidence for one explicit conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct RealizationReport {
    /// Requested deviation in model units.
    pub tolerance: f64,
    /// Largest conservative native endpoint/support gap from validation.
    pub geometry_gap: f64,
    /// Largest circular chord sagitta from actual sampled parameter intervals,
    /// before narrowing.
    pub max_curve_deviation: f64,
    /// Largest emitted vertex displacement, including placement and narrowing.
    pub max_vertex_displacement: f64,
    /// Largest emitted paired-sample residual from the ideal rotational ruling.
    /// Cylinder rulings translate; conical rulings also scale the reference radius.
    /// Includes parameter-grid mismatch and authoritative endpoint offsets.
    pub max_ruling_displacement: f64,
    /// Conservative geometric deviation bound: curve error, emitted and ruling
    /// displacement, and two native gaps (endpoint-to-curve and curve-to-support).
    pub max_deviation: f64,
    /// Segment counts indexed by native edge; zero for detached edges.
    pub edge_segments: Vec<u32>,
}
impl RealizationReport {
    /// Whether the emitted result meets the requested geometric deviation.
    #[must_use]
    pub fn within_tolerance(&self) -> bool {
        self.max_deviation <= self.tolerance
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
    /// A curved surface trim is not one supported four-edge strip.
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
    /// An explicit storage budget is exceeded.
    Budget {
        /// Budgeted storage unit.
        storage: &'static str,
        /// Required entries.
        required: usize,
        /// Allowed entries.
        maximum: u32,
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
struct Patch {
    a: Coedge,
    b: Coedge,
    translation: [f64; 3],
    center: [f64; 3],
    radius_scale: f64,
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
    ///
    /// # Errors
    /// Returns the same bounded refusals as [`Self::realize`].
    pub fn realize_with_regions(
        &self,
        placement: Placement3,
        policy: &RealizationPolicy,
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
        let mut patches = alloc::vec![None; self.faces().len()];
        let mut groups = Groups::new(self.edges().len());
        for (i, face) in self.faces().iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            for &loop_id in &face.loops {
                for coedge in &self.boundary_loop(loop_id).expect("validated loop").coedges {
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
            let patch = if matches!(face.surface, Surface::Plane(_)) {
                // Coaxial annuli/sectors retain angular rulings, which are
                // also the construction chart's nondegenerate coordinate net.
                self.rotational_patch(id, policy.geometry_gap).ok()
            } else {
                Some(self.rotational_patch(id, policy.geometry_gap)?)
            };
            if let Some(patch) = patch {
                groups.join(patch.a.edge.index(), patch.b.edge.index());
                patches[i] = Some(patch);
            }
        }
        let mut counts = Vec::with_capacity(self.edges().len());
        for (i, edge) in self.edges().iter().enumerate() {
            let n = if incidence[i].count == 0 {
                0
            } else {
                match &edge.curve {
                    Curve::Line(_) => 1,
                    Curve::Circle(circle) => circular_edge_count(
                        circle.radius(),
                        edge.range.span(),
                        policy.tolerance,
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
            };
            counts.push(n);
        }
        groups.synchronize(&mut counts);
        let mut active_vertices = alloc::vec![false;self.vertices().len()];
        let mut samples = 0_usize;
        let mut interior = 0_usize;
        for (edge, &n) in self.edges().iter().zip(&counts) {
            if n == 0 {
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
            .saturating_add(interior);
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
            displacement = displacement.max(distance(p.map(f64::from), exact));
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
            if n == 0 {
                continue;
            }
            let id = EdgeId(crate::usize_to_u32(i));
            chains
                .vertices
                .push(vertex_map[edge.start.index()].expect("active vertex"));
            let [min, max] = edge.range.bounds();
            let key = (min.to_bits(), max.to_bits(), n);
            let reuse_phases = phase_key == Some(key);
            if matches!(edge.curve, Curve::Circle(_)) && !reuse_phases {
                phases.clear();
                phases.reserve(n as usize - 1);
                phase_key = Some(key);
            }
            let mut previous_parameter = min;
            let mut max_parameter_step = 0.0_f64;
            for k in 1..n {
                let t = f64::from(k) / f64::from(n);
                let parameter = edge.range.at(t).expect("bounded sample fraction");
                max_parameter_step = max_parameter_step.max((parameter - previous_parameter).abs());
                previous_parameter = parameter;
                let point = match &edge.curve {
                    Curve::Circle(circle) => {
                        let [sin, cos] = if reuse_phases {
                            phases[k as usize - 1]
                        } else {
                            let phase = [libm::sin(parameter), libm::cos(parameter)];
                            phases.push(phase);
                            phase
                        };
                        let r = circle.radius();
                        circle.frame().transform_point([r * cos, r * sin, 0.0])
                    }
                    Curve::Line(_) => {
                        edge.curve
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
                displacement = displacement.max(distance(p.map(f64::from), exact));
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
            if let Curve::Circle(circle) = &edge.curve {
                // Absolute parameter origins can make nominally equal steps
                // round differently. Bound the intervals actually evaluated.
                max_parameter_step = max_parameter_step.max((max - previous_parameter).abs());
                let sin = libm::sin(max_parameter_step / 4.0);
                curve_deviation = curve_deviation.max(2.0 * circle.radius() * sin * sin);
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
        // Counts and trims already determine output cardinality. Reserve the
        // three parallel arrays once instead of repeatedly growing each while
        // emitting strips. A planar polygon contributes N + 2H - 2 triangles.
        let triangle_count = self
            .faces()
            .iter()
            .enumerate()
            .map(|(i, face)| {
                patches[i].map_or_else(
                    || {
                        face.loops
                            .iter()
                            .flat_map(|id| {
                                &self.boundary_loop(*id).expect("validated loop").coedges
                            })
                            .map(|coedge| counts[coedge.edge.index()] as usize)
                            .fold(0_usize, usize::saturating_add)
                            .saturating_add(face.loops.len().saturating_sub(1).saturating_mul(2))
                            .saturating_sub(2)
                    },
                    |patch| (counts[patch.a.edge.index()] as usize).saturating_mul(2),
                )
            })
            .try_fold(0_usize, |total, count| {
                total
                    .checked_add(count)
                    .filter(|&n| n <= u32::MAX as usize)
                    .ok_or(RealizationError::Budget {
                        storage: "triangles",
                        required: total.saturating_add(count),
                        maximum: u32::MAX,
                    })
            })?;
        let mut triangles = Vec::with_capacity(triangle_count);
        let mut regions = Vec::with_capacity(triangle_count);
        let mut native_faces = Vec::with_capacity(triangle_count);
        let mut ruling_displacement = 0.0_f64;
        for (i, face) in self.faces().iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            let face_region = region(id, face);
            if patches[i].is_none() {
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
                    triangulate_preserving_boundary(&input, &TriParams::default())
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
                let patch = patches[i].expect("prepared patch");
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
                for j in 0..a.len() {
                    let lower = emitted[a_at(j) as usize].map(f64::from);
                    let upper = emitted[b_at(j) as usize].map(f64::from);
                    let radial_change =
                        exedra_math::sub(lower, center).map(|v| v * (patch.radius_scale - 1.0));
                    let expected_delta = exedra_math::add(translation, radial_change);
                    let residual = distance(exedra_math::sub(upper, lower), expected_delta);
                    if !residual.is_finite() {
                        return Err(RealizationError::DegenerateFace { face: id });
                    }
                    ruling_displacement = ruling_displacement.max(residual);
                }
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
        let edge_updates: Vec<_> = mesh
            .half_edges()
            .filter_map(|edge| {
                if mesh.canonical_edge(edge) != Some(edge) {
                    return None;
                }
                let a = mesh.from_vertex(edge)?.index();
                let b = mesh.to_vertex(edge)?.index();
                sharp
                    .binary_search_by_key(&ordered(a, b), |&(a, b, _, _)| (a, b))
                    .ok()
                    .map(|i| (edge, sharp[i].2))
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
        let _: () = edit.finish();
        let edge_chains = EdgeChains {
            vertices: chains
                .vertices
                .into_iter()
                .map(|i| vertex_ids[i as usize])
                .collect(),
            offsets: chains.offsets,
        };
        Ok(RealizedBody {
            mesh,
            mesh_faces: face_ids.into_iter().zip(native_faces).collect(),
            mesh_vertices: vertex_ids.into_iter().zip(sources).collect(),
            edge_chains,
            report: RealizationReport {
                tolerance: policy.tolerance,
                geometry_gap: validation.max_gap,
                max_curve_deviation: curve_deviation,
                max_vertex_displacement: displacement,
                max_ruling_displacement: ruling_displacement,
                max_deviation: curve_deviation
                    + displacement
                    + ruling_displacement
                    + 2.0 * validation.max_gap,
                edge_segments: counts,
            },
        })
    }

    fn rotational_patch(&self, id: FaceId, gap: f64) -> Result<Patch, RealizationError> {
        let face = self.face(id).expect("known face");
        let refuse = || RealizationError::UnsupportedSurfaceTrim { face: id };
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
                !matches!(edge.curve, Curve::Circle(_)) || edge.start != edge.end
            }) {
                return Err(refuse());
            }
            let circle = |coedge: Coedge| {
                let Curve::Circle(circle) = &self.edges()[coedge.edge.index()].curve else {
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
                    (self.edges()[c.edge.index()].range.span() - core::f64::consts::TAU).abs()
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
            if coedges.len() != 4 {
                return Err(refuse());
            }
            let first = coedges
                .iter()
                .position(|c| matches!(self.edges()[c.edge.index()].curve, Curve::Circle(_)))
                .ok_or_else(refuse)?;
            let (a, b) = (coedges[first], coedges[(first + 2) % 4]);
            if !matches!(self.edges()[b.edge.index()].curve, Curve::Circle(_))
                || [1, 3].into_iter().any(|j| {
                    !matches!(
                        self.edges()[coedges[(first + j) % 4].edge.index()].curve,
                        Curve::Line(_)
                    )
                })
            {
                return Err(refuse());
            }
            (a, b)
        };
        let axis = face.surface.frame().rows.map(|r| r[2]);
        let axis_length = distance(axis, [0.0; 3]);
        let axis = axis.map(|v| v / axis_length);
        let signed = |coedge: Coedge| {
            let edge = &self.edges()[coedge.edge.index()];
            let Curve::Circle(circle) = &edge.curve else {
                unreachable!();
            };
            exedra_math::dot(circle.frame().rows.map(|r| r[2]), axis).signum()
                * edge.range.span()
                * if coedge.reversed { -1.0 } else { 1.0 }
        };
        let circle = |coedge: Coedge| {
            let Curve::Circle(circle) = &self.edges()[coedge.edge.index()].curve else {
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
        Ok(Patch {
            a,
            b,
            translation,
            center: lower.frame().rows.map(|r| r[3]),
            radius_scale,
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
        let normal = |id| {
            self.face(id)
                .and_then(|f| f.surface.parameters(point))
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
            curve: Curve::line(
                vertices[from],
                exedra_math::sub(vertices[to], vertices[from]),
            )
            .unwrap(),
            range: ParameterRange::UNIT,
            start: VertexId(u32::try_from(from).unwrap()),
            end: VertexId(u32::try_from(to).unwrap()),
            role: (),
        };
        let edges = vec![
            Edge {
                curve: lower,
                range: a,
                start: VertexId(0),
                end: VertexId(1),
                role: (),
            },
            Edge {
                curve: upper,
                range: b,
                start: VertexId(2),
                end: VertexId(3),
                role: (),
            },
            line(1, 3),
            line(0, 2),
        ];
        Body::try_from_parts(
            BodyParts {
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
        parts.edges[1].curve = Curve::circle(Placement3::IDENTITY, 1.0).unwrap();
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
            parts.edges[1].curve = Curve::circle(
                Placement3::rotate_z_then_translate(-phase, 0.0, 0.0, 3.0),
                2.0,
            )
            .unwrap();
            parts.edges[1].range =
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
                    VertexSource::Vertex(id) => body.vertices()[id.index()],
                    VertexSource::Edge { edge, parameter } => {
                        let curve = &body.edges()[edge.index()];
                        curve
                            .curve
                            .evaluate(curve.range.at(parameter).unwrap())
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
            let result = body
                .realize(
                    placement,
                    &RealizationPolicy {
                        tolerance: 0.011,
                        ..RealizationPolicy::default()
                    },
                )
                .unwrap();
            assert!(!result.report.within_tolerance());
            assert!(result.report.max_curve_deviation >= 0.0175);
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
        parts.edges[0].curve = Curve::circle(
            Placement3::rotate_z_then_translate(1e15, 0.0, 0.0, 0.0),
            1.0,
        )
        .unwrap();
        parts.edges[0].range = ParameterRange::new(0.0, 1.125).unwrap();
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
}
