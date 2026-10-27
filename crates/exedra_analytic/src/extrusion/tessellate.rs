// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mesh realization of an [`Extrusion`] from shared edge samples.
//!
//! Each profile segment's sample count is chosen once. Both of its profile
//! edges, both caps and its wall consume the same samples and mesh vertices,
//! so the mesh is closed by construction rather than by welding. After the
//! samples are placed and narrowed to `f32`, each cap's boundary is
//! triangulated in the emitted coordinates, which checks that the sampled
//! arrangement survived: no crossing or touching boundaries, holes inside the
//! outer loop and apart from each other. A coarse chord that cuts across a
//! nearby boundary is refined, within the policy's budget; anything else is a
//! typed refusal.

use alloc::vec::Vec;

use exedra_math::{Placement3, narrow};
use exedra_mesh::{FaceBuildAttrs, Mesh, MeshBuilder};
use exedra_profile::{CircularEdgeConstraints, CircularSamplingError, Curve, circular_edge_count};
use exedra_triangulate::{PolygonInput, TriError, TriParams, triangulate_preserving_boundary};

use super::{Boundary, BoundarySegment, EdgeId, End, Extrusion, Face, FaceId, Level, VertexId};

/// Accuracy and work limits for [`Extrusion::tessellate`].
///
/// Caps and pocket floors use constrained Delaunay triangulation while keeping
/// their shared boundary samples fixed. This improves diagonal choices without
/// adding interior vertices or promising a minimum triangle angle. Curve
/// accuracy is controlled independently by [`Self::chord_tolerance`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TessellationPolicy {
    /// Largest chord-to-arc deviation in local model units, before `f32`
    /// narrowing. [`TessellationReport::max_deviation`] measures the result.
    pub chord_tolerance: f64,
    /// Fewest edges per arc segment.
    pub min_arc_edges: u32,
    /// Most edges per profile segment.
    pub max_segment_edges: u32,
    /// Most mesh vertices.
    pub max_vertices: u32,
    /// Largest turn, in radians, between adjacent segments' tangents at a
    /// profile vertex for its seam to be smooth rather than sharp.
    pub smooth_seam_angle: f64,
    /// Times a cap boundary contact may refine the arcs involved before the
    /// realization is refused.
    pub max_refinements: u32,
}

impl Default for TessellationPolicy {
    fn default() -> Self {
        Self {
            chord_tolerance: 0.01,
            min_arc_edges: 4,
            max_segment_edges: 4096,
            max_vertices: 1 << 20,
            smooth_seam_angle: 1e-6,
            max_refinements: 8,
        }
    }
}

pub use crate::realize::VertexSource;

/// Evidence about one realization.
#[derive(Clone, Debug, PartialEq)]
pub struct TessellationReport {
    /// The requested chord tolerance.
    pub chord_tolerance: f64,
    /// Bound on the distance from an emitted boundary chord to its native
    /// curve, measured on the emitted `f32` coordinates in local units: the
    /// in-plane chord deviation and the endpoints' axial error combined.
    /// Includes narrowing.
    pub max_deviation: f64,
    /// Largest distance from an emitted vertex to its native position, in
    /// local units: the narrowing error alone.
    pub max_vertex_displacement: f64,
    /// Edges used for each profile segment, outer loop first.
    pub segment_edges: Vec<u32>,
    /// Refinements caused by cap boundary contacts.
    pub refinements: u32,
}

impl TessellationReport {
    /// Whether the emitted mesh meets the requested chord tolerance.
    #[must_use]
    pub fn within_tolerance(&self) -> bool {
        self.max_deviation <= self.chord_tolerance
    }
}

/// A mesh realization with its correspondence to the native solid.
#[derive(Clone, Debug)]
pub struct TessellatedExtrusion {
    /// The closed triangle-and-quad mesh in placed coordinates. Face regions
    /// are native face indices, or the caller's mapping from
    /// [`Extrusion::tessellate_with_regions`]; rims and nonsmooth seams are
    /// sharp.
    pub mesh: Mesh,
    /// The native face of each mesh face, in mesh face order.
    pub mesh_faces: Vec<(exedra_mesh::FaceId, FaceId)>,
    /// The native source of each mesh vertex.
    pub mesh_vertices: Vec<(exedra_mesh::VertexId, VertexSource)>,
    /// For each native edge, its mesh vertices from start to end.
    pub edge_chains: Vec<Vec<exedra_mesh::VertexId>>,
    /// Accuracy and work evidence.
    pub report: TessellationReport,
}

