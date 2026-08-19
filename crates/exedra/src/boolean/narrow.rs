// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Boolean narrow phase: triangle-triangle intersection over broad-phase
//! candidate pairs.
//!
//! Classification is exact: plane-side signs come from
//! [`exedra_triangulate::predicates::orient3d`] over positions promoted
//! exactly from f32 to f64, so whether two triangles cross, touch, are
//! coplanar, or miss is never a tolerance guess. Intersection *positions*
//! are f64 constructions (rounded like all constructed geometry) and are
//! deterministic: pure f64 arithmetic in a fixed evaluation order.
//!
//! Output segments carry endpoint provenance — which vertex or edge of
//! each source triangle produced each endpoint — which the intersection
//! graph stage (`exe-ikdf`) consumes to build connectivity, and which
//! constructive source maps compose with later.
//!
//! Never panics: degenerate and ambiguous inputs are classified into the
//! caller's [`BooleanDiagnostics`] and skipped. Exactly coplanar pairs
//! produce no segments here: they are counted in
//! [`BooleanNarrowPhaseStats::coplanar_pairs`] and owned by the coplanar
//! contact stage ([`super::collect_coplanar_contacts`]), which handles
//! face-on-face overlap regions and reports what it cannot handle as
//! typed [`super::BooleanFailureKind::CoplanarAmbiguity`] deferrals.

use alloc::vec::Vec;

use exedra_triangulate::predicates::{Orientation3d, orient3d};

use super::diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
use super::{BooleanCandidatePair, BooleanScratch, BooleanTriangleRef};
use crate::{CornerId, FaceTriangulation, Mesh};

/// How an intersection endpoint relates to one of the two triangles.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EndpointSource {
    /// The endpoint lies in the triangle's interior (on its plane).
    Interior,
    /// The endpoint lies on triangle edge `index` (0: v0→v1, 1: v1→v2,
    /// 2: v2→v0).
    Edge(u8),
    /// The endpoint is triangle vertex `index`.
    Vertex(u8),
}

/// One endpoint of an intersection segment, with provenance on both
/// triangles.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct IntersectionEndpoint {
    /// Constructed position (f64; deterministic, not exact).
    pub position: [f64; 3],
    /// Relation to triangle A.
    pub source_a: EndpointSource,
    /// Relation to triangle B.
    pub source_b: EndpointSource,
}

/// Segment classification.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SegmentKind {
    /// The triangles cross along a positive-length segment.
    Transversal,
    /// The triangles touch at a single point (or along a zero-length
    /// overlap after interval intersection).
    Touch,
}

/// One intersection segment between a candidate triangle pair.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct IntersectionSegment {
    /// The candidate pair that produced this segment.
    pub pair: BooleanCandidatePair,
    /// First endpoint (endpoints are ordered ascending along
    /// `cross(normal_a, normal_b)`).
    pub start: IntersectionEndpoint,
    /// Second endpoint.
    pub end: IntersectionEndpoint,
    /// Whether this is a crossing or a touch.
    pub kind: SegmentKind,
}

/// Deterministic narrow-phase counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct BooleanNarrowPhaseStats {
    /// Candidate pairs tested.
    pub pairs_tested: u64,
    /// Transversal segments emitted.
    pub segments: u64,
    /// Touch contacts emitted.
    pub touches: u64,
    /// Pairs skipped as exactly coplanar (owned by the coplanar contact
    /// stage, [`super::collect_coplanar_contacts`]).
    pub coplanar_pairs: u64,
    /// Pairs skipped for degenerate input triangles.
    pub degenerate_pairs: u64,
    /// Pairs with no intersection.
    pub disjoint_pairs: u64,
}

