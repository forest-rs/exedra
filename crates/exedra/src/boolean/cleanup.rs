// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Post-stitch seam cleanup: sliver removal along boolean cut rings.
//!
//! Boolean outputs concentrate low-quality triangles along their seam
//! rings — cut-loop vertices are f32-narrowed f64 constructions, and the
//! exactly-collinear rim vertices reinserted by drilled-face re-facing
//! produce zero-or-near-zero-area triangles hugging the rings. This pass
//! removes them with the kernel's [`crate::op::collapse_edge`] and
//! [`crate::op::flip_edge`] surgery under explicit, geometry-conservative
//! guards:
//!
//! - **No vertex ever moves.** Collapse keeps the surviving vertex's
//!   authored position; flips rewire connectivity only. Volume drift is
//!   therefore local and bounded, and the pass tracks it against an
//!   explicit budget.
//! - **Seam rings stay closed and in place.** A collapse along a seam
//!   edge shortens its ring (dropping a near-collinear rim vertex); a
//!   collapse that would pull a seam vertex off its ring, bridge two
//!   rings, or erase a seam edge by flipping it is skipped and counted.
//! - **`FACE_REGION` fidelity is preserved.** A dropped triangle's region
//!   must survive on an adjacent face, and flips never cross a region
//!   boundary.
//! - **Refusal over degradation.** Every skipped candidate is counted by
//!   reason; kernel precondition failures leave the mesh byte-identical.
//!
//! Determinism: candidate faces are visited in ascending face-id order
//! within bounded rounds, every geometric decision is plain f64
//! arithmetic over exactly promoted f32 positions, flips require strict
//! quality improvement (no cycles), and the total op count is budgeted.

use alloc::vec::Vec;

use crate::op::{collapse_edge, flip_edge};
use crate::{FaceId, HalfEdgeId, Mesh, VertexId, attr};

/// Explicit thresholds for [`cleanup_seams`]. No hidden constants: every
/// decision the pass makes is derived from these fields.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SeamCleanupPolicy {
    /// A triangle is a needle when its shortest edge is below this
    /// fraction of its longest edge; the shortest edge collapses.
    pub needle_ratio: f64,
    /// A triangle is a cap sliver when its area is below this fraction of
    /// its squared longest edge (an equilateral triangle measures ~0.433);
    /// its longest edge flips when quality strictly improves. Compared in
    /// squared form, so the pass needs no square roots.
    pub sliver_quality: f64,
    /// Cumulative absolute volume drift allowed across the pass, as a
    /// fraction of the mesh's initial absolute volume (falling back to the
    /// cubed largest bounding-box extent for volume-less inputs).
    pub relative_volume_budget: f64,
    /// Hard cap on applied kernel ops (collapses + flips).
    pub max_ops: u32,
    /// Hard cap on sweep rounds; each round revisits surviving faces.
    pub max_rounds: u32,
    /// Restrict candidates to faces touching a seam vertex (the pass's
    /// namesake scope). `false` sweeps every triangle in the mesh.
    pub seam_scope: bool,
}

impl Default for SeamCleanupPolicy {
    fn default() -> Self {
        Self {
            needle_ratio: 1e-3,
            sliver_quality: 1e-3,
            relative_volume_budget: 1e-9,
            max_ops: 10_000,
            max_rounds: 8,
            seam_scope: true,
        }
    }
}

/// Introspection counters returned by [`cleanup_seams`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SeamCleanupStats {
    /// Sweep rounds executed.
    pub rounds: u32,
    /// Candidate sliver triangles considered.
    pub candidates: u64,
    /// Edge collapses applied.
    pub collapses: u64,
    /// Edge flips applied.
    pub flips: u64,
    /// Candidates skipped by a kernel precondition (mesh unchanged).
    pub skipped_kernel: u64,
    /// Candidates skipped to keep seam rings closed and in place.
    pub skipped_seam_guard: u64,
    /// Candidates skipped to preserve `FACE_REGION` fidelity.
    pub skipped_region_guard: u64,
    /// Candidates skipped because the volume budget would be exceeded.
    pub skipped_budget: u64,
    /// Flip candidates skipped for lack of strict quality improvement or
    /// orientation agreement.
    pub skipped_quality_guard: u64,
    /// Cumulative signed volume drift applied (fan-volume metric).
    pub volume_drift: f64,
    /// Cumulative absolute volume drift applied (budgeted quantity).
    pub volume_drift_abs: f64,
}

