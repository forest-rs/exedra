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
use exedra_profile::{
    CircularEdgeConstraints, CircularSamplingError, Curve, LoopRef, SegmentRef, circular_edge_count,
};
use exedra_triangulate::{PolygonInput, TriError, TriParams, triangulate_preserving_boundary};

use super::{EdgeId, End, Extrusion, FaceId, VertexId};

/// Accuracy and work limits for [`Extrusion::tessellate`].
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

/// Where a mesh vertex lies on the native solid.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum VertexSource {
    /// A native vertex.
    Vertex(VertexId),
    /// An interior sample of a native edge, at a fraction of its sweep.
    Edge {
        /// The sampled edge.
        edge: EdgeId,
        /// Fraction from the edge's start to its end, in `(0, 1)`.
        parameter: f64,
    },
}

/// Evidence about one realization.
#[derive(Clone, Debug, PartialEq)]
pub struct TessellationReport {
    /// The requested chord tolerance.
    pub chord_tolerance: f64,
    /// Largest distance from an emitted boundary chord to its native curve,
    /// measured on the emitted `f32` coordinates in local units. Includes
    /// narrowing.
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
    /// are native face indices; rims and nonsmooth seams are sharp.
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
        /// The arc's profile segment.
        segment: SegmentRef,
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
        segment: SegmentRef,
    },
    /// A cap's emitted boundary is not a valid arrangement, and refinement
    /// could not or did not repair it.
    Arrangement {
        /// The refused cap.
        cap: End,
        /// The triangulator's diagnosis on emitted coordinates.
        error: TriError,
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
                    segment.segment, segment.loop_ref
                )
            }
            Self::VertexBudget { required, maximum } => {
                write!(f, "{required} vertices exceed the budget of {maximum}")
            }
            Self::Collapsed { segment } => write!(
                f,
                "segment {} of {:?} collapses in f32",
                segment.segment, segment.loop_ref
            ),
            Self::Arrangement { cap, error } => write!(f, "{cap:?} cap boundary: {error}"),
            Self::Build(error) => write!(f, "mesh build: {error}"),
        }
    }
}

impl core::error::Error for TessellateExtrusionError {}

/// One loop's samples, in segment order: segment `k` owns ring points
/// `starts[k] .. starts[k + 1]`, the first of which is its start vertex.
struct Ring {
    loop_ref: LoopRef,
    points: Vec<[f64; 2]>,
    starts: Vec<usize>,
}