/// Why an extrusion could not be realized.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TessellateExtrusionError {
    /// A tolerance, budget or angle in the policy is invalid.
    InvalidPolicy,
    /// An arc cannot meet the tolerance within the per-segment budget.
    Sampling {
        /// The arc's boundary segment.
        segment: BoundarySegment,
        /// The sampling failure.
        error: CircularSamplingError,
    },
    /// The realization needs more vertices than the budget allows.
    VertexBudget {
        /// Vertices needed.
        required: usize,
        /// The budget.
        maximum: u32,
    },
    /// Two consecutive samples of a segment, or the ends of a seam, narrow to
    /// the same `f32` point.
    Collapsed {
        /// The segment whose samples coincide.
        segment: BoundarySegment,
    },
    /// A planar face's emitted boundary is not a valid arrangement, and
    /// refinement could not or did not repair it.
    Arrangement {
        /// The refused cap or floor.
        face: FaceId,
        /// The triangulator's diagnosis on emitted coordinates.
        error: TriError,
    },
    /// After narrowing, a pocket's floor meets or passes the opposite cap
    /// (`None`) or the floor of an overlapping pocket on the other end: the
    /// material between them would vanish or turn inside out.
    Web {
        /// The pocket.
        pocket: usize,
        /// The overlapping pocket on the other end, or `None` for the cap.
        opposite: Option<usize>,
    },
    /// The mesh builder refused the faces.
    Build(exedra_mesh::BuildError),
}

impl core::fmt::Display for TessellateExtrusionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPolicy => write!(f, "tessellation policy is invalid"),
            Self::Sampling { segment, error } => {
                write!(
                    f,
                    "segment {} of {:?}: {error}",
                    segment.segment, segment.boundary
                )
            }
            Self::VertexBudget { required, maximum } => {
                write!(f, "{required} vertices exceed the budget of {maximum}")
            }
            Self::Collapsed { segment } => write!(
                f,
                "segment {} of {:?} collapses in f32",
                segment.segment, segment.boundary
            ),
            Self::Arrangement { face, error } => {
                write!(f, "face {} boundary: {error}", face.0)
            }
            Self::Web { pocket, opposite } => match opposite {
                Some(other) => write!(f, "pocket {pocket} floor meets pocket {other} in f32"),
                None => write!(f, "pocket {pocket} floor meets the opposite cap in f32"),
            },
            Self::Build(error) => write!(f, "mesh build: {error}"),
        }
    }
}

impl core::error::Error for TessellateExtrusionError {}

/// One boundary's samples, in segment order: segment `k` owns ring points
/// `starts[k] .. starts[k + 1]`, the first of which is its start vertex.
struct Ring {
    points: Vec<[f64; 2]>,
    starts: Vec<usize>,
}

impl Ring {
    fn segment_of(&self, point: usize) -> usize {
        self.starts.partition_point(|&s| s <= point) - 1
    }
}

impl Extrusion {
    /// Realizes the solid as a mesh under `policy`, with each mesh face's
    /// `FACE_REGION` set to its native face index.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an invalid policy, an arc or vertex budget
    /// that cannot be met, samples that coincide in `f32`, or a cap or floor
    /// boundary that is not a valid arrangement in the emitted coordinates.
    pub fn tessellate(
        &self,
        policy: &TessellationPolicy,
    ) -> Result<TessellatedExtrusion, TessellateExtrusionError> {
        self.tessellate_with_regions(policy, |face, _| face.0)
    }