/// Cleans sliver triangles along a boolean output's seam rings with
/// kernel collapse/flip surgery under geometry-conservative guards.
///
/// See the module documentation for the guard semantics; see
/// [`SeamCleanupPolicy`] for every threshold. The pass never moves a
/// vertex, keeps seam rings closed, preserves `FACE_REGION` fidelity,
/// and skips (and counts) anything it cannot improve safely — a mesh
/// with no slivers passes through untouched.
pub fn cleanup_seams(mesh: &mut Mesh, policy: &SeamCleanupPolicy) -> SeamCleanupStats {
    let mut stats = SeamCleanupStats::default();
    let budget = drift_budget(mesh, policy);
    let mut ops = 0_u32;

    for _ in 0..policy.max_rounds {
        stats.rounds += 1;
        let seam_vertices = mark_seam_vertices(mesh);
        let faces: Vec<FaceId> = mesh.faces().collect();
        let mut progressed = false;

        for face in faces {
            if ops >= policy.max_ops {
                return stats;
            }
            // The face may have died to an earlier collapse this round.
            let loop_edges: Vec<HalfEdgeId> = mesh.face_loop(face).collect();
            if loop_edges.len() != 3 {
                continue;
            }
            let Some(corners) = triangle_corners(mesh, &loop_edges) else {
                continue;
            };
            if policy.seam_scope
                && !corners
                    .iter()
                    .any(|&(v, _)| seam_vertices.get(v.index() as usize) == Some(&true))
            {
                continue;
            }

            let Some(shape) = TriangleShape::measure(&corners) else {
                continue;
            };
            let is_needle =
                shape.min_edge_sq < policy.needle_ratio * policy.needle_ratio * shape.max_edge_sq;
            let threshold = policy.sliver_quality * shape.max_edge_sq;
            let is_cap = shape.area_sq < threshold * threshold;
            if !is_needle && !is_cap {
                continue;
            }
            stats.candidates += 1;

            let applied = if is_needle {
                try_collapse(
                    mesh,
                    loop_edges[shape.min_edge],
                    &seam_vertices,
                    budget,
                    &mut stats,
                )
            } else {
                try_flip(mesh, loop_edges[shape.max_edge], budget, &mut stats)
            };
            if applied {
                ops += 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    stats
}

/// One triangle corner: the vertex a loop half-edge leaves from, with its
/// promoted position.
type Corner = (VertexId, [f64; 3]);

fn triangle_corners(mesh: &Mesh, loop_edges: &[HalfEdgeId]) -> Option<[Corner; 3]> {
    let corner = |half_edge: HalfEdgeId| -> Option<Corner> {
        let vertex = mesh.from_vertex(half_edge)?;
        let position = mesh.vertex_position(vertex)?;
        Some((vertex, promote(*position)))
    };
    Some([
        corner(loop_edges[0])?,
        corner(loop_edges[1])?,
        corner(loop_edges[2])?,
    ])
}

/// Squared edge lengths and squared area of one triangle, with the
/// extremal edge slots (edge `i` runs from corner `i` to corner `i + 1`).
struct TriangleShape {
    min_edge: usize,
    max_edge: usize,
    min_edge_sq: f64,
    max_edge_sq: f64,
    area_sq: f64,
}

impl TriangleShape {
    fn measure(corners: &[Corner; 3]) -> Option<Self> {
        let mut lengths = [0.0_f64; 3];
        for (i, slot) in lengths.iter_mut().enumerate() {
            let (_, a) = corners[i];
            let (_, b) = corners[(i + 1) % 3];
            *slot = distance_sq(a, b);
        }
        let (mut min_edge, mut max_edge) = (0, 0);
        for i in 1..3 {
            if lengths[i] < lengths[min_edge] {
                min_edge = i;
            }
            if lengths[i] > lengths[max_edge] {
                max_edge = i;
            }
        }
        if lengths[max_edge] <= 0.0 || !lengths[max_edge].is_finite() {
            return None;
        }
        let area_sq = triangle_area_sq(corners[0].1, corners[1].1, corners[2].1);
        Some(Self {
            min_edge,
            max_edge,
            min_edge_sq: lengths[min_edge],
            max_edge_sq: lengths[max_edge],
            area_sq,
        })
    }
}

/// Marks every vertex incident to a seam-tagged edge. Indexed by vertex
/// index; used only for membership, so no iteration-order dependence.
fn mark_seam_vertices(mesh: &Mesh) -> Vec<bool> {
    let mut marked = Vec::new();
    let mut mark = |vertex: VertexId| {
        let index = vertex.index() as usize;
        if marked.len() <= index {
            marked.resize(index + 1, false);
        }
        marked[index] = true;
    };
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            if mesh.edge_seam(half_edge) == Some(true)
                && let (Some(from), Some(to)) =
                    (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
            {
                mark(from);
                mark(to);
            }
        }
    }
    marked
}

fn try_collapse(
    mesh: &mut Mesh,
    half_edge: HalfEdgeId,
    seam_vertices: &[bool],
    budget: f64,
    stats: &mut SeamCleanupStats,
) -> bool {
    let (Some(from), Some(to)) = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge)) else {
        return false;
    };
    // Mirror the kernel's deterministic survivor rule (smaller id wins).
    let (keep, remove) = if from.index() <= to.index() {
        (from, to)
    } else {
        (to, from)
    };

    // Seam guard: a seam edge may collapse (its ring shortens but stays
    // closed, and both endpoints already lie on the ring). Otherwise a
    // seam vertex must never be pulled off its ring: two seam endpoints
    // would bridge or pinch rings, and a removed seam endpoint would drag
    // its ring to the survivor's position.
    let on_seam = |vertex: VertexId| seam_vertices.get(vertex.index() as usize) == Some(&true);
    if mesh.edge_seam(half_edge) != Some(true) && on_seam(remove) {
        // Keeping a seam survivor in place is fine; removing one is not.
        stats.skipped_seam_guard += 1;
        return false;
    }

    // Region guard: each triangle dropped by the collapse must leave its
    // region represented on an adjacent surviving face.
    if !dropped_regions_survive(mesh, half_edge) {
        stats.skipped_region_guard += 1;
        return false;
    }

    // Volume budget: the collapse only reshapes faces incident to the
    // removed vertex; substituting the survivor's position into their
    // loops gives the exact post-op fan volume.
    let delta = collapse_volume_delta(mesh, keep, remove);
    if !delta.is_finite() || stats.volume_drift_abs + delta.abs() > budget {
        stats.skipped_budget += 1;
        return false;
    }

    let mut session = mesh.edit();
    let applied = collapse_edge(&mut session, half_edge).is_ok();
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
    if applied {
        stats.collapses += 1;
        stats.volume_drift += delta;
        stats.volume_drift_abs += delta.abs();
    } else {
        stats.skipped_kernel += 1;
    }
    applied
}

