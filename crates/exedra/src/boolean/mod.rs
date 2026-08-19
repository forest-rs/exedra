// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Boolean broad-phase candidate discovery.
//!
//! This module owns the deterministic broad phase for staged boolean
//! operations. It builds a bounding-volume hierarchy over the triangles
//! enumerated by [`Mesh::face_triangles`] under an explicit
//! [`FaceTriangulation`] strategy (recorded on the hierarchy and in its
//! stats) and reports potentially overlapping triangle pairs. Narrow-phase
//! intersection, classification, splitting, and stitching are intentionally
//! out of scope for this module.

use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::{CornerId, FaceId, FaceTriangulation, Mesh};

mod diag;
mod narrow;

pub use diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
pub use narrow::{
    BooleanNarrowPhaseStats, EndpointSource, IntersectionEndpoint, IntersectionSegment,
    SegmentKind, narrow_phase,
};

const LEAF_TRIANGLE_COUNT: usize = 4;

/// Axis-aligned bounding box used by the boolean broad phase.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum coordinate per axis.
    pub min: [f32; 3],
    /// Maximum coordinate per axis.
    pub max: [f32; 3],
}

impl Aabb {
    /// Empty bounds that expand when points or boxes are included.
    pub const EMPTY: Self = Self {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };

    /// Builds bounds covering all provided points.
    #[must_use]
    pub fn from_points(points: &[[f32; 3]]) -> Self {
        let mut bounds = Self::EMPTY;
        for point in points {
            bounds.include_point(*point);
        }
        bounds
    }

    /// Expands this box to include `point`.
    pub fn include_point(&mut self, point: [f32; 3]) {
        for (axis, value) in point.into_iter().enumerate() {
            self.min[axis] = self.min[axis].min(value);
            self.max[axis] = self.max[axis].max(value);
        }
    }

    /// Expands this box to include `other`.
    pub fn include_aabb(&mut self, other: Self) {
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(other.min[axis]);
            self.max[axis] = self.max[axis].max(other.max[axis]);
        }
    }

    /// Returns `true` when two boxes overlap or touch.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.min[0] <= other.max[0]
            && self.max[0] >= other.min[0]
            && self.min[1] <= other.max[1]
            && self.max[1] >= other.min[1]
            && self.min[2] <= other.max[2]
            && self.max[2] >= other.min[2]
    }

    fn centroid(self) -> [f32; 3] {
        [
            0.5 * (self.min[0] + self.max[0]),
            0.5 * (self.min[1] + self.max[1]),
            0.5 * (self.min[2] + self.max[2]),
        ]
    }

    fn longest_axis(self) -> usize {
        let extent = [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ];
        if extent[1] > extent[0] && extent[1] >= extent[2] {
            1
        } else if extent[2] > extent[0] && extent[2] > extent[1] {
            2
        } else {
            0
        }
    }
}

/// Stable reference to one triangle enumerated from a mesh face.
///
/// `triangle_index` is the zero-based index into
/// [`Mesh::face_triangles`] for `face` under the [`FaceTriangulation`]
/// strategy the hierarchy was built with (recorded on [`BooleanBvh`] and in
/// [`BooleanBroadPhaseStats`]). A triangle index is meaningless without
/// that strategy; downstream narrow-phase stages must enumerate with the
/// same one.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BooleanTriangleRef {
    /// Source mesh face.
    pub face: FaceId,
    /// Zero-based triangle index within the source face's enumeration.
    pub triangle_index: u32,
}

/// Potentially overlapping triangle pair returned by the broad phase.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BooleanCandidatePair {
    /// Triangle from the left/input-A hierarchy.
    pub a: BooleanTriangleRef,
    /// Triangle from the right/input-B hierarchy.
    pub b: BooleanTriangleRef,
}