    /// Realizes the solid like [`Self::tessellate`], with each mesh face's
    /// `FACE_REGION` set by `region` from its native face.
    ///
    /// Several native faces can share a region, so a material bound to one
    /// region covers a whole feature: for example every wall whose segment
    /// carries a given tag. [`TessellatedExtrusion::mesh_faces`] still maps
    /// each mesh face to its own native face.
    ///
    /// ```
    /// use exedra_analytic::extrusion::{Extrusion, FaceRole, TessellationPolicy};
    /// use exedra_math::Placement3;
    /// use exedra_profile::{Loop, Profile, ProfilePolicy, Segment, SegTag};
    ///
    /// const ROUND: SegTag = SegTag(1);
    /// let outline = Loop::new(vec![
    ///     Segment::line([2.0, 0.0]),
    ///     Segment::arc([2.0, 2.0], 1.0).tagged(ROUND),
    ///     Segment::line([0.0, 2.0]),
    ///     Segment::line([0.0, 0.0]),
    /// ])?;
    /// let profile = Profile::new(outline, vec![], &ProfilePolicy::new(1e-9))?;
    /// let solid = Extrusion::new(profile, Placement3::IDENTITY, 1.0)?;
    /// let realized = solid.tessellate_with_regions(&TessellationPolicy::default(), |_, face| {
    ///     match face.role {
    ///         FaceRole::Wall { tag: Some(ROUND), .. } => 1,
    ///         _ => 0,
    ///     }
    /// })?;
    /// let regions = realized.mesh.attrs().dense(exedra_mesh::attr::FACE_REGION).unwrap();
    /// let rounded = realized
    ///     .mesh_faces
    ///     .iter()
    ///     .filter(|(mesh_face, _)| regions.get(mesh_face.as_id()) == Some(&1))
    ///     .count();
    /// assert!(rounded > 1, "the round wall is several mesh faces in one region");
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Same as [`Self::tessellate`].
    pub fn tessellate_with_regions(
        &self,
        policy: &TessellationPolicy,
        region: impl Fn(FaceId, &Face) -> u32,
    ) -> Result<TessellatedExtrusion, TessellateExtrusionError> {
        if !(policy.chord_tolerance.is_finite() && policy.chord_tolerance > 0.0)
            || !(policy.smooth_seam_angle.is_finite() && policy.smooth_seam_angle >= 0.0)
            || policy.min_arc_edges == 0
            || policy.min_arc_edges > policy.max_segment_edges
        {
            return Err(TessellateExtrusionError::InvalidPolicy);
        }
        let mut counts: Vec<Vec<u32>> = Vec::with_capacity(self.bands().len());
        for band in self.bands() {
            let loop_ = self.boundary_loop(band.boundary);
            let mut band_counts = Vec::with_capacity(loop_.segments().len());
            for (segment, s) in loop_.segments().iter().enumerate() {
                let count = match s.curve {
                    Curve::Line => 1,
                    Curve::Arc { .. } | Curve::ArcCenter { .. } => {
                        let arc = loop_.arc(segment).expect("validated arc");
                        circular_edge_count(
                            arc.radius,
                            arc.sweep.abs(),
                            policy.chord_tolerance,
                            CircularEdgeConstraints::new(
                                policy.min_arc_edges,
                                policy.max_segment_edges,
                            ),
                        )
                        .map_err(|error| {
                            TessellateExtrusionError::Sampling {
                                segment: BoundarySegment {
                                    boundary: band.boundary,
                                    segment,
                                },
                                error,
                            }
                        })?
                    }
                };
                band_counts.push(count);
            }
            counts.push(band_counts);
        }
        let mut refinements = 0;
        loop {
            let rings = self.sample(&counts);
            let required = 2 * rings.iter().map(|ring| ring.points.len()).sum::<usize>();
            if required > policy.max_vertices as usize {
                return Err(TessellateExtrusionError::VertexBudget {
                    required,
                    maximum: policy.max_vertices,
                });
            }
            match self.realize(&rings, policy, &region) {
                Ok(mut realized) => {
                    realized.report.segment_edges = counts.concat();
                    realized.report.refinements = refinements;
                    return Ok(realized);
                }
                Err(Refusal::Fatal(error)) => return Err(error),
                Err(Refusal::Contact {
                    face,
                    error,
                    segments,
                }) => {
                    // Refine the arcs involved: a chord that cuts across a
                    // nearby boundary moves toward its arc.
                    let mut refined = false;
                    for (band, segment) in segments {
                        let loop_ = self.boundary_loop(self.bands()[band].boundary);
                        let count = &mut counts[band][segment];
                        let is_arc = matches!(
                            loop_.segments()[segment].curve,
                            Curve::Arc { .. } | Curve::ArcCenter { .. }
                        );
                        if is_arc && *count < policy.max_segment_edges {
                            *count = count.saturating_mul(2).min(policy.max_segment_edges);
                            refined = true;
                        }
                    }
                    if !refined || refinements >= policy.max_refinements {
                        return Err(TessellateExtrusionError::Arrangement { face, error });
                    }
                    refinements += 1;
                }
            }
        }
    }