fn try_flip(
    mesh: &mut Mesh,
    half_edge: HalfEdgeId,
    budget: f64,
    stats: &mut SeamCleanupStats,
) -> bool {
    // Seam guard: flipping a seam edge would erase it from its ring.
    if mesh.edge_seam(half_edge) == Some(true) {
        stats.skipped_seam_guard += 1;
        return false;
    }
    let Some(twin) = mesh.twin(half_edge) else {
        return false;
    };
    let (Some(face), Some(twin_face)) = (mesh.face(half_edge), mesh.face(twin)) else {
        return false;
    };
    if face == FaceId::OUTSIDE || twin_face == FaceId::OUTSIDE {
        return false;
    }
    // Region guard: flips shift covered area across the diagonal, so both
    // triangles must belong to the same region.
    if face_region(mesh, face) != face_region(mesh, twin_face) {
        stats.skipped_region_guard += 1;
        return false;
    }

    // The quad around the edge: a = from, b = to, c opposite in `face`,
    // d opposite in `twin_face` (both triangles required by the kernel).
    let quad = (|| {
        let a = mesh.from_vertex(half_edge)?;
        let b = mesh.to_vertex(half_edge)?;
        let c = mesh.to_vertex(mesh.next(half_edge)?)?;
        let d = mesh.to_vertex(mesh.next(twin)?)?;
        let point = |v: VertexId| mesh.vertex_position(v).map(|p| promote(*p));
        Some([point(a)?, point(b)?, point(c)?, point(d)?])
    })();
    let Some([a, b, c, d]) = quad else {
        return false;
    };

    // Quality guard: the flipped pair's worst quality must strictly beat
    // the current pair's, and both new triangles must agree with the
    // region's current orientation (no fold-over).
    let old_worst = triangle_quality(a, b, c).min(triangle_quality(b, a, d));
    let new_worst = triangle_quality(c, d, b).min(triangle_quality(d, c, a));
    // Strict improvement required (NaN or equal both refuse).
    if new_worst.partial_cmp(&old_worst) != Some(core::cmp::Ordering::Greater) {
        stats.skipped_quality_guard += 1;
        return false;
    }
    let reference = add(normal(a, b, c), normal(b, a, d));
    if dot(normal(c, d, b), reference) <= 0.0 || dot(normal(d, c, a), reference) <= 0.0 {
        stats.skipped_quality_guard += 1;
        return false;
    }

    // Volume budget: the flip replaces two fan triangles.
    let delta =
        (signed_tet(c, d, b) + signed_tet(d, c, a)) - (signed_tet(a, b, c) + signed_tet(b, a, d));
    if !delta.is_finite() || stats.volume_drift_abs + delta.abs() > budget {
        stats.skipped_budget += 1;
        return false;
    }

    let mut session = mesh.edit();
    let applied = flip_edge(&mut session, half_edge).is_ok();
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
    if applied {
        stats.flips += 1;
        stats.volume_drift += delta;
        stats.volume_drift_abs += delta.abs();
    } else {
        stats.skipped_kernel += 1;
    }
    applied
}