/// Computes intersection segments for broad-phase candidate pairs.
///
/// `strategy` must be the enumeration strategy the broad phase recorded in
/// [`super::BooleanBroadPhaseStats::strategy`]; triangle indices are only
/// meaningful under it. Output order is deterministic: segments appear in
/// candidate-pair order (one segment at most per pair), and each segment's
/// endpoints are ordered ascending along `cross(normal_a, normal_b)`.
///
/// `out` is cleared (retaining capacity) and appended to; diagnostics for
/// skipped pairs accumulate into `diagnostics` in candidate order.
/// `scratch` supplies reusable triangle-staging buffers shared with the
/// broad phase (allocation-free steady state; reuse is observable via
/// [`BooleanScratch::counters`]).
pub fn narrow_phase(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    pairs: &[BooleanCandidatePair],
    strategy: FaceTriangulation,
    scratch: &mut BooleanScratch,
    out: &mut Vec<IntersectionSegment>,
    diagnostics: &mut BooleanDiagnostics,
) -> BooleanNarrowPhaseStats {
    out.clear();
    let mut stats = BooleanNarrowPhaseStats::default();
    let (mut buffer_a, mut buffer_b) = (
        core::mem::take(&mut scratch.narrow_face_a),
        core::mem::take(&mut scratch.narrow_face_b),
    );
    let mut cache_a = TriangleCache::new(&mut buffer_a);
    let mut cache_b = TriangleCache::new(&mut buffer_b);

    for &pair in pairs {
        stats.pairs_tested += 1;
        let Some(tri_a) = cache_a.fetch(mesh_a, pair.a, strategy, &mut scratch.counters) else {
            diagnostics.push(BooleanDiagnostic {
                kind: BooleanFailureKind::InternalInvariantViolation,
                a: Some(pair.a),
                b: Some(pair.b),
                detail: "candidate references a stale or missing triangle in mesh A",
            });
            continue;
        };
        let Some(tri_b) = cache_b.fetch(mesh_b, pair.b, strategy, &mut scratch.counters) else {
            diagnostics.push(BooleanDiagnostic {
                kind: BooleanFailureKind::InternalInvariantViolation,
                a: Some(pair.a),
                b: Some(pair.b),
                detail: "candidate references a stale or missing triangle in mesh B",
            });
            continue;
        };
        match intersect_pair(pair, &tri_a, &tri_b) {
            PairOutcome::Segment(segment) => {
                match segment.kind {
                    SegmentKind::Transversal => stats.segments += 1,
                    SegmentKind::Touch => stats.touches += 1,
                }
                out.push(segment);
            }
            PairOutcome::Disjoint => stats.disjoint_pairs += 1,
            // Coplanar pairs are the coplanar contact stage's input; they
            // are counted here, not diagnosed.
            PairOutcome::Coplanar => stats.coplanar_pairs += 1,
            PairOutcome::Degenerate => {
                stats.degenerate_pairs += 1;
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::DegenerateTriangle,
                    a: Some(pair.a),
                    b: Some(pair.b),
                    detail: "zero-area triangle in candidate pair",
                });
            }
            PairOutcome::Unstable => {
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::NumericalInstability,
                    a: Some(pair.a),
                    b: Some(pair.b),
                    detail: "plane intersection direction vanished for a crossing pair",
                });
            }
        }
    }
    // The caches only borrow the buffers; ending their scope releases the
    // borrows so the buffers return to the scratch for the next call.
    let _ = (cache_a, cache_b);
    scratch.narrow_face_a = buffer_a;
    scratch.narrow_face_b = buffer_b;
    stats
}

/// Per-mesh triangle memo over a reused enumeration buffer: candidate
/// pairs arrive sorted, so consecutive pairs usually share a face and the
/// buffer is only re-filled on face changes.
struct TriangleCache<'a> {
    buffer: &'a mut Vec<[CornerId; 3]>,
    face: Option<crate::FaceId>,
    last: Option<(BooleanTriangleRef, [[f64; 3]; 3])>,
}

impl<'a> TriangleCache<'a> {
    fn new(buffer: &'a mut Vec<[CornerId; 3]>) -> Self {
        Self {
            buffer,
            face: None,
            last: None,
        }
    }