    fn sample(&self, counts: &[Vec<u32>]) -> Vec<Ring> {
        self.bands()
            .iter()
            .zip(counts)
            .map(|(band, band_counts)| {
                let loop_ = self.boundary_loop(band.boundary);
                let mut points = Vec::new();
                let mut starts = Vec::with_capacity(band_counts.len() + 1);
                for (segment, ((from, s), &count)) in
                    loop_.iter_with_starts().zip(band_counts).enumerate()
                {
                    starts.push(points.len());
                    points.push(from);
                    if let Curve::Arc { .. } | Curve::ArcCenter { .. } = s.curve {
                        let arc = loop_.arc(segment).expect("validated arc");
                        points.extend(
                            (1..count).map(|i| arc.point_at(f64::from(i) / f64::from(count))),
                        );
                    } else {
                        points.extend((1..count).map(|i| {
                            let t = f64::from(i) / f64::from(count);
                            [
                                from[0] + t * (s.to[0] - from[0]),
                                from[1] + t * (s.to[1] - from[1]),
                            ]
                        }));
                    }
                }
                starts.push(points.len());
                Ring { points, starts }
            })
            .collect()
    }

    #[expect(clippy::too_many_lines, reason = "one pass over shared samples")]
    fn realize(
        &self,
        rings: &[Ring],
        policy: &TessellationPolicy,
        region: &dyn Fn(FaceId, &Face) -> u32,
    ) -> Result<TessellatedExtrusion, Refusal> {
        let bands = self.bands();
        let region_of = |face: FaceId| region(face, &self.body.faces()[face.index()]);
        // Each band has a vertex block per level: its ring at that height.
        let mut blocks = Vec::with_capacity(bands.len());
        let mut emitted = Vec::new();
        let mut local = Vec::new();
        let mut max_vertex_displacement = 0.0_f64;
        for (band, ring) in bands.iter().zip(rings) {
            let mut band_blocks = [0; 2];
            for (slot, z) in band.heights.into_iter().enumerate() {
                band_blocks[slot] = emitted.len();
                for point in &ring.points {
                    let exact = [point[0], point[1], z];
                    let placed = narrow(self.frame.transform_point(exact));
                    let back = to_local(&self.frame, placed);
                    max_vertex_displacement = max_vertex_displacement
                        .max(exedra_math::norm(exedra_math::sub(back, exact)));
                    emitted.push(placed);
                    local.push(back);
                }
            }
            blocks.push(band_blocks);
        }
        // Coincident consecutive samples or seam ends cannot form edges.
        for ((band, ring), block) in bands.iter().zip(rings).zip(&blocks) {
            let n = ring.points.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let collapsed = block
                    .iter()
                    .any(|&base| emitted[base + i] == emitted[base + j])
                    || emitted[block[0] + i] == emitted[block[1] + i];
                if collapsed {
                    return Err(Refusal::Fatal(TessellateExtrusionError::Collapsed {
                        segment: BoundarySegment {
                            boundary: band.boundary,
                            segment: ring.segment_of(i),
                        },
                    }));
                }
            }
        }
        self.check_webs(&blocks, rings, &local)?;
        let mut builder = MeshBuilder::new();
        for position in &emitted {
            let _ = builder.push_vertex(*position);
        }
        let index = |i: usize| crate::usize_to_u32(i);
        let build = |error| Refusal::Fatal(TessellateExtrusionError::Build(error));
        let mut native_faces = Vec::new();
        // Caps and floors, triangulated in emitted coordinates. A floor's
        // stored ring winds clockwise, so it is read backwards as an outer
        // boundary.
        let mut planar = Vec::with_capacity(self.planes().len());
        for plane in self.planes() {
            let reversed = |ring: usize| plane.region_right && ring == 0;
            let mut points: Vec<Vec<[f64; 2]>> = Vec::with_capacity(plane.rings.len());
            let mut vertices: Vec<usize> = Vec::new();
            for (ring_index, &(band, slot)) in plane.rings.iter().enumerate() {
                let n = rings[band].points.len();
                let order: Vec<usize> = if reversed(ring_index) {
                    (0..n).rev().collect()
                } else {
                    (0..n).collect()
                };
                points.push(
                    order
                        .iter()
                        .map(|&i| {
                            let p = local[blocks[band][slot] + i];
                            [p[0], p[1]]
                        })
                        .collect(),
                );
                vertices.extend(order.iter().map(|&i| blocks[band][slot] + i));
            }
            let holes: Vec<&[[f64; 2]]> = points[1..].iter().map(Vec::as_slice).collect();
            let input = PolygonInput {
                outer: &points[0],
                holes: &holes,
            };
            match triangulate_preserving_boundary(&input, &TriParams::constrained_delaunay()) {
                Ok(triangulation) => planar.push((plane, vertices, triangulation.triangles)),
                Err(error) => {
                    let witness = |ring: usize, edge: usize| {
                        let (band, _) = plane.rings[ring];
                        let n = rings[band].points.len();
                        // Edge `edge` of a reversed ring joins original
                        // points `n - 1 - edge` and `n - 2 - edge`.
                        let first = if reversed(ring) {
                            (2 * n - 2 - edge) % n
                        } else {
                            edge
                        };
                        (band, rings[band].segment_of(first))
                    };
                    let segments = match error {
                        TriError::BoundaryContact { first, second, .. } => [first, second]
                            .into_iter()
                            .map(|edge| witness(edge.hole.map_or(0, |hole| hole + 1), edge.edge))
                            .collect(),
                        // Chords that cut under a hole leave it outside:
                        // refine the outer boundary and that hole.
                        TriError::HoleOutsideOuter { hole } => [0, hole + 1]
                            .into_iter()
                            .flat_map(|ring| {
                                let band = plane.rings[ring].0;
                                (0..rings[band].starts.len() - 1)
                                    .map(move |segment| (band, segment))
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    return Err(Refusal::Contact {
                        face: FaceId(crate::usize_to_u32(plane.face)),
                        error,
                        segments,
                    });
                }
            }
        }
        // Planar faces wind counter-clockwise seen from +Z; downward ones
        // reverse.
        for (plane, vertices, triangles) in &planar {
            let face = FaceId(crate::usize_to_u32(plane.face));
            for [a, b, c] in triangles.iter().map(|t| t.map(|v| vertices[v as usize])) {
                let corners = if plane.upward {
                    [index(a), index(b), index(c)]
                } else {
                    [index(a), index(c), index(b)]
                };
                builder
                    .add_face_with_attrs(
                        &corners,
                        &FaceBuildAttrs {
                            region: Some(region_of(face)),
                            edge_seams: None,
                            edge_sharpness: None,
                        },
                    )
                    .map_err(build)?;
                native_faces.push(face);
            }
        }
        // Walls: quads [lower i, lower j, upper j, upper i] per sample
        // interval, whose edges are the lower rim, the lateral at j, the upper
        // rim and the lateral at i.
        for ((band, ring), block) in bands.iter().zip(rings).zip(&blocks) {
            let loop_ = self.boundary_loop(band.boundary);
            let n = ring.points.len();
            let segments = ring.starts.len() - 1;
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                let previous = (segment + segments - 1) % segments;
                let sharp_end = !smooth_at(loop_, segment, next, policy.smooth_seam_angle);
                let sharp_start = !smooth_at(loop_, previous, segment, policy.smooth_seam_angle);
                let face = FaceId(crate::usize_to_u32(band.walls + segment));
                let (first, last) = (ring.starts[segment], ring.starts[segment + 1]);
                for i in first..last {
                    let j = (i + 1) % n;
                    let lateral =
                        |at_seam: bool, sharp: bool| f32::from(u8::from(at_seam && sharp));
                    let sharpness = [
                        1.0,
                        lateral(i + 1 == last, sharp_end),
                        1.0,
                        lateral(i == first, sharp_start),
                    ];
                    builder
                        .add_face_with_attrs(
                            &[
                                index(block[0] + i),
                                index(block[0] + j),
                                index(block[1] + j),
                                index(block[1] + i),
                            ],
                            &FaceBuildAttrs {
                                region: Some(region_of(face)),
                                edge_seams: None,
                                edge_sharpness: Some(&sharpness),
                            },
                        )
                        .map_err(build)?;
                    native_faces.push(face);
                }
            }
        }
        let built = builder.build().map_err(build)?;
        // Correspondence: every sample is a native vertex or an interior
        // point of the boundary edge at its level.
        let mut sources = Vec::with_capacity(emitted.len());
        let mut edge_chains = alloc::vec![Vec::new(); self.body.edges().len()];
        for ((band, ring), block) in bands.iter().zip(rings).zip(&blocks) {
            let n = ring.points.len();
            let segments = ring.starts.len() - 1;
            for slot in 0..2 {
                for segment in 0..segments {
                    let count = ring.starts[segment + 1] - ring.starts[segment];
                    for i in 0..count {
                        sources.push(if i == 0 {
                            let previous = (segment + segments - 1) % segments;
                            VertexSource::Vertex(VertexId(crate::usize_to_u32(
                                band.vertices[slot] + previous,
                            )))
                        } else {
                            VertexSource::Edge {
                                edge: EdgeId(crate::usize_to_u32(band.edges[slot] + segment)),
                                parameter: f64_ratio(i, count),
                            }
                        });
                    }
                    edge_chains[band.edges[slot] + segment] = (ring.starts[segment]
                        ..=ring.starts[segment + 1])
                        .map(|i| built.vertex_ids[block[slot] + i % n])
                        .collect();
                }
            }
            for segment in 0..segments {
                let at = ring.starts[segment + 1] % n;
                edge_chains[band.seams + segment] = alloc::vec![
                    built.vertex_ids[block[0] + at],
                    built.vertex_ids[block[1] + at]
                ];
            }
        }
        // Evidence on emitted coordinates.
        let mut max_deviation = max_vertex_displacement;
        for ((band, ring), block) in bands.iter().zip(rings).zip(&blocks) {
            let loop_ = self.boundary_loop(band.boundary);
            let n = ring.points.len();
            for segment in 0..ring.starts.len() - 1 {
                let Some(arc) = loop_.arc(segment) else {
                    continue;
                };
                for (base, height) in block.iter().zip(band.heights) {
                    for i in ring.starts[segment]..ring.starts[segment + 1] {
                        let (a, b) = (local[base + i], local[base + (i + 1) % n]);
                        // Height varies linearly along the chord, so its
                        // error is bounded by the endpoints'; the in-plane
                        // and axial errors combine.
                        let planar =
                            chord_deviation([a[0], a[1]], [b[0], b[1]], arc.center, arc.radius);
                        let axial = (a[2] - height).abs().max((b[2] - height).abs());
                        max_deviation = max_deviation.max(libm::hypot(planar, axial));
                    }
                }
            }
        }
        Ok(TessellatedExtrusion {
            mesh_faces: built.face_ids.iter().copied().zip(native_faces).collect(),
            mesh_vertices: built.vertex_ids.iter().copied().zip(sources).collect(),
            edge_chains,
            mesh: built.mesh,
            report: TessellationReport {
                chord_tolerance: policy.chord_tolerance,
                max_deviation,
                max_vertex_displacement,
                segment_edges: Vec::new(),
                refinements: 0,
            },
        })
    }
}

impl Extrusion {
    /// Refuses floors that meet or pass the opposite cap, or the floor of an
    /// overlapping pocket on the other end, in emitted coordinates. Each ring
    /// is compared by its whole height range, which also covers the small
    /// tilt narrowing gives a placed plane.
    fn check_webs(
        &self,
        blocks: &[[usize; 2]],
        rings: &[Ring],
        local: &[[f64; 3]],
    ) -> Result<(), Refusal> {
        let bands = self.bands();
        let heights = |band: usize, slot: usize| {
            let base = blocks[band][slot];
            (0..rings[band].points.len())
                .map(|i| local[base + i][2])
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                    (lo.min(z), hi.max(z))
                })
        };
        let pocket_band = |pocket: usize| {
            bands
                .iter()
                .position(|band| band.boundary == Boundary::Pocket(pocket))
                .expect("every pocket has a band")
        };
        // Floors are the pocket band's lower level at the end, upper at the
        // start; caps are every band's ring at that level.
        let floor = |pocket: usize| {
            let slot = usize::from(self.pockets()[pocket].end == End::Start);
            heights(pocket_band(pocket), slot)
        };
        let cap = |level: Level| {
            bands
                .iter()
                .enumerate()
                .flat_map(|(band, b)| {
                    b.levels
                        .iter()
                        .enumerate()
                        .map(move |(slot, l)| (band, slot, *l))
                })
                .filter(|&(_, _, l)| l == level)
                .map(|(band, slot, _)| heights(band, slot))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (a, b)| {
                    (lo.min(a), hi.max(b))
                })
        };
        let (start_cap, end_cap) = (cap(Level::Start), cap(Level::End));
        let refuse =
            |pocket, opposite| Refusal::Fatal(TessellateExtrusionError::Web { pocket, opposite });
        for (pocket, p) in self.pockets().iter().enumerate() {
            let (low, high) = floor(pocket);
            let clear = match p.end {
                End::End => low > start_cap.1,
                End::Start => high < end_cap.0,
            };
            if !clear {
                return Err(refuse(pocket, None));
            }
        }
        for &[end_pocket, start_pocket] in self.webs() {
            if floor(end_pocket).0 <= floor(start_pocket).1 {
                return Err(refuse(end_pocket, Some(start_pocket)));
            }
        }
        Ok(())
    }
}