/// True when every triangle the collapse would drop leaves its
/// `FACE_REGION` represented on an adjacent surviving interior face.
fn dropped_regions_survive(mesh: &Mesh, half_edge: HalfEdgeId) -> bool {
    let Some(twin) = mesh.twin(half_edge) else {
        return false;
    };
    for on_edge in [half_edge, twin] {
        let Some(face) = mesh.face(on_edge) else {
            continue;
        };
        if face == FaceId::OUTSIDE || mesh.face_loop(face).count() != 3 {
            continue; // Only triangles on the edge are dropped.
        }
        let region = face_region(mesh, face);
        if region.is_none() {
            continue;
        }
        let mut survives = false;
        for side in mesh.face_loop(face) {
            if mesh.canonical_edge(side) == mesh.canonical_edge(on_edge) {
                continue; // The collapsing edge's other side also drops.
            }
            let neighbor = mesh
                .twin(side)
                .and_then(|t| mesh.face(t))
                .filter(|&f| f != FaceId::OUTSIDE && f != face);
            if let Some(neighbor) = neighbor {
                let neighbor_drops =
                    mesh.face_loop(neighbor).count() == 3 && face_on_edge(mesh, neighbor, on_edge);
                if !neighbor_drops && face_region(mesh, neighbor) == region {
                    survives = true;
                    break;
                }
            }
        }
        if !survives {
            return false;
        }
    }
    true
}

/// True when `face` contains the same undirected edge as `on_edge` (both
/// faces on the collapsing edge drop together).
fn face_on_edge(mesh: &Mesh, face: FaceId, on_edge: HalfEdgeId) -> bool {
    let canonical = mesh.canonical_edge(on_edge);
    mesh.face_loop(face)
        .any(|h| mesh.canonical_edge(h) == canonical)
}

fn face_region(mesh: &Mesh, face: FaceId) -> Option<u32> {
    mesh.attrs()
        .dense(attr::FACE_REGION)
        .and_then(|layer| layer.get(face.as_id()).copied())
}