impl Extrusion {
    /// Realizes the solid as a mesh under `policy`.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an invalid policy, an arc or vertex budget
    /// that cannot be met, samples that coincide in `f32`, or a cap boundary
    /// that is not a valid arrangement in the emitted coordinates.
    pub fn tessellate(
        &self,
        policy: &TessellationPolicy,
    ) -> Result<TessellatedExtrusion, TessellateExtrusionError> {
        if !(policy.chord_tolerance.is_finite() && policy.chord_tolerance > 0.0)
            || !(policy.smooth_seam_angle.is_finite() && policy.smooth_seam_angle >= 0.0)
            || policy.min_arc_edges == 0
            || policy.min_arc_edges > policy.max_segment_edges
        {
            return Err(TessellateExtrusionError::InvalidPolicy);
        }
        let loops = self.loops();
        let mut counts: Vec<Vec<u32>> = Vec::with_capacity(loops.len());
        for (loop_ref, loop_) in &loops {
            let mut loop_counts = Vec::with_capacity(loop_.segments().len());
            for (segment, s) in loop_.segments().iter().enumerate() {
                let count = match s.curve {
                    Curve::Line => 1,
                    Curve::Arc { .. } => {
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
                                segment: SegmentRef {
                                    loop_ref: *loop_ref,
                                    segment,
                                },
                                error,
                            }
                        })?
                    }
                };
                loop_counts.push(count);
            }
            counts.push(loop_counts);
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
            match self.realize(&rings, policy) {
                Ok(mut realized) => {
                    realized.report.segment_edges = counts.concat();
                    realized.report.refinements = refinements;
                    return Ok(realized);
                }
                Err(Refusal::Fatal(error)) => return Err(error),
                Err(Refusal::Contact {
                    cap,
                    error,
                    segments,
                }) => {
                    // Refine the arcs of the touching boundary edges: a chord
                    // that cuts across a nearby boundary moves toward its arc.
                    let mut refined = false;
                    for (loop_index, segment) in segments {
                        let count = &mut counts[loop_index][segment];
                        let is_arc = matches!(
                            loops[loop_index].1.segments()[segment].curve,
                            Curve::Arc { .. }
                        );
                        if is_arc && *count < policy.max_segment_edges {
                            *count = count.saturating_mul(2).min(policy.max_segment_edges);
                            refined = true;
                        }
                    }
                    if !refined || refinements >= policy.max_refinements {
                        return Err(TessellateExtrusionError::Arrangement { cap, error });
                    }
                    refinements += 1;
                }
            }
        }
    }

    fn loops(&self) -> Vec<(LoopRef, &exedra_profile::Loop)> {
        core::iter::once((LoopRef::Outer, self.profile.outer()))
            .chain(
                self.profile
                    .holes()
                    .iter()
                    .enumerate()
                    .map(|(hole, loop_)| (LoopRef::Hole(hole), loop_)),
            )
            .collect()
    }

    fn sample(&self, counts: &[Vec<u32>]) -> Vec<Ring> {
        self.loops()
            .into_iter()
            .zip(counts)
            .map(|((loop_ref, loop_), loop_counts)| {
                let mut points = Vec::new();
                let mut starts = Vec::with_capacity(loop_counts.len() + 1);
                for (segment, ((from, s), &count)) in
                    loop_.iter_with_starts().zip(loop_counts).enumerate()
                {
                    starts.push(points.len());
                    points.push(from);
                    if let Curve::Arc { .. } = s.curve {
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
                Ring {
                    loop_ref,
                    points,
                    starts,
                }
            })
            .collect()
    }

    #[expect(clippy::too_many_lines, reason = "one pass over shared samples")]
    fn realize(
        &self,
        rings: &[Ring],
        policy: &TessellationPolicy,
    ) -> Result<TessellatedExtrusion, Refusal> {
        let loops = self.loops();
        let total: usize = rings.iter().map(|ring| ring.points.len()).sum();
        let ends = [(End::Start, 0.0), (End::End, self.length)];
        // Emitted positions, and the same points mapped back to local
        // coordinates, for both ends.
        let mut emitted = [Vec::with_capacity(total), Vec::with_capacity(total)];
        let mut local = [Vec::with_capacity(total), Vec::with_capacity(total)];
        let mut max_vertex_displacement = 0.0_f64;
        for (level, (_, z)) in ends.iter().enumerate() {
            for ring in rings {
                for point in &ring.points {
                    let exact = [point[0], point[1], *z];
                    let placed = narrow(self.frame.transform_point(exact));
                    let back = to_local(&self.frame, placed);
                    max_vertex_displacement = max_vertex_displacement
                        .max(exedra_math::norm(exedra_math::sub(back, exact)));
                    emitted[level].push(placed);
                    local[level].push(back);
                }
            }
        }
        // Coincident consecutive samples or seam ends cannot form edges.
        let mut offset = 0;
        for ring in rings {
            let n = ring.points.len();
            for segment in 0..ring.starts.len() - 1 {
                for i in ring.starts[segment]..ring.starts[segment + 1] {
                    let (a, b) = (offset + i, offset + (i + 1) % n);
                    let collapsed = (0..2).any(|level| emitted[level][a] == emitted[level][b])
                        || emitted[0][a] == emitted[1][a];
                    if collapsed {
                        return Err(Refusal::Fatal(TessellateExtrusionError::Collapsed {
                            segment: SegmentRef {
                                loop_ref: ring.loop_ref,
                                segment,
                            },
                        }));
                    }
                }
            }
            offset += n;
        }
        // Triangulate each cap in emitted coordinates.
        let mut caps = [Vec::new(), Vec::new()];
        for (level, (end, _)) in ends.iter().enumerate() {
            let planar: Vec<[f64; 2]> = local[level].iter().map(|p| [p[0], p[1]]).collect();
            let mut slices = Vec::with_capacity(rings.len());
            let mut start = 0;
            for ring in rings {
                slices.push(&planar[start..start + ring.points.len()]);
                start += ring.points.len();
            }
            let input = PolygonInput {
                outer: slices[0],
                holes: &slices[1..],
            };
            match triangulate_preserving_boundary(&input, &TriParams::default()) {
                Ok(triangulation) => caps[level] = triangulation.triangles,
                Err(error) => {
                    let segments = match error {
                        TriError::BoundaryContact { first, second, .. } => [first, second]
                            .into_iter()
                            .map(|edge| {
                                let loop_index = edge.hole.map_or(0, |hole| hole + 1);
                                let ring = &rings[loop_index];
                                let segment = ring.starts.partition_point(|&s| s <= edge.edge) - 1;
                                (loop_index, segment)
                            })
                            .collect(),
                        // A hole left outside by chords that cut under it:
                        // refine the outer loop and that hole.
                        TriError::HoleOutsideOuter { hole } => [0, hole + 1]
                            .into_iter()
                            .flat_map(|loop_index| {
                                (0..rings[loop_index].starts.len() - 1)
                                    .map(move |segment| (loop_index, segment))
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    return Err(Refusal::Contact {
                        cap: *end,
                        error,
                        segments,
                    });
                }
            }
        }
        // Vertices: the start ring, then the end ring.
        let mut builder = MeshBuilder::new();
        for level in &emitted {
            for position in level {
                let _ = builder.push_vertex(*position);
            }
        }
        let index = |level: usize, i: usize| crate::usize_to_u32(level * total + i);
        let mut native_faces = Vec::new();
        let build = |error| Refusal::Fatal(TessellateExtrusionError::Build(error));
        // Caps: the triangulation winds counter-clockwise seen from +Z, which
        // is outward for the end cap and inward for the start cap.
        for (level, triangles) in caps.iter().enumerate() {
            let face = FaceId(crate::usize_to_u32(level));
            for [a, b, c] in triangles.iter().map(|t| t.map(|v| v as usize)) {
                let corners = if level == 0 {
                    [index(0, a), index(0, c), index(0, b)]
                } else {
                    [index(1, a), index(1, b), index(1, c)]
                };
                builder
                    .add_face_with_attrs(
                        &corners,
                        &FaceBuildAttrs {
                            region: Some(face.0),
                            edge_seams: None,
                            edge_sharpness: None,
                        },
                    )
                    .map_err(build)?;
                native_faces.push(face);
            }
        }
        // Walls: quads [start i, start j, end j, end i] per sample interval,
        // whose edges are the start rim, the lateral at j, the end rim and
        // the lateral at i.
        let mut face = 2;
        let mut offset = 0;
        for (ring, (_, loop_)) in rings.iter().zip(&loops) {
            let n = ring.points.len();
            let segments = ring.starts.len() - 1;
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                let sharp_end = !smooth_at(loop_, segment, next, policy.smooth_seam_angle);
                let previous = (segment + segments - 1) % segments;
                let sharp_start = !smooth_at(loop_, previous, segment, policy.smooth_seam_angle);
                let (first, last) = (ring.starts[segment], ring.starts[segment + 1]);
                for i in first..last {
                    let (a, b) = (offset + i, offset + (i + 1) % n);
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
                            &[index(0, a), index(0, b), index(1, b), index(1, a)],
                            &FaceBuildAttrs {
                                region: Some(face),
                                edge_seams: None,
                                edge_sharpness: Some(&sharpness),
                            },
                        )
                        .map_err(build)?;
                    native_faces.push(FaceId(face));
                }
                face += 1;
            }
            offset += n;
        }
        let built = builder.build().map_err(build)?;
        // Correspondence.
        let mut sources = Vec::with_capacity(2 * total);
        let profile_edges = self.edges.len() / 3;
        for level in 0..2 {
            let mut vertex_base = 0;
            let mut edge_base = 0;
            for ring in rings {
                let segments = ring.starts.len() - 1;
                for segment in 0..segments {
                    let count = ring.starts[segment + 1] - ring.starts[segment];
                    for i in 0..count {
                        sources.push(if i == 0 {
                            let previous = (segment + segments - 1) % segments;
                            VertexSource::Vertex(VertexId(crate::usize_to_u32(
                                level * profile_edges + vertex_base + previous,
                            )))
                        } else {
                            VertexSource::Edge {
                                edge: EdgeId(crate::usize_to_u32(
                                    level * profile_edges + edge_base + segment,
                                )),
                                parameter: f64_ratio(i, count),
                            }
                        });
                    }
                }
                vertex_base += segments;
                edge_base += segments;
            }
        }
        let mesh_vertex = |level: usize, i: usize| built.vertex_ids[level * total + i];
        let mut edge_chains = Vec::with_capacity(self.edges.len());
        for level in 0..2 {
            let mut offset = 0;
            for ring in rings {
                let n = ring.points.len();
                for segment in 0..ring.starts.len() - 1 {
                    let chain = (ring.starts[segment]..=ring.starts[segment + 1])
                        .map(|i| mesh_vertex(level, offset + i % n))
                        .collect();
                    edge_chains.push(chain);
                }
                offset += n;
            }
        }
        let mut offset = 0;
        for ring in rings {
            let n = ring.points.len();
            for segment in 0..ring.starts.len() - 1 {
                let at = offset + ring.starts[segment + 1] % n;
                edge_chains.push(alloc::vec![mesh_vertex(0, at), mesh_vertex(1, at)]);
            }
            offset += n;
        }
        // Evidence on emitted coordinates.
        let mut max_deviation = max_vertex_displacement;
        for points in &local {
            let mut offset = 0;
            for (ring, (_, loop_)) in rings.iter().zip(&loops) {
                let n = ring.points.len();
                for segment in 0..ring.starts.len() - 1 {
                    let Some(arc) = loop_.arc(segment) else {
                        continue;
                    };
                    for i in ring.starts[segment]..ring.starts[segment + 1] {
                        let (a, b) = (points[offset + i], points[offset + (i + 1) % n]);
                        max_deviation = max_deviation.max(chord_deviation(
                            [a[0], a[1]],
                            [b[0], b[1]],
                            arc.center,
                            arc.radius,
                        ));
                    }
                }
                offset += n;
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

/// Why one realization attempt stopped.
enum Refusal {
    Fatal(TessellateExtrusionError),
    Contact {
        cap: End,
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
        Curve::Arc { bulge } => {
            let arc = exedra_profile::Arc::from_bulge(from, to, bulge).expect("validated arc");
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