    fn fetch(
        &mut self,
        mesh: &Mesh,
        triangle: BooleanTriangleRef,
        strategy: FaceTriangulation,
        counters: &mut super::BooleanScratchCounters,
    ) -> Option<[[f64; 3]; 3]> {
        if let Some((cached, positions)) = &self.last
            && *cached == triangle
        {
            counters.enumeration_cache_hits += 1;
            return Some(*positions);
        }
        if self.face != Some(triangle.face) {
            let _ = mesh.face_triangles_into(triangle.face, strategy, self.buffer);
            counters.face_enumerations += 1;
            self.face = Some(triangle.face);
        } else {
            counters.enumeration_cache_hits += 1;
        }
        let corners = self.buffer.get(triangle.triangle_index as usize)?;
        let mut positions = [[0.0_f64; 3]; 3];
        for (slot, corner) in positions.iter_mut().zip(corners.iter()) {
            let vertex = mesh.to_vertex(*corner)?;
            let p = mesh.vertex_position(vertex)?;
            // Exact promotion: every f32 is exactly representable in f64.
            *slot = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
        }
        self.last = Some((triangle, positions));
        Some(positions)
    }
}

enum PairOutcome {
    Segment(IntersectionSegment),
    Disjoint,
    Coplanar,
    Degenerate,
    Unstable,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Exact plane-side signs of `points` against the plane of `(p0, p1, p2)`:
/// `+1` above (CCW-normal side), `-1` below, `0` on.
fn plane_signs(plane: &[[f64; 3]; 3], points: &[[f64; 3]; 3]) -> [i8; 3] {
    let mut signs = [0_i8; 3];
    for (sign, point) in signs.iter_mut().zip(points.iter()) {
        *sign = match orient3d(plane[0], plane[1], plane[2], *point) {
            Orientation3d::Above => 1,
            Orientation3d::Below => -1,
            Orientation3d::Coplanar => 0,
        };
    }
    signs
}

/// True when the triangle has exactly collinear corners.
fn is_degenerate(tri: &[[f64; 3]; 3]) -> bool {
    // Collinear iff every point of the triangle is coplanar with every
    // plane through the three corners — equivalently the corners plus any
    // off-line probe are coplanar for two independent probes. Cheaper and
    // exact enough for classification: the cross product of the promoted
    // edges is exactly zero (f32-promoted coordinates make this a wide
    // target only for genuinely degenerate fans).
    cross(sub(tri[1], tri[0]), sub(tri[2], tri[0])) == [0.0, 0.0, 0.0]
}

/// The (up to two) points where a triangle meets the other triangle's
/// plane, with provenance.
struct PlaneCut {
    points: [([f64; 3], EndpointSource); 2],
    count: usize,
}

/// Computes the triangle/plane cut given exact per-vertex signs.
///
/// Returns `None` when the triangle does not reach the plane. Positions
/// for edge crossings are f64 constructions from signed-distance ratios;
/// vertices on the plane are passed through exactly.
fn plane_cut(tri: &[[f64; 3]; 3], plane: &[[f64; 3]; 3], signs: [i8; 3]) -> Option<PlaneCut> {
    let zeros = signs.iter().filter(|s| **s == 0).count();
    let positives = signs.iter().filter(|s| **s > 0).count();
    let negatives = signs.iter().filter(|s| **s < 0).count();

    if zeros == 0 && (positives == 0 || negatives == 0) {
        return None; // Strictly one side.
    }

    let mut cut = PlaneCut {
        points: [([0.0; 3], EndpointSource::Interior); 2],
        count: 0,
    };
    let push = |point: [f64; 3], source: EndpointSource, cut: &mut PlaneCut| {
        if cut.count < 2 {
            cut.points[cut.count] = (point, source);
            cut.count += 1;
        }
    };

    // Vertices exactly on the plane pass through exactly.
    for (index, sign) in signs.iter().enumerate() {
        if *sign == 0 {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "triangle vertex indices are 0..3"
            )]
            push(tri[index], EndpointSource::Vertex(index as u8), &mut cut);
        }
    }
    if zeros == 3 {
        // Coplanar; handled by the caller before this point.
        return None;
    }
    if zeros == 1 && (positives == 0 || negatives == 0) {
        // Touching the plane at one vertex only.
        return Some(cut);
    }
    if zeros == 2 {
        // An edge lies in the plane; both endpoints already pushed.
        return Some(cut);
    }

    // Edge crossings: edges whose endpoint signs strictly straddle.
    // Signed distances via the plane normal are f64 constructions; the
    // *decision* to cross came from exact signs, so the parameter is a
    // ratio of same-sign quantities and stays in (0, 1).
    let normal = cross(sub(plane[1], plane[0]), sub(plane[2], plane[0]));
    for edge in 0..3_u8 {
        let i = edge as usize;
        let j = (i + 1) % 3;
        if signs[i] as i16 * signs[j] as i16 == -1 {
            let di = dot(normal, sub(tri[i], plane[0]));
            let dj = dot(normal, sub(tri[j], plane[0]));
            let t = di / (di - dj);
            push(
                lerp(tri[i], tri[j], t),
                EndpointSource::Edge(edge),
                &mut cut,
            );
        }
    }
    Some(cut)
}