/// Exact (per the fan-volume metric) volume change of collapsing
/// `remove` into `keep`: only faces incident to `remove` reshape, and
/// substituting the survivor's position reproduces their post-op loops
/// (dropped triangles degenerate to zero).
fn collapse_volume_delta(mesh: &Mesh, keep: VertexId, remove: VertexId) -> f64 {
    let keep_position = mesh.vertex_position(keep).map(|p| promote(*p));
    let Some(keep_position) = keep_position else {
        return f64::NAN;
    };
    let mut delta = 0.0;
    for face in mesh.faces() {
        let loop_vertices: Vec<VertexId> = mesh
            .face_loop(face)
            .filter_map(|h| mesh.from_vertex(h))
            .collect();
        if !loop_vertices.contains(&remove) {
            continue;
        }
        let before: Vec<[f64; 3]> = loop_vertices
            .iter()
            .filter_map(|&v| mesh.vertex_position(v).map(|p| promote(*p)))
            .collect();
        if before.len() != loop_vertices.len() {
            return f64::NAN;
        }
        let after: Vec<[f64; 3]> = loop_vertices
            .iter()
            .zip(&before)
            .map(|(&v, &p)| if v == remove { keep_position } else { p })
            .collect();
        delta += fan_volume(&after) - fan_volume(&before);
    }
    delta
}

/// The pass's absolute drift allowance: relative to the mesh's absolute
/// fan volume, falling back to the cubed bounding-box diagonal for
/// volume-less (open or empty) inputs.
fn drift_budget(mesh: &Mesh, policy: &SeamCleanupPolicy) -> f64 {
    let mut volume = 0.0;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for face in mesh.faces() {
        let corners: Vec<[f64; 3]> = mesh
            .face_loop(face)
            .filter_map(|h| mesh.from_vertex(h))
            .filter_map(|v| mesh.vertex_position(v).map(|p| promote(*p)))
            .collect();
        volume += fan_volume(&corners);
        for corner in corners {
            for axis in 0..3 {
                min[axis] = min[axis].min(corner[axis]);
                max[axis] = max[axis].max(corner[axis]);
            }
        }
    }
    let scale = if volume.abs() > 0.0 {
        volume.abs()
    } else {
        let extent = (0..3)
            .map(|axis| (max[axis] - min[axis]).max(0.0))
            .fold(0.0_f64, f64::max);
        extent * extent * extent
    };
    policy.relative_volume_budget * scale
}