/// Why one realization attempt stopped.
enum Refusal {
    Fatal(TessellateExtrusionError),
    Contact {
        face: FaceId,
        error: TriError,
        segments: Vec<(usize, usize)>,
    },
}

/// Whether the profile turns by at most `angle` from `segment` into `next`.
fn smooth_at(loop_: &exedra_profile::Loop, segment: usize, next: usize, angle: f64) -> bool {
    let leaving = end_tangent(loop_, segment, true);
    let entering = end_tangent(loop_, next, false);
    let turn = libm::atan2(
        leaving[0] * entering[1] - leaving[1] * entering[0],
        leaving[0] * entering[0] + leaving[1] * entering[1],
    );
    turn.abs() <= angle
}

/// Unit tangent of segment `index` at its end (`at_end`) or start.
fn end_tangent(loop_: &exedra_profile::Loop, index: usize, at_end: bool) -> [f64; 2] {
    let segments = loop_.segments();
    let from = segments[(index + segments.len() - 1) % segments.len()].to;
    let to = segments[index].to;
    let direction = match segments[index].curve {
        Curve::Arc { .. } | Curve::ArcCenter { .. } => {
            let arc = loop_.arc(index).expect("validated arc");
            let point = if at_end { to } else { from };
            let radial = [point[0] - arc.center[0], point[1] - arc.center[1]];
            // Counter-clockwise travel turns the radius a quarter turn left.
            let sense = arc.sweep.signum();
            [-radial[1] * sense, radial[0] * sense]
        }
        Curve::Line => [to[0] - from[0], to[1] - from[1]],
    };
    let length = libm::hypot(direction[0], direction[1]);
    [direction[0] / length, direction[1] / length]
}