fn intersect_pair(
    pair: BooleanCandidatePair,
    tri_a: &[[f64; 3]; 3],
    tri_b: &[[f64; 3]; 3],
) -> PairOutcome {
    if is_degenerate(tri_a) || is_degenerate(tri_b) {
        return PairOutcome::Degenerate;
    }

    // Exact classification of each triangle against the other's plane.
    let signs_a = plane_signs(tri_b, tri_a); // A's vertices vs plane of B
    if signs_a == [0, 0, 0] {
        return PairOutcome::Coplanar;
    }
    if signs_a.iter().all(|s| *s > 0) || signs_a.iter().all(|s| *s < 0) {
        return PairOutcome::Disjoint;
    }
    let signs_b = plane_signs(tri_a, tri_b);
    if signs_b.iter().all(|s| *s > 0) || signs_b.iter().all(|s| *s < 0) {
        return PairOutcome::Disjoint;
    }

    let Some(cut_a) = plane_cut(tri_a, tri_b, signs_a) else {
        return PairOutcome::Disjoint;
    };
    let Some(cut_b) = plane_cut(tri_b, tri_a, signs_b) else {
        return PairOutcome::Disjoint;
    };

    // Both cuts lie on the plane-intersection line; overlap their scalar
    // intervals along its direction.
    let normal_a = cross(sub(tri_a[1], tri_a[0]), sub(tri_a[2], tri_a[0]));
    let normal_b = cross(sub(tri_b[1], tri_b[0]), sub(tri_b[2], tri_b[0]));
    let direction = cross(normal_a, normal_b);
    if direction == [0.0, 0.0, 0.0] {
        // Exact signs said the planes cross, but the constructed direction
        // vanished (extreme near-parallel). Classified, not guessed.
        return PairOutcome::Unstable;
    }

    let interval_a = Interval::from_cut(&cut_a, direction);
    let interval_b = Interval::from_cut(&cut_b, direction);

    // Overlap: max of the mins, min of the maxes; provenance follows
    // whichever cut point bounds the overlap.
    let (start_scalar, start_a, start_b) = if interval_a.min.0 >= interval_b.min.0 {
        (interval_a.min.0, interval_a.min.1, EndpointSource::Interior)
    } else {
        (interval_b.min.0, EndpointSource::Interior, interval_b.min.1)
    };
    let start_point = if interval_a.min.0 >= interval_b.min.0 {
        interval_a.min.2
    } else {
        interval_b.min.2
    };
    let (end_scalar, end_a, end_b) = if interval_a.max.0 <= interval_b.max.0 {
        (interval_a.max.0, interval_a.max.1, EndpointSource::Interior)
    } else {
        (interval_b.max.0, EndpointSource::Interior, interval_b.max.1)
    };
    let end_point = if interval_a.max.0 <= interval_b.max.0 {
        interval_a.max.2
    } else {
        interval_b.max.2
    };

    if start_scalar > end_scalar {
        return PairOutcome::Disjoint;
    }

    let kind = if start_scalar == end_scalar {
        SegmentKind::Touch
    } else {
        SegmentKind::Transversal
    };
    // When both intervals share a bound exactly, prefer the sharper
    // provenance from each triangle.
    let (start_a, start_b) = refine_shared_bound(
        start_scalar,
        &interval_a,
        &interval_b,
        true,
        start_a,
        start_b,
    );
    let (end_a, end_b) =
        refine_shared_bound(end_scalar, &interval_a, &interval_b, false, end_a, end_b);

    PairOutcome::Segment(IntersectionSegment {
        pair,
        start: IntersectionEndpoint {
            position: start_point,
            source_a: start_a,
            source_b: start_b,
        },
        end: IntersectionEndpoint {
            position: end_point,
            source_a: end_a,
            source_b: end_b,
        },
        kind,
    })
}