/// Deterministic broad-phase counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct BooleanBroadPhaseStats {
    /// Number of possible triangle pairs before broad-phase culling.
    pub total_pairs: u64,
    /// Number of pairs reported after AABB culling.
    pub candidate_pairs: u64,
    /// Triangle enumeration strategy the queried hierarchies were built
    /// with; triangle indices in the candidate pairs are only meaningful
    /// under this strategy.
    pub strategy: FaceTriangulation,
}

impl BooleanBroadPhaseStats {
    /// Fraction of possible pairs culled by the broad phase.
    ///
    /// Returns `0.0` when `total_pairs == 0`.
    #[must_use]
    pub fn reduction_ratio(self) -> f32 {
        if self.total_pairs == 0 {
            return 0.0;
        }
        1.0 - (self.candidate_pairs as f32 / self.total_pairs as f32)
    }
}

/// Cumulative scratch-reuse counters (introspection; tenet 4).
///
/// Counters accumulate across calls and survive [`BooleanScratch::clear`]
/// (which resets buffers, not the ledger); read with
/// [`BooleanScratch::counters`], reset with
/// [`BooleanScratch::reset_counters`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct BooleanScratchCounters {
    /// Face-triangle enumerations performed into reused buffers.
    pub face_enumerations: u64,
    /// Narrow-phase triangle fetches served from the per-mesh memo
    /// without re-enumerating.
    pub enumeration_cache_hits: u64,
}

/// Reusable temporary storage for the boolean pipeline (broad-phase
/// construction and queries, narrow-phase triangle staging).
///
/// `clear` retains capacity so callers can keep one scratch value per
/// session and avoid repeated temporary allocation across stages and
/// across boolean calls.
#[derive(Clone, Debug, Default)]
pub struct BooleanScratch {
    face_corners: Vec<CornerId>,
    node_pair_stack: Vec<(usize, usize)>,
    pub(super) face_triangles: Vec<[CornerId; 3]>,
    pub(super) narrow_face_a: Vec<[CornerId; 3]>,
    pub(super) narrow_face_b: Vec<[CornerId; 3]>,
    pub(super) counters: BooleanScratchCounters,
}

impl BooleanScratch {
    /// Creates empty boolean scratch storage.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            face_corners: Vec::new(),
            node_pair_stack: Vec::new(),
            face_triangles: Vec::new(),
            narrow_face_a: Vec::new(),
            narrow_face_b: Vec::new(),
            counters: BooleanScratchCounters {
                face_enumerations: 0,
                enumeration_cache_hits: 0,
            },
        }
    }

    /// Clears temporary buffers while retaining allocated capacity.
    /// Counters are cumulative and survive; see
    /// [`BooleanScratch::reset_counters`].
    pub fn clear(&mut self) {
        self.face_corners.clear();
        self.node_pair_stack.clear();
        self.face_triangles.clear();
        self.narrow_face_a.clear();
        self.narrow_face_b.clear();
    }

    /// Cumulative reuse counters.
    #[must_use]
    pub fn counters(&self) -> BooleanScratchCounters {
        self.counters
    }

    /// Resets the cumulative counters to zero.
    pub fn reset_counters(&mut self) {
        self.counters = BooleanScratchCounters::default();
    }
}

/// Owned boolean broad-phase hierarchy.
///
/// Rebuild an existing value with [`BooleanBvh::rebuild`] to retain allocations
/// across repeated boolean operations.
#[derive(Clone, Debug, Default)]
pub struct BooleanBvh {
    nodes: Vec<BvhNode>,
    triangles: Vec<TriangleEntry>,
    strategy: FaceTriangulation,
}