fn promote(p: [f32; 3]) -> [f64; 3] {
    [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
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

fn distance_sq(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = sub(b, a);
    dot(d, d)
}

fn normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    cross(sub(b, a), sub(c, a))
}

fn triangle_area_sq(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let n = normal(a, b, c);
    0.25 * dot(n, n)
}

/// Squared normalized triangle quality: `(area / longest_edge_sq)^2`.
/// Squaring is monotone on the nonnegative quality, so comparisons are
/// unchanged and the pass needs no square roots.
fn triangle_quality(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let longest = distance_sq(a, b)
        .max(distance_sq(b, c))
        .max(distance_sq(c, a));
    if longest <= 0.0 || !longest.is_finite() {
        return 0.0;
    }
    triangle_area_sq(a, b, c) / (longest * longest)
}

/// Signed volume contribution of one origin-apex tetrahedron.
fn signed_tet(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    dot(a, cross(b, c)) / 6.0
}

/// Signed fan volume contribution of one face loop.
fn fan_volume(corners: &[[f64; 3]]) -> f64 {
    let mut volume = 0.0;
    for i in 1..corners.len().saturating_sub(1) {
        volume += signed_tet(corners[0], corners[i], corners[i + 1]);
    }
    volume
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::{BooleanDiagnostics, BooleanOp, BooleanScratch, boolean_mesh};
    use crate::{FaceTriangulation, MeshBuilder};

    /// The 4 x 4 x 1 slab from the stitch drill tests.
    fn slab() -> Mesh {
        let positions = [
            [-2.0, -2.0, -0.5],
            [2.0, -2.0, -0.5],
            [2.0, 2.0, -0.5],
            [-2.0, 2.0, -0.5],
            [-2.0, -2.0, 0.5],
            [2.0, -2.0, 0.5],
            [2.0, 2.0, 0.5],
            [-2.0, 2.0, 0.5],
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
            builder.add_face(&face).expect("valid slab face");
        }
        builder.build().expect("valid slab").mesh
    }

    /// A 16-gon drill prism (radius 0.8, through the slab). Its wall
    /// quads' fan diagonals cross the cap planes at exact chord midpoints,
    /// so the cut loops carry near-collinear triples — the re-faced caps
    /// come out full of the rim slivers this pass targets.
    fn sliver_drill() -> Mesh {
        let n = 16_u32;
        let mut builder = MeshBuilder::new();
        for z in [-1.5_f64, 1.5] {
            for i in 0..n {
                let angle = core::f64::consts::TAU * f64::from(i) / f64::from(n);
                let position = [0.8 * angle.cos(), 0.8 * angle.sin(), z];
                #[expect(clippy::cast_possible_truncation, reason = "test geometry narrowing")]
                builder.push_vertex([position[0] as f32, position[1] as f32, position[2] as f32]);
            }
        }
        let bottom: Vec<u32> = (0..n).rev().collect();
        builder.add_face(&bottom).expect("bottom cap");
        let top: Vec<u32> = (n..2 * n).collect();
        builder.add_face(&top).expect("top cap");
        for i in 0..n {
            let j = (i + 1) % n;
            builder.add_face(&[i, j, n + j, n + i]).expect("side wall");
        }
        builder.build().expect("valid prism").mesh
    }

    fn drilled_difference() -> Mesh {
        let mut scratch = BooleanScratch::new();
        let mut diagnostics = BooleanDiagnostics::default();
        let output = boolean_mesh(
            &slab(),
            &sliver_drill(),
            BooleanOp::Difference,
            FaceTriangulation::Fan,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("drill boolean succeeds");
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        output.mesh
    }

    /// Two unit cubes sharing a full face: a clean union with no slivers.
    fn touching_union() -> Mesh {
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
        let mut scratch = BooleanScratch::new();
        let mut diagnostics = BooleanDiagnostics::default();
        boolean_mesh(
            &cube([0.0, 0.0, 0.0]),
            &cube([1.0, 0.0, 0.0]),
            BooleanOp::Union,
            FaceTriangulation::Fan,
            &mut scratch,
            &mut diagnostics,
        )
        .expect("touching union succeeds")
        .mesh
    }

    fn signed_volume(mesh: &Mesh) -> f64 {
        let mut volume = 0.0;
        for face in mesh.faces() {
            let corners: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|h| mesh.from_vertex(h))
                .filter_map(|v| mesh.vertex_position(v).map(|p| promote(*p)))
                .collect();
            volume += fan_volume(&corners);
        }
        volume
    }

    fn euler_characteristic(mesh: &Mesh) -> i64 {
        let vertices = i64::try_from(mesh.vertices().count()).expect("small");
        let faces = i64::try_from(mesh.faces().count()).expect("small");
        let half_edges: usize = mesh.faces().map(|face| mesh.face_loop(face).count()).sum();
        let edges = i64::try_from(half_edges).expect("small") / 2;
        vertices - edges + faces
    }

    /// Worst (squared) triangle quality over the mesh's triangle faces.
    fn worst_quality(mesh: &Mesh) -> f64 {
        let mut worst = f64::INFINITY;
        for face in mesh.faces() {
            let loop_edges: Vec<HalfEdgeId> = mesh.face_loop(face).collect();
            if loop_edges.len() != 3 {
                continue;
            }
            let Some(corners) = triangle_corners(mesh, &loop_edges) else {
                continue;
            };
            worst = worst.min(triangle_quality(corners[0].1, corners[1].1, corners[2].1));
        }
        worst
    }

    /// Undirected seam-edge degree per vertex: 2 everywhere on closed rings.
    fn seam_ring_degrees_are_two(mesh: &Mesh) -> bool {
        let mut degrees: alloc::collections::BTreeMap<u32, u32> =
            alloc::collections::BTreeMap::new();
        for face in mesh.faces() {
            for half_edge in mesh.face_loop(face) {
                if mesh.edge_seam(half_edge) != Some(true) {
                    continue;
                }
                let Some(from) = mesh.from_vertex(half_edge) else {
                    return false;
                };
                // Each undirected seam edge visits here twice (once per
                // half-edge), so counting `from` incidences covers both
                // endpoints exactly once each.
                *degrees.entry(from.index()).or_insert(0) += 1;
            }
        }
        !degrees.is_empty() && degrees.values().all(|&d| d == 2)
    }

    type Snapshot = (Vec<Vec<u32>>, Vec<[u32; 3]>);

    fn snapshot(mesh: &Mesh) -> Snapshot {
        let faces: Vec<Vec<u32>> = mesh
            .faces()
            .map(|face| {
                mesh.face_loop(face)
                    .filter_map(|h| mesh.to_vertex(h))
                    .map(|v| v.index())
                    .collect()
            })
            .collect();
        let positions: Vec<[u32; 3]> = mesh
            .vertices()
            .filter_map(|v| mesh.vertex_position(v))
            .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
            .collect();
        (faces, positions)
    }

    #[test]
    fn drill_cleanup_improves_worst_quality_within_budget() {
        let policy = SeamCleanupPolicy::default();
        let mut mesh = drilled_difference();
        let before = worst_quality(&mesh);
        let volume_before = signed_volume(&mesh);
        assert!(before < 1e-12, "the drill output must carry rim slivers");
        assert_eq!(euler_characteristic(&mesh), 0, "through-hole shell");
        assert!(seam_ring_degrees_are_two(&mesh), "closed seam rings");

        let stats = cleanup_seams(&mut mesh, &policy);

        assert!(stats.collapses + stats.flips > 0, "{stats:?}");
        let after = worst_quality(&mesh);
        assert!(
            after > before && after > 1e-6,
            "worst quality {before:e} -> {after:e}"
        );
        let volume_after = signed_volume(&mesh);
        let budget = policy.relative_volume_budget * volume_before.abs();
        assert!(
            (volume_after - volume_before).abs() <= budget,
            "volume {volume_before} -> {volume_after} exceeds budget {budget:e}"
        );
        assert!(stats.volume_drift_abs <= budget, "{stats:?}");
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(euler_characteristic(&mesh), 0, "genus preserved");
        assert!(seam_ring_degrees_are_two(&mesh), "seam rings stay closed");
    }

    #[test]
    fn clean_output_passes_through_untouched() {
        let mut mesh = touching_union();
        let before = snapshot(&mesh);
        let stats = cleanup_seams(&mut mesh, &SeamCleanupPolicy::default());
        assert_eq!(stats.collapses, 0, "{stats:?}");
        assert_eq!(stats.flips, 0, "{stats:?}");
        assert_eq!(snapshot(&mesh), before, "no-op leaves the mesh unchanged");
    }

    #[test]
    fn zero_budget_admits_only_zero_drift_ops() {
        let mut mesh = drilled_difference();
        let volume_before = signed_volume(&mesh);
        let stats = cleanup_seams(
            &mut mesh,
            &SeamCleanupPolicy {
                relative_volume_budget: 0.0,
                ..SeamCleanupPolicy::default()
            },
        );
        // Ops with exactly zero drift are free under any budget; anything
        // that would move volume at all is refused and counted.
        assert!(stats.skipped_budget > 0, "{stats:?}");
        assert_eq!(stats.volume_drift_abs, 0.0, "{stats:?}");
        assert_eq!(
            signed_volume(&mesh).to_bits(),
            volume_before.to_bits(),
            "zero budget keeps the volume bit-identical"
        );
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn cleanup_is_deterministic() {
        let policy = SeamCleanupPolicy::default();
        let mut first = drilled_difference();
        let mut second = drilled_difference();
        let stats_first = cleanup_seams(&mut first, &policy);
        let stats_second = cleanup_seams(&mut second, &policy);
        assert_eq!(stats_first, stats_second);
        assert_eq!(snapshot(&first), snapshot(&second));
    }
}
