// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Caller-defined layer transfer along an operation's source correspondence.
//!
//! Operations that rebuild topology know, for every output face, the source
//! face it came from and where each of its corners lies on that source face.
//! [`Transfer`] turns that correspondence into layer values: output faces take
//! their source face's values, corners and vertices take the source face's
//! corners or vertices weighted by where they lie on it. A point at a corner's
//! position selects that corner alone, and a point on a face-loop edge
//! interpolates its two ends; both are checked on the face loop itself, so a
//! corner the robust triangulation drops (a collinear T-vertex) is still
//! exact. Other points take barycentric coordinates in the robust
//! triangulation: the containing triangle, or the one the point overshoots
//! least, clamped and renormalized, so arbitrary data is never extrapolated.
//! Only positive weights are kept, heaviest first, so every rule (including
//! `Copy`) takes a source vertex's value exactly.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use exedra_math::{cross, dot, promote, sub};
use exedra_mesh::attributes::{AttrError, Domain};
use exedra_mesh::{
    ChangeSetBuilder, FaceId, FaceTriangulation, HalfEdgeId, Mesh, VertexId, attr, op,
};

/// True when `mesh` carries any caller-defined layer.
/// True when `mesh` has a caller-defined layer in `domain`.
fn has_caller_layers_in(mesh: &Mesh, domain: Domain) -> bool {
    mesh.attrs()
        .keys()
        .any(|(d, name)| d == domain && !attr::is_reserved(d, name))
}

pub(crate) fn has_caller_layers(mesh: &Mesh) -> bool {
    mesh.attrs()
        .keys()
        .any(|(domain, name)| !attr::is_reserved(domain, name))
}

/// Where an output vertex's values come from.
#[derive(Copy, Clone, Debug)]
pub(crate) enum VertexSample {
    /// A surviving source vertex.
    Vertex(VertexId),
    /// A point on a source face, in source coordinates.
    Point { face: FaceId, point: [f64; 3] },
    /// A point on the segment between two source vertices, `parameter` of
    /// the way from the first.
    Edge {
        vertices: [VertexId; 2],
        parameter: f64,
    },
}

/// Where an output corner's values come from.
#[derive(Copy, Clone, Debug)]
pub(crate) enum CornerSample {
    /// A source corner, carried as is.
    Corner(HalfEdgeId),
    /// A point on the output face's source face, in source coordinates.
    Point([f64; 3]),
}

/// One output face's source face and per-corner samples.
struct FaceRecord {
    output: FaceId,
    source: FaceId,
    corners: Vec<(HalfEdgeId, CornerSample)>,
}

/// Accumulates a rebuild's correspondence and writes the values.
pub(crate) struct Transfer<'a> {
    source: &'a Mesh,
    /// Relabeling rebuild: exact samples are written as is, whatever the rule.
    verbatim: bool,
    charts: BTreeMap<FaceId, Chart>,
    faces: Vec<FaceRecord>,
    vertices: Vec<(VertexId, VertexSample)>,
}

impl<'a> Transfer<'a> {
    /// Returns `None` when `source` has no caller-defined layer.
    pub(crate) fn new(source: &'a Mesh) -> Option<Self> {
        has_caller_layers(source).then(|| Self {
            source,
            verbatim: false,
            charts: BTreeMap::new(),
            faces: Vec::new(),
            vertices: Vec::new(),
        })
    }

    /// Like [`Self::new`], for a rebuild that only relabels elements (every
    /// output element is one source element with the same meaning): values are
    /// written as they are, whatever each layer's rule. Only exact samples
    /// ([`CornerSample::Corner`], [`VertexSample::Vertex`]) may be recorded.
    pub(crate) fn verbatim(source: &'a Mesh) -> Option<Self> {
        Self::new(source).map(|transfer| Self {
            verbatim: true,
            ..transfer
        })
    }

    /// Records that `output` came from `source_face`, with each output
    /// corner's source.
    pub(crate) fn face(
        &mut self,
        output: FaceId,
        source_face: FaceId,
        corners: Vec<(HalfEdgeId, CornerSample)>,
    ) {
        self.faces.push(FaceRecord {
            output,
            source: source_face,
            corners,
        });
    }

    /// Records where an output vertex's values come from.
    pub(crate) fn vertex(&mut self, output: VertexId, sample: VertexSample) {
        self.vertices.push((output, sample));
    }