/// Largest distance between the chord `a -> b` and the circle it samples.
fn chord_deviation(a: [f64; 2], b: [f64; 2], center: [f64; 2], radius: f64) -> f64 {
    let (ra, rb) = (
        [a[0] - center[0], a[1] - center[1]],
        [b[0] - center[0], b[1] - center[1]],
    );
    let delta = [rb[0] - ra[0], rb[1] - ra[1]];
    let span = libm::hypot(delta[0], delta[1]);
    let t = (-(ra[0] * delta[0] + ra[1] * delta[1]) / (span * span)).clamp(0.0, 1.0);
    let closest = [ra[0] + t * delta[0], ra[1] + t * delta[1]];
    (libm::hypot(ra[0], ra[1]) - radius)
        .abs()
        .max((libm::hypot(rb[0], rb[1]) - radius).abs())
        .max(radius - libm::hypot(closest[0], closest[1]))
}

/// Maps an emitted point back to local coordinates through the rigid frame.
fn to_local(frame: &Placement3, point: [f32; 3]) -> [f64; 3] {
    let r = &frame.rows;
    let d = [
        f64::from(point[0]) - r[0][3],
        f64::from(point[1]) - r[1][3],
        f64::from(point[2]) - r[2][3],
    ];
    [0, 1, 2].map(|axis| r[0][axis] * d[0] + r[1][axis] * d[1] + r[2][axis] * d[2])
}

fn f64_ratio(numerator: usize, denominator: usize) -> f64 {
    f64::from(crate::usize_to_u32(numerator)) / f64::from(crate::usize_to_u32(denominator))
}

#[cfg(test)]
mod tests;