impl BooleanBvh {
    /// Creates an empty hierarchy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            strategy: FaceTriangulation::Fan,
            nodes: Vec::new(),
            triangles: Vec::new(),
        }
    }

    /// Builds a hierarchy for `mesh` under an explicit triangle
    /// enumeration strategy.
    #[must_use]
    pub fn build(mesh: &Mesh, strategy: FaceTriangulation, scratch: &mut BooleanScratch) -> Self {
        let mut bvh = Self::new();
        bvh.rebuild(mesh, strategy, scratch);
        bvh
    }

    /// Rebuilds this hierarchy for `mesh`, retaining existing capacity.
    pub fn rebuild(
        &mut self,
        mesh: &Mesh,
        strategy: FaceTriangulation,
        scratch: &mut BooleanScratch,
    ) {
        self.nodes.clear();
        self.triangles.clear();
        self.strategy = strategy;
        scratch.clear();

        collect_mesh_triangles(
            mesh,
            strategy,
            &mut self.triangles,
            &mut scratch.face_triangles,
            &mut scratch.counters,
        );
        if !self.triangles.is_empty() {
            let len = self.triangles.len();
            let _root = build_node(&mut self.triangles, &mut self.nodes, 0, len);
            debug_assert_eq!(_root, 0, "root node is inserted first");
        }
    }

    /// The triangle enumeration strategy this hierarchy was built with.
    #[must_use]
    pub fn strategy(&self) -> FaceTriangulation {
        self.strategy
    }

    /// Number of triangles in this hierarchy.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Number of hierarchy nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns `true` when this hierarchy has no triangles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Appends no data from previous calls, then writes overlapping candidate pairs.
    ///
    /// The output list is sorted by
    /// `(a.face, a.triangle_index, b.face, b.triangle_index)` for
    /// deterministic downstream processing. `scratch` is cleared at entry
    /// and used as traversal stack storage. Both hierarchies must have been
    /// built with the same [`FaceTriangulation`] strategy.
    pub fn query_overlaps(
        &self,
        other: &Self,
        scratch: &mut BooleanScratch,
        out: &mut Vec<BooleanCandidatePair>,
    ) -> BooleanBroadPhaseStats {
        debug_assert_eq!(
            self.strategy, other.strategy,
            "both hierarchies must use the same triangle enumeration strategy"
        );
        out.clear();
        scratch.node_pair_stack.clear();

        let total_pairs =
            usize_to_u64(self.triangles.len()).saturating_mul(usize_to_u64(other.triangles.len()));
        if self.nodes.is_empty() || other.nodes.is_empty() {
            return BooleanBroadPhaseStats {
                total_pairs,
                candidate_pairs: 0,
                strategy: self.strategy,
            };
        }

        scratch.node_pair_stack.push((0, 0));
        while let Some((a_index, b_index)) = scratch.node_pair_stack.pop() {
            let a = self.nodes[a_index];
            let b = other.nodes[b_index];
            if !a.bounds.overlaps(b.bounds) {
                continue;
            }

            match (a.is_leaf(), b.is_leaf()) {
                (true, true) => {
                    self.emit_leaf_pairs(other, a, b, out);
                }
                (true, false) => {
                    scratch.node_pair_stack.push((a_index, b.left));
                    scratch.node_pair_stack.push((a_index, b.right));
                }
                (false, true) => {
                    scratch.node_pair_stack.push((a.left, b_index));
                    scratch.node_pair_stack.push((a.right, b_index));
                }
                (false, false) => {
                    scratch.node_pair_stack.push((a.left, b.left));
                    scratch.node_pair_stack.push((a.left, b.right));
                    scratch.node_pair_stack.push((a.right, b.left));
                    scratch.node_pair_stack.push((a.right, b.right));
                }
            }
        }

        out.sort_unstable();
        BooleanBroadPhaseStats {
            total_pairs,
            candidate_pairs: usize_to_u64(out.len()),
            strategy: self.strategy,
        }
    }

    fn emit_leaf_pairs(
        &self,
        other: &Self,
        a: BvhNode,
        b: BvhNode,
        out: &mut Vec<BooleanCandidatePair>,
    ) {
        for a_triangle in &self.triangles[a.start..a.end()] {
            for b_triangle in &other.triangles[b.start..b.end()] {
                if a_triangle.bounds.overlaps(b_triangle.bounds) {
                    out.push(BooleanCandidatePair {
                        a: a_triangle.reference,
                        b: b_triangle.reference,
                    });
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
struct TriangleEntry {
    reference: BooleanTriangleRef,
    bounds: Aabb,
    centroid: [f32; 3],
}

#[derive(Copy, Clone, Debug, PartialEq)]
struct BvhNode {
    bounds: Aabb,
    start: usize,
    len: usize,
    left: usize,
    right: usize,
}

impl BvhNode {
    const EMPTY: Self = Self {
        bounds: Aabb::EMPTY,
        start: 0,
        len: 0,
        left: usize::MAX,
        right: usize::MAX,
    };

    fn leaf(bounds: Aabb, start: usize, len: usize) -> Self {
        Self {
            bounds,
            start,
            len,
            left: usize::MAX,
            right: usize::MAX,
        }
    }

    fn branch(bounds: Aabb, left: usize, right: usize) -> Self {
        Self {
            bounds,
            start: 0,
            len: 0,
            left,
            right,
        }
    }

    fn is_leaf(self) -> bool {
        self.left == usize::MAX
    }

    fn end(self) -> usize {
        self.start + self.len
    }
}

fn collect_mesh_triangles(
    mesh: &Mesh,
    strategy: FaceTriangulation,
    triangles: &mut Vec<TriangleEntry>,
    scratch_faces: &mut Vec<[CornerId; 3]>,
    counters: &mut BooleanScratchCounters,
) {
    for face in mesh.faces() {
        // The kernel-owned enumeration is the single source of truth for
        // triangle indices (ADR-0008); re-deriving fans inline here would
        // silently couple stored references to one strategy. The buffer
        // variant reuses one allocation across every face.
        let _ = mesh.face_triangles_into(face, strategy, scratch_faces);
        counters.face_enumerations += 1;
        for (triangle_index, corners) in scratch_faces.iter().copied().enumerate() {
            let bounds = triangle_bounds(mesh, corners);
            triangles.push(TriangleEntry {
                reference: BooleanTriangleRef {
                    face,
                    triangle_index: usize_to_u32(triangle_index),
                },
                bounds,
                centroid: bounds.centroid(),
            });
        }
    }
}

fn triangle_bounds(mesh: &Mesh, corners: [CornerId; 3]) -> Aabb {
    let mut points = [[0.0; 3]; 3];
    for (index, corner) in corners.into_iter().enumerate() {
        let vertex = mesh
            .to_vertex(corner)
            .expect("fan-triangulated corner must have a destination vertex");
        points[index] = *mesh
            .vertex_position(vertex)
            .expect("live mesh vertex must have a required position");
    }
    Aabb::from_points(&points)
}

fn build_node(
    triangles: &mut [TriangleEntry],
    nodes: &mut Vec<BvhNode>,
    start: usize,
    len: usize,
) -> usize {
    let node_index = nodes.len();
    nodes.push(BvhNode::EMPTY);

    let bounds = range_bounds(&triangles[start..start + len]);
    if len <= LEAF_TRIANGLE_COUNT {
        nodes[node_index] = BvhNode::leaf(bounds, start, len);
        return node_index;
    }

    let centroid_bounds = range_centroid_bounds(&triangles[start..start + len]);
    let axis = centroid_bounds.longest_axis();
    triangles[start..start + len].sort_unstable_by(|a, b| compare_triangle_axis(a, b, axis));

    let left_len = len / 2;
    let right_start = start + left_len;
    let right_len = len - left_len;
    let left = build_node(triangles, nodes, start, left_len);
    let right = build_node(triangles, nodes, right_start, right_len);
    nodes[node_index] = BvhNode::branch(bounds, left, right);
    node_index
}

fn range_bounds(triangles: &[TriangleEntry]) -> Aabb {
    let mut bounds = Aabb::EMPTY;
    for triangle in triangles {
        bounds.include_aabb(triangle.bounds);
    }
    bounds
}

fn range_centroid_bounds(triangles: &[TriangleEntry]) -> Aabb {
    let mut bounds = Aabb::EMPTY;
    for triangle in triangles {
        bounds.include_point(triangle.centroid);
    }
    bounds
}

fn compare_triangle_axis(a: &TriangleEntry, b: &TriangleEntry, axis: usize) -> Ordering {
    a.centroid[axis]
        .total_cmp(&b.centroid[axis])
        .then_with(|| a.reference.cmp(&b.reference))
}

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("fan triangle index overflowed u32")
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{Aabb, BooleanBvh, BooleanCandidatePair, BooleanScratch, BooleanTriangleRef};
    use crate::{BuildParams, FaceId, FaceTriangulation, Mesh};

    fn triangle_mesh(offset_x: f32) -> Mesh {
        Mesh::from_indexed_triangles(
            &[
                [offset_x, 0.0, 0.0],
                [offset_x + 1.0, 0.0, 0.0],
                [offset_x, 1.0, 0.0],
            ],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle mesh should build")
    }

    fn first_face(mesh: &Mesh) -> FaceId {
        mesh.faces().next().expect("mesh should have a face")
    }

    #[test]
    fn aabb_overlap_is_inclusive() {
        let a = Aabb::from_points(&[[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]);
        let b = Aabb::from_points(&[[1.0, 0.5, 0.5], [2.0, 1.5, 1.5]]);
        let c = Aabb::from_points(&[[2.0, 0.0, 0.0], [3.0, 1.0, 1.0]]);

        assert!(a.overlaps(b), "touching boxes should overlap");
        assert!(!a.overlaps(c), "separated boxes should not overlap");
    }

    #[test]
    fn bvh_builds_fan_triangles_for_polygon_faces() {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad should build");
        let mut scratch = BooleanScratch::new();
        let bvh = BooleanBvh::build(&mesh, FaceTriangulation::Fan, &mut scratch);

        assert_eq!(bvh.triangle_count(), 2);
        assert!(bvh.node_count() > 0);
    }

    #[test]
    fn broad_phase_records_its_enumeration_strategy() {
        let a = triangle_mesh(0.0);
        let b = triangle_mesh(0.25);
        let mut scratch = BooleanScratch::new();
        let a_bvh = BooleanBvh::build(&a, FaceTriangulation::Robust, &mut scratch);
        let b_bvh = BooleanBvh::build(&b, FaceTriangulation::Robust, &mut scratch);
        assert_eq!(a_bvh.strategy(), FaceTriangulation::Robust);

        let mut out = Vec::new();
        let stats = a_bvh.query_overlaps(&b_bvh, &mut scratch, &mut out);
        assert_eq!(stats.strategy, FaceTriangulation::Robust);
        assert!(stats.candidate_pairs > 0);

        // Triangle-per-triangle meshes enumerate identically under both
        // strategies, so candidates match the fan build exactly.
        let a_fan = BooleanBvh::build(&a, FaceTriangulation::Fan, &mut scratch);
        let b_fan = BooleanBvh::build(&b, FaceTriangulation::Fan, &mut scratch);
        let mut fan_out = Vec::new();
        let fan_stats = a_fan.query_overlaps(&b_fan, &mut scratch, &mut fan_out);
        assert_eq!(out, fan_out);
        assert_eq!(fan_stats.strategy, FaceTriangulation::Fan);
    }

    #[test]
    fn broad_phase_reports_no_candidates_for_disjoint_meshes() {
        let a = triangle_mesh(0.0);
        let b = triangle_mesh(10.0);
        let mut scratch = BooleanScratch::new();
        let a_bvh = BooleanBvh::build(&a, FaceTriangulation::Fan, &mut scratch);
        let b_bvh = BooleanBvh::build(&b, FaceTriangulation::Fan, &mut scratch);
        let mut candidates = Vec::new();

        let stats = a_bvh.query_overlaps(&b_bvh, &mut scratch, &mut candidates);

        assert_eq!(stats.total_pairs, 1);
        assert_eq!(stats.candidate_pairs, 0);
        assert_eq!(stats.reduction_ratio(), 1.0);
        assert!(candidates.is_empty());
    }

    #[test]
    fn broad_phase_reports_overlapping_triangle_pair() {
        let a = triangle_mesh(0.0);
        let b = triangle_mesh(0.25);
        let a_face = first_face(&a);
        let b_face = first_face(&b);
        let mut scratch = BooleanScratch::new();
        let a_bvh = BooleanBvh::build(&a, FaceTriangulation::Fan, &mut scratch);
        let b_bvh = BooleanBvh::build(&b, FaceTriangulation::Fan, &mut scratch);
        let mut candidates = Vec::new();

        let stats = a_bvh.query_overlaps(&b_bvh, &mut scratch, &mut candidates);

        assert_eq!(stats.total_pairs, 1);
        assert_eq!(stats.candidate_pairs, 1);
        assert_eq!(stats.reduction_ratio(), 0.0);
        assert_eq!(
            candidates,
            vec![BooleanCandidatePair {
                a: BooleanTriangleRef {
                    face: a_face,
                    triangle_index: 0,
                },
                b: BooleanTriangleRef {
                    face: b_face,
                    triangle_index: 0,
                },
            }]
        );
    }

    #[test]
    fn broad_phase_orders_candidates_deterministically() {
        let a = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [1, 3, 2]],
            &BuildParams::default(),
        )
        .expect("two-triangle mesh should build");
        let b = triangle_mesh(0.25);
        let a_faces = a.faces().collect::<Vec<_>>();
        let b_face = first_face(&b);
        let mut scratch = BooleanScratch::new();
        let a_bvh = BooleanBvh::build(&a, FaceTriangulation::Fan, &mut scratch);
        let b_bvh = BooleanBvh::build(&b, FaceTriangulation::Fan, &mut scratch);
        let mut candidates = Vec::new();

        let stats = a_bvh.query_overlaps(&b_bvh, &mut scratch, &mut candidates);

        assert_eq!(stats.total_pairs, 2);
        assert_eq!(stats.candidate_pairs, 2);
        assert_eq!(
            candidates,
            vec![
                BooleanCandidatePair {
                    a: BooleanTriangleRef {
                        face: a_faces[0],
                        triangle_index: 0,
                    },
                    b: BooleanTriangleRef {
                        face: b_face,
                        triangle_index: 0,
                    },
                },
                BooleanCandidatePair {
                    a: BooleanTriangleRef {
                        face: a_faces[1],
                        triangle_index: 0,
                    },
                    b: BooleanTriangleRef {
                        face: b_face,
                        triangle_index: 0,
                    },
                },
            ]
        );
    }

    #[test]
    fn query_clears_output_and_reuses_scratch() {
        let a = triangle_mesh(0.0);
        let b = triangle_mesh(0.25);
        let mut scratch = BooleanScratch::new();
        let a_bvh = BooleanBvh::build(&a, FaceTriangulation::Fan, &mut scratch);
        let b_bvh = BooleanBvh::build(&b, FaceTriangulation::Fan, &mut scratch);
        let mut candidates = Vec::with_capacity(4);
        candidates.push(BooleanCandidatePair {
            a: BooleanTriangleRef {
                face: FaceId::OUTSIDE,
                triangle_index: 999,
            },
            b: BooleanTriangleRef {
                face: FaceId::OUTSIDE,
                triangle_index: 999,
            },
        });
        let old_capacity = candidates.capacity();

        let stats = a_bvh.query_overlaps(&b_bvh, &mut scratch, &mut candidates);

        assert_eq!(stats.candidate_pairs, 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates.capacity(), old_capacity);
    }
}