    /// Adopts the source's caller-defined layers onto `output` and writes every
    /// recorded value under each layer's rule. Returns the values an
    /// `Unspecified` rule could not carry, plus one per output corner or
    /// vertex sampled on a source face with no area (every triangle
    /// degenerate and the point on no loop edge), whose layers of that
    /// domain therefore start empty.
    pub(crate) fn apply(mut self, output: &mut Mesh) -> Result<u64, AttrError> {
        let _ = output.adopt_attribute_layers(self.source)?;
        let source = self.source;
        let corner_layers = has_caller_layers_in(source, Domain::HalfEdge);
        let vertex_layers = has_caller_layers_in(source, Domain::Vertex);
        let mut unsampled = 0_u64;
        let mut edit = output.edit_with(ChangeSetBuilder::new());
        let verbatim = self.verbatim;
        let exact = |element| {
            if verbatim {
                source.capture_attributes_verbatim(element)
            } else {
                source.capture_attributes(&[(element, 1.0)])
            }
        };
        for record in core::mem::take(&mut self.faces) {
            let FaceRecord {
                output: face,
                source: source_face,
                corners,
            } = record;
            let captured = if verbatim {
                source.capture_attributes_verbatim(source_face)
            } else {
                source.capture_attributes(&[(source_face, 1.0)])
            };
            let _ = op::restore_attributes(&mut edit, face, &captured);
            for (corner, sample) in corners {
                let captured = match sample {
                    CornerSample::Corner(source_corner) => exact(source_corner),
                    CornerSample::Point(point) => {
                        debug_assert!(!verbatim, "relabeling rebuilds use exact samples");
                        let chart = self
                            .charts
                            .entry(source_face)
                            .or_insert_with(|| Chart::new(source, source_face));
                        let weights = chart.weights(point);
                        if weights.is_empty() && corner_layers {
                            unsampled += 1;
                        }
                        source.capture_attributes(&weights)
                    }
                };
                let _ = op::restore_attributes(&mut edit, corner, &captured);
            }
        }
        for (vertex, sample) in core::mem::take(&mut self.vertices) {
            let captured = match sample {
                VertexSample::Vertex(source_vertex) => {
                    if verbatim {
                        source.capture_attributes_verbatim(source_vertex)
                    } else {
                        source.capture_attributes(&[(source_vertex, 1.0)])
                    }
                }
                VertexSample::Edge {
                    vertices: [a, b],
                    parameter,
                } => {
                    debug_assert!(!verbatim, "relabeling rebuilds use exact samples");
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "weights narrow once for capture"
                    )]
                    let t = parameter.clamp(0.0, 1.0) as f32;
                    source.capture_attributes(&positive_weights(&[(a, 1.0 - t), (b, t)]))
                }
                VertexSample::Point { face, point } => {
                    debug_assert!(!verbatim, "relabeling rebuilds use exact samples");
                    let chart = self
                        .charts
                        .entry(face)
                        .or_insert_with(|| Chart::new(source, face));
                    let weights: Vec<(VertexId, f32)> = chart
                        .weights(point)
                        .into_iter()
                        .filter_map(|(corner, w)| source.to_vertex(corner).map(|v| (v, w)))
                        .collect();
                    if weights.is_empty() && vertex_layers {
                        unsampled += 1;
                    }
                    source.capture_attributes(&weights)
                }
            };
            let _ = op::restore_attributes(&mut edit, vertex, &captured);
        }
        Ok(edit.finish().unpropagated_attribute_values + unsampled)
    }
}

/// Distance within which a point counts as lying on a face-loop edge, as the
/// larger of two terms.
///
/// Positions are stored in `f32`. A point computed on an edge (a split or a
/// section cut) and narrowed moves by at most half an ulp per coordinate,
/// which scales with the coordinate's magnitude, not with the edge's length:
/// far from the origin a short edge's narrowed points sit many edge-relative
/// `2^-20` away from it. So the tolerance is `2^-20` of the edge length, or
/// [`ON_EDGE_ULPS`] `f32` epsilons of the largest endpoint coordinate
/// magnitude, whichever is larger. Anything farther is interior and takes
/// barycentric weights.
const ON_EDGE_TOLERANCE: f64 = 1.0 / 1_048_576.0;

/// `f32` epsilons of coordinate magnitude a narrowed on-edge point may stray,
/// with margin over the half-ulp-per-coordinate bound (at most `√3/2`
/// epsilons of the largest coordinate).
const ON_EDGE_ULPS: f64 = 4.0;

/// A source face's loop and robust triangulation, for sample weights.
pub(crate) struct Chart {
    /// Every face-loop corner with its position, in loop order.
    corners: Vec<(HalfEdgeId, [f64; 3])>,
    triangles: Vec<([HalfEdgeId; 3], [[f64; 3]; 3])>,
}

impl Chart {
    pub(crate) fn new(mesh: &Mesh, face: FaceId) -> Self {
        let corners = mesh
            .face_loop(face)
            .filter_map(|corner| {
                let vertex = mesh.to_vertex(corner)?;
                mesh.vertex_position(vertex).map(|p| (corner, promote(*p)))
            })
            .collect();
        let (triangles, _) = mesh.face_triangles_counted(face, FaceTriangulation::Robust);
        let triangles = triangles
            .into_iter()
            .filter_map(|corners| {
                let points = corners.map(|corner| {
                    let vertex = mesh.to_vertex(corner)?;
                    mesh.vertex_position(vertex).map(|p| promote(*p))
                });
                let [Some(a), Some(b), Some(c)] = points else {
                    return None;
                };
                Some((corners, [a, b, c]))
            })
            .collect();
        Self { corners, triangles }
    }