/// Scalar interval of a plane cut along the intersection direction.
///
/// `min`/`max` carry `(scalar, provenance, position)`; ties inside a cut
/// resolve by keeping the first point in cut order (deterministic).
struct Interval {
    min: (f64, EndpointSource, [f64; 3]),
    max: (f64, EndpointSource, [f64; 3]),
}

impl Interval {
    fn from_cut(cut: &PlaneCut, direction: [f64; 3]) -> Self {
        let first = cut.points[0];
        let first_scalar = dot(first.0, direction);
        let mut interval = Self {
            min: (first_scalar, first.1, first.0),
            max: (first_scalar, first.1, first.0),
        };
        if cut.count == 2 {
            let second = cut.points[1];
            let second_scalar = dot(second.0, direction);
            if second_scalar < interval.min.0 {
                interval.min = (second_scalar, second.1, second.0);
            }
            if second_scalar > interval.max.0 {
                interval.max = (second_scalar, second.1, second.0);
            }
        }
        interval
    }
}

/// When an overlap bound is shared by both intervals exactly, both
/// triangles contribute provenance for that endpoint.
fn refine_shared_bound(
    scalar: f64,
    interval_a: &Interval,
    interval_b: &Interval,
    is_min: bool,
    default_a: EndpointSource,
    default_b: EndpointSource,
) -> (EndpointSource, EndpointSource) {
    let bound_a = if is_min {
        &interval_a.min
    } else {
        &interval_a.max
    };
    let bound_b = if is_min {
        &interval_b.min
    } else {
        &interval_b.max
    };
    let a = if bound_a.0 == scalar {
        bound_a.1
    } else {
        default_a
    };
    let b = if bound_b.0 == scalar {
        bound_b.1
    } else {
        default_b
    };
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::{BooleanBvh, BooleanScratch};
    use crate::{BuildParams, MeshBuilder};

    fn tri_ref(face_index: u32) -> BooleanTriangleRef {
        BooleanTriangleRef {
            face: crate::FaceId::new(face_index, core::num::NonZeroU32::MIN),
            triangle_index: 0,
        }
    }

    fn pair() -> BooleanCandidatePair {
        BooleanCandidatePair {
            a: tri_ref(0),
            b: tri_ref(1),
        }
    }

    #[test]
    fn crossing_triangles_produce_the_expected_segment() {
        // A in the z = 0 plane, B perpendicular through x = [0.25, 0.75]
        // at y = 0.25 line: hand-computable intersection along y = 0.25?
        // Simpler canonical: B vertical crossing the interior.
        let a = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let b = [[0.5, 0.5, -1.0], [1.5, 0.5, -1.0], [0.5, 0.5, 1.0]];
        let PairOutcome::Segment(segment) = intersect_pair(pair(), &a, &b) else {
            panic!("expected a segment");
        };
        assert_eq!(segment.kind, SegmentKind::Transversal);
        // The intersection lies on z = 0, y = 0.5, x in [0.5, 1.0]:
        // B's plane is y = 0.5? No: B spans x with fixed y = 0.5 — its
        // plane is y = 0.5. A's plane is z = 0. Line: y = 0.5, z = 0.
        // B cut on z=0: from (0.5,0.5,0) (edge v0->v2) to (1.0,0.5,0)
        // (edge v1->v2 midpoint at z=0: (1.0,0.5,0)). A covers that whole
        // span (interior), so the segment is exactly that.
        let mut xs = [segment.start.position[0], segment.end.position[0]];
        xs.sort_unstable_by(f64::total_cmp);
        assert!((xs[0] - 0.5).abs() < 1e-12 && (xs[1] - 1.0).abs() < 1e-12);
        for endpoint in [segment.start, segment.end] {
            assert!((endpoint.position[1] - 0.5).abs() < 1e-12);
            assert!(endpoint.position[2].abs() < 1e-12);
            // Both endpoints come from B's edges crossing A's interior.
            assert!(matches!(endpoint.source_b, EndpointSource::Edge(_)));
            assert_eq!(endpoint.source_a, EndpointSource::Interior);
        }
    }

    #[test]
    fn vertex_touch_classifies_as_touch() {
        let a = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        // B touches A's plane at exactly one vertex (0.5, 0.5, 0).
        let b = [[0.5, 0.5, 0.0], [1.5, 0.5, 1.0], [0.5, 1.5, 1.0]];
        let PairOutcome::Segment(segment) = intersect_pair(pair(), &a, &b) else {
            panic!("expected a touch");
        };
        assert_eq!(segment.kind, SegmentKind::Touch);
        assert_eq!(segment.start.source_b, EndpointSource::Vertex(0));
        assert_eq!(segment.start.position, [0.5, 0.5, 0.0]);
    }

    #[test]
    fn edge_in_plane_produces_edge_overlap() {
        let a = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        // B's v0->v1 edge lies exactly in A's plane, inside A.
        let b = [[0.25, 0.25, 0.0], [1.0, 0.25, 0.0], [0.5, 0.5, 1.0]];
        let PairOutcome::Segment(segment) = intersect_pair(pair(), &a, &b) else {
            panic!("expected a segment");
        };
        assert_eq!(segment.kind, SegmentKind::Transversal);
        for endpoint in [segment.start, segment.end] {
            assert!(matches!(endpoint.source_b, EndpointSource::Vertex(_)));
            assert_eq!(endpoint.source_a, EndpointSource::Interior);
            assert_eq!(endpoint.position[2], 0.0);
        }
    }

    #[test]
    fn coplanar_and_disjoint_classify() {
        let a = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let coplanar = [[3.0, 3.0, 0.0], [5.0, 3.0, 0.0], [3.0, 5.0, 0.0]];
        assert!(matches!(
            intersect_pair(pair(), &a, &coplanar),
            PairOutcome::Coplanar
        ));
        let above = [[0.0, 0.0, 1.0], [2.0, 0.0, 1.0], [0.0, 2.0, 2.0]];
        assert!(matches!(
            intersect_pair(pair(), &a, &above),
            PairOutcome::Disjoint
        ));
        // Straddles the plane but misses the triangle.
        let miss = [[5.0, 5.0, -1.0], [6.0, 5.0, 1.0], [5.0, 6.0, 1.0]];
        assert!(matches!(
            intersect_pair(pair(), &a, &miss),
            PairOutcome::Disjoint
        ));
        let degenerate = [[0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.5]];
        assert!(matches!(
            intersect_pair(pair(), &a, &degenerate),
            PairOutcome::Degenerate
        ));
    }

    #[test]
    fn near_parallel_torture_is_deterministic_and_calm() {
        let a = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]];
        for i in 1..=64_u32 {
            let tilt = f64::from(i) * 1e-14;
            let b = [[0.5, 0.5, -1e-13], [3.0, 0.5, tilt], [0.5, 3.0, tilt * 0.5]];
            let first = intersect_pair(pair(), &a, &b);
            let second = intersect_pair(pair(), &a, &b);
            match (&first, &second) {
                (PairOutcome::Segment(x), PairOutcome::Segment(y)) => {
                    assert_eq!(x, y, "tilt {i}: bit-determinism");
                }
                (PairOutcome::Disjoint, PairOutcome::Disjoint)
                | (PairOutcome::Coplanar, PairOutcome::Coplanar)
                | (PairOutcome::Unstable, PairOutcome::Unstable)
                | (PairOutcome::Degenerate, PairOutcome::Degenerate) => {}
                _ => panic!("tilt {i}: outcome must be reproducible"),
            }
        }
    }

    /// Builds a unit cube mesh with its minimum corner at `origin`.
    fn cube(origin: [f32; 3]) -> Mesh {
        let o = origin;
        let positions = [
            [o[0], o[1], o[2]],
            [o[0] + 1.0, o[1], o[2]],
            [o[0] + 1.0, o[1] + 1.0, o[2]],
            [o[0], o[1] + 1.0, o[2]],
            [o[0], o[1], o[2] + 1.0],
            [o[0] + 1.0, o[1], o[2] + 1.0],
            [o[0] + 1.0, o[1] + 1.0, o[2] + 1.0],
            [o[0], o[1] + 1.0, o[2] + 1.0],
        ];
        let faces: [[u32; 4]; 6] = [
            [3, 2, 1, 0],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(&face).expect("valid cube face");
        }
        builder.build().expect("valid cube").mesh
    }

    #[test]
    fn two_box_end_to_end_broad_plus_narrow() {
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, 0.5, 0.5]);
        let mut scratch = BooleanScratch::new();
        let strategy = FaceTriangulation::Fan;
        let bvh_a = BooleanBvh::build(&mesh_a, strategy, &mut scratch);
        let bvh_b = BooleanBvh::build(&mesh_b, strategy, &mut scratch);
        let mut pairs = Vec::new();
        let stats = bvh_a.query_overlaps(&bvh_b, &mut scratch, &mut pairs);
        assert_eq!(stats.strategy, strategy);
        assert!(!pairs.is_empty());

        let mut segments = Vec::new();
        let mut diagnostics = BooleanDiagnostics::default();
        let narrow_stats = narrow_phase(
            &mesh_a,
            &mesh_b,
            &pairs,
            strategy,
            &mut scratch,
            &mut segments,
            &mut diagnostics,
        );
        let counters = scratch.counters();
        assert!(counters.face_enumerations > 0);
        assert!(
            counters.enumeration_cache_hits > 0,
            "sorted pairs must hit the per-face memo"
        );
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(
            narrow_stats.segments >= 6,
            "two overlapping cubes cross along at least six face-pair segments, got {}",
            narrow_stats.segments
        );
        // Every emitted segment lies inside the shared overlap cube
        // [0.5, 1.0]^3 (with construction-level slack).
        for segment in &segments {
            for endpoint in [segment.start, segment.end] {
                for axis in 0..3 {
                    assert!(
                        endpoint.position[axis] >= 0.5 - 1e-9
                            && endpoint.position[axis] <= 1.0 + 1e-9,
                        "endpoint outside overlap: {:?}",
                        endpoint.position
                    );
                }
            }
        }

        // End-to-end double-run determinism.
        let mut segments_again = Vec::new();
        let mut diagnostics_again = BooleanDiagnostics::default();
        let narrow_again = narrow_phase(
            &mesh_a,
            &mesh_b,
            &pairs,
            strategy,
            &mut scratch,
            &mut segments_again,
            &mut diagnostics_again,
        );
        assert_eq!(narrow_stats, narrow_again);
        assert_eq!(segments, segments_again);
        let _ = BuildParams::default();
    }
}