    /// Sample weights of `point` over the face's corners.
    ///
    /// A point at a corner's exact position yields that corner alone. A point
    /// on a face-loop edge (within the on-edge tolerance: [`ON_EDGE_TOLERANCE`]
    /// of its length or [`ON_EDGE_ULPS`] `f32` epsilons of its coordinates)
    /// interpolates the edge's two corners by distance along it. Both checks
    /// walk the face loop, so corners the robust triangulation drops as
    /// collinear still sample exactly. Otherwise the weights are the clamped
    /// barycentric coordinates in the containing triangle, or the
    /// least-overshot triangle when none contains it.
    ///
    /// Zero weights are dropped and the rest ordered heaviest first (ties in
    /// corner order). Empty when the point is on no edge and every triangle of
    /// the face is degenerate (zero area): such a face has no interior to
    /// sample, so its corners and vertices capture nothing; [`Transfer::apply`]
    /// counts those elements as unpropagated.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "weights narrow once for capture; they are renormalized there"
    )]
    pub(crate) fn weights(&self, point: [f64; 3]) -> Vec<(HalfEdgeId, f32)> {
        if let Some(&(corner, _)) = self.corners.iter().find(|(_, p)| *p == point) {
            return alloc::vec![(corner, 1.0)];
        }
        if let Some(weights) = self.edge_weights(point) {
            return weights;
        }
        let mut best: Option<(f64, usize, [f64; 3])> = None;
        for (index, (_, points)) in self.triangles.iter().enumerate() {
            let Some(weights) = barycentric(points, point) else {
                continue;
            };
            let outside = weights.iter().map(|w| (-w).max(0.0)).sum::<f64>();
            if best.is_none_or(|(score, _, _)| outside < score) {
                best = Some((outside, index, weights));
            }
            if outside == 0.0 {
                break;
            }
        }
        let Some((_, index, weights)) = best else {
            return Vec::new();
        };
        let clamped = weights.map(|w| w.max(0.0));
        let total: f64 = clamped.iter().sum();
        let corners = self.triangles[index].0;
        positive_weights(&[0, 1, 2].map(|k| {
            let weight = if total > 0.0 {
                (clamped[k] / total) as f32
            } else {
                0.0
            };
            (corners[k], weight)
        }))
    }
}

impl Chart {
    /// Weights of `point` when it lies on a face-loop edge strictly between
    /// its two corners.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "weights narrow once for capture; they are renormalized there"
    )]
    fn edge_weights(&self, point: [f64; 3]) -> Option<Vec<(HalfEdgeId, f32)>> {
        let count = self.corners.len();
        for k in 0..count {
            let (from, a) = self.corners[k];
            let (to, b) = self.corners[(k + 1) % count];
            let edge = sub(b, a);
            let length_squared = dot(edge, edge);
            if !(length_squared > 0.0 && length_squared.is_finite()) {
                continue;
            }
            let t = dot(sub(point, a), edge) / length_squared;
            if !(t > 0.0 && t < 1.0) {
                continue;
            }
            let offset = sub(
                point,
                [a[0] + t * edge[0], a[1] + t * edge[1], a[2] + t * edge[2]],
            );
            let magnitude = a
                .iter()
                .chain(b.iter())
                .fold(0.0_f64, |largest, c| largest.max(c.abs()));
            let absolute = ON_EDGE_ULPS * f64::from(f32::EPSILON) * magnitude;
            let tolerance =
                (ON_EDGE_TOLERANCE * ON_EDGE_TOLERANCE * length_squared).max(absolute * absolute);
            if dot(offset, offset) <= tolerance {
                return Some(positive_weights(&[
                    (from, (1.0 - t) as f32),
                    (to, t as f32),
                ]));
            }
        }
        None
    }
}

/// Keeps the finite positive weights of `pairs`, heaviest first with ties in
/// the given order, so a capture's first source is its dominant one.
fn positive_weights<E: Copy>(pairs: &[(E, f32)]) -> Vec<(E, f32)> {
    let mut kept: Vec<(E, f32)> = pairs
        .iter()
        .copied()
        .filter(|&(_, weight)| weight.is_finite() && weight > 0.0)
        .collect();
    // `sort_by` is stable, so equal weights keep their order.
    kept.sort_by(|a, b| b.1.total_cmp(&a.1));
    kept
}

/// Barycentric coordinates of `point` projected onto the triangle's plane.
fn barycentric(points: &[[f64; 3]; 3], point: [f64; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = *points;
    let normal = cross(sub(b, a), sub(c, a));
    let normal_squared = dot(normal, normal);
    if !(normal_squared > 0.0 && normal_squared.is_finite()) {
        return None;
    }
    let offset = sub(point, a);
    let v = dot(cross(offset, sub(c, a)), normal) / normal_squared;
    let w = dot(cross(sub(b, a), offset), normal) / normal_squared;
    let weights = [1.0 - v - w, v, w];
    weights.iter().all(|w| w.is_finite()).then_some(weights)
}
