// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Coplanar face-on-face contact detection for boolean operations.
//!
//! Touching solids meet along exactly coplanar faces. The narrow phase
//! classifies those triangle pairs as coplanar and skips them (counting
//! them in its stats); this stage re-examines the broad-phase candidates
//! at face level, keeps the coplanar face pairs whose 2D overlap has
//! positive area, and records for each such contact the shared-plane
//! geometry that patch classification consumes later.
//!
//! The stage constructs nothing: every decision is an exact predicate —
//! [`orient3d`] for coplanarity and outward-normal agreement, [`orient2d`]
//! for the separating-axis overlap test in the dominant-axis projection.
//! The contact region's boundary curves are not built here either: they
//! already arise in the narrow phase as transversal segments where the
//! neighboring (non-coplanar) faces of one solid meet the contact plane
//! of the other, so splitting carves the contact region out of both
//! meshes through the ordinary intersection-graph machinery.
//!
//! Zero-area coplanar overlaps (faces sharing only an edge or a vertex in
//! the plane) are fully understood and contribute nothing; they are
//! skipped without a diagnostic. Configurations outside the envelope
//! (non-planar faces hiding coplanar triangles, degenerate orientation
//! probes) are typed [`BooleanFailureKind::CoplanarAmbiguity`] deferrals —
//! reported, never guessed.
//!
//! Determinism: candidate face pairs process in ascending
//! `(face_a, face_b)` index order, and contacts are emitted in that order.

use alloc::vec::Vec;

use exedra_triangulate::predicates::{Orientation, Orientation3d, orient2d, orient3d};

use super::diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
use super::{BooleanCandidatePair, BooleanScratch, BooleanTriangleRef};
use crate::{CornerId, FaceId, FaceTriangulation, Mesh};
use exedra_math::{promote, sub};

/// One coplanar face-on-face contact with positive-area overlap.
///
/// `polygon_a`/`polygon_b` are the pre-split face loops projected into the
/// shared plane along `axis` (exact coordinate selection over exactly
/// promoted positions), captured before splitting mutates the meshes so
/// classification can test post-split faces against them.
#[derive(Clone, Debug, PartialEq)]
pub struct CoplanarContact {
    /// The pre-split face of mesh A.
    pub face_a: FaceId,
    /// The pre-split face of mesh B.
    pub face_b: FaceId,
    /// True when the faces' outward normals oppose (typical touching
    /// solids); false when they agree (flush overlapping boundaries).
    pub opposed: bool,
    /// Dominant axis of the shared plane's normal; projection drops this
    /// coordinate and keeps the other two in cyclic order.
    pub axis: usize,
    /// Face A's loop projected into the shared plane.
    pub polygon_a: Vec<[f64; 2]>,
    /// Face B's loop projected into the shared plane.
    pub polygon_b: Vec<[f64; 2]>,
}

/// Deterministic contact-detection counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct CoplanarStats {
    /// Unique candidate face pairs examined.
    pub face_pairs: u64,
    /// Face pairs found exactly coplanar.
    pub coplanar_face_pairs: u64,
    /// Positive-area contacts recorded.
    pub contacts: u64,
    /// Coplanar face pairs with zero-area (edge/vertex) overlap, skipped
    /// as fully handled.
    pub grazing_pairs: u64,
    /// Face pairs deferred with a typed diagnostic.
    pub deferred_pairs: u64,
}

/// Detects coplanar face-on-face contacts among broad-phase candidates.
///
/// `pairs` is the broad-phase candidate list (the same list the narrow
/// phase consumed); `strategy` must match the enumeration strategy it was
/// produced under. Both meshes must be in their pre-split state: the
/// recorded polygons are the original face loops classification tests
/// against after splitting.
///
/// `out` is cleared (retaining capacity) and filled in ascending
/// `(face_a, face_b)` order. Unhandled coplanar configurations push
/// [`BooleanFailureKind::CoplanarAmbiguity`] diagnostics — typed, never
/// silent.
pub fn collect_coplanar_contacts(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    pairs: &[BooleanCandidatePair],
    strategy: FaceTriangulation,
    scratch: &mut BooleanScratch,
    out: &mut Vec<CoplanarContact>,
    diagnostics: &mut BooleanDiagnostics,
) -> CoplanarStats {
    out.clear();
    let mut stats = CoplanarStats::default();
    let mut buffer = core::mem::take(&mut scratch.narrow_face_a);

    // Unique candidate face pairs in ascending index order.
    let mut face_pairs: Vec<(FaceId, FaceId)> = pairs
        .iter()
        .map(|pair| (pair.a.face, pair.b.face))
        .collect();
    face_pairs.sort_unstable_by_key(|(a, b)| (a.index(), b.index()));
    face_pairs.dedup();

    // Reused per-pair storage (allocation-calm steady state).
    let mut polygon_a3: Vec<[f64; 3]> = Vec::new();
    let mut polygon_b3: Vec<[f64; 3]> = Vec::new();
    let mut triangles_a: Vec<[[f64; 3]; 3]> = Vec::new();
    let mut triangles_b: Vec<[[f64; 3]; 3]> = Vec::new();

    for (face_a, face_b) in face_pairs {
        stats.face_pairs += 1;
        face_polygon(mesh_a, face_a, &mut polygon_a3);
        face_polygon(mesh_b, face_b, &mut polygon_b3);
        if polygon_a3.len() < 3 || polygon_b3.len() < 3 {
            continue; // Degenerate loops; narrow-phase diagnostics own these.
        }
        face_triangles(mesh_a, face_a, strategy, &mut buffer, &mut triangles_a);
        face_triangles(mesh_b, face_b, strategy, &mut buffer, &mut triangles_b);
        let Some(plane_a) = triangles_a
            .iter()
            .copied()
            .find(|tri| !triangle_is_flat(tri))
        else {
            continue; // All-degenerate face; narrow-phase diagnostics own it.
        };

        // Quick exact rejection: most candidate face pairs are not
        // coplanar and reject on the first off-plane vertex.
        let b_on_plane = on_plane(&plane_a, &polygon_b3);
        let a_planar = on_plane(&plane_a, &polygon_a3);
        if !(a_planar && b_on_plane) {
            // Whole-face contact is off the table. Planar faces on
            // distinct planes have no coplanar triangles (the transversal
            // machinery owns them); a non-planar face can still hide
            // exactly coplanar triangle pairs, which are outside this
            // stage's envelope and must stay typed.
            let b_planar = triangles_b
                .iter()
                .copied()
                .find(|tri| !triangle_is_flat(tri))
                .is_none_or(|plane_b| on_plane(&plane_b, &polygon_b3));
            if !(a_planar && b_planar) && has_coplanar_triangle_pair(&triangles_a, &triangles_b) {
                stats.deferred_pairs += 1;
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::CoplanarAmbiguity,
                    a: Some(BooleanTriangleRef {
                        face: face_a,
                        triangle_index: 0,
                    }),
                    b: Some(BooleanTriangleRef {
                        face: face_b,
                        triangle_index: 0,
                    }),
                    detail: "coplanar triangles within a non-planar face pair; contact deferred",
                });
            }
            continue;
        }
        stats.coplanar_face_pairs += 1;

        // Outward-normal agreement via an exact probe: a constructed
        // point off the plane sees each face's triangle orientation as an
        // exact orient3d sign; equal signs mean equal outward normals.
        let normal = triangle_normal(&plane_a);
        let probe = [
            plane_a[0][0] + normal[0],
            plane_a[0][1] + normal[1],
            plane_a[0][2] + normal[2],
        ];
        let side_a = orient3d(plane_a[0], plane_a[1], plane_a[2], probe);
        let side_b = triangles_b
            .iter()
            .copied()
            .find(|tri| !triangle_is_flat(tri))
            .map(|tri| orient3d(tri[0], tri[1], tri[2], probe));
        let Some(side_b) = side_b else {
            continue; // All-degenerate B face; nothing to overlap.
        };
        if side_a == Orientation3d::Coplanar || side_b == Orientation3d::Coplanar {
            stats.deferred_pairs += 1;
            diagnostics.push(BooleanDiagnostic {
                kind: BooleanFailureKind::CoplanarAmbiguity,
                a: Some(BooleanTriangleRef {
                    face: face_a,
                    triangle_index: 0,
                }),
                b: Some(BooleanTriangleRef {
                    face: face_b,
                    triangle_index: 0,
                }),
                detail: "coplanar contact orientation probe degenerated; contact deferred",
            });
            continue;
        }
        let opposed = side_a != side_b;

        // Positive-area overlap: some triangle pair's interiors intersect
        // in the dominant-axis projection (exact separating-axis test).
        let axis = dominant_axis(normal);
        let positive_area = triangles_a.iter().any(|ta| {
            let ta2 = project_triangle(ta, axis);
            !projected_flat(&ta2)
                && triangles_b.iter().any(|tb| {
                    let tb2 = project_triangle(tb, axis);
                    !projected_flat(&tb2) && interiors_overlap_2d(&ta2, &tb2)
                })
        });
        if !positive_area {
            stats.grazing_pairs += 1;
            continue;
        }

        out.push(CoplanarContact {
            face_a,
            face_b,
            opposed,
            axis,
            polygon_a: polygon_a3.iter().map(|&p| project_point(p, axis)).collect(),
            polygon_b: polygon_b3.iter().map(|&p| project_point(p, axis)).collect(),
        });
        stats.contacts += 1;
    }

    scratch.narrow_face_a = buffer;
    stats
}

/// Projects a point into the plane by dropping the dominant-axis
/// coordinate; the two remaining coordinates keep cyclic order.
pub(super) fn project_point(p: [f64; 3], axis: usize) -> [f64; 2] {
    [p[(axis + 1) % 3], p[(axis + 2) % 3]]
}

/// Dominant (largest-magnitude) axis of a vector; ties resolve to the
/// lowest axis for determinism.
pub(super) fn dominant_axis(v: [f64; 3]) -> usize {
    let abs = [v[0].abs(), v[1].abs(), v[2].abs()];
    if abs[0] >= abs[1] && abs[0] >= abs[2] {
        0
    } else if abs[1] >= abs[2] {
        1
    } else {
        2
    }
}

/// Where a point lies relative to a simple polygon (closed region).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum Placement {
    /// Strictly interior.
    Inside,
    /// Exactly on the boundary.
    OnBoundary,
    /// Strictly exterior.
    Outside,
}

/// Exact point-in-simple-polygon: boundary membership first (collinear
/// plus bounding-box containment, both exact), then crossing parity with
/// the half-open vertical rule where every side decision is an exact
/// [`orient2d`] sign.
pub(super) fn place_point_in_polygon(p: [f64; 2], polygon: &[[f64; 2]]) -> Placement {
    let count = polygon.len();
    for i in 0..count {
        let a = polygon[i];
        let b = polygon[(i + 1) % count];
        if orient2d(a, b, p) == Orientation::Collinear
            && p[0] >= a[0].min(b[0])
            && p[0] <= a[0].max(b[0])
            && p[1] >= a[1].min(b[1])
            && p[1] <= a[1].max(b[1])
        {
            return Placement::OnBoundary;
        }
    }
    let mut inside = false;
    for i in 0..count {
        let a = polygon[i];
        let b = polygon[(i + 1) % count];
        if (a[1] > p[1]) == (b[1] > p[1]) {
            continue; // Half-open rule: the edge does not straddle p's row.
        }
        // The crossing lies strictly right of p exactly when p is on the
        // interior-left of an upward edge (Ccw) or of a downward edge
        // seen reversed (Cw). p on the edge line was caught above.
        let side = orient2d(a, b, p);
        let crossing_is_right = if b[1] > a[1] {
            side == Orientation::Ccw
        } else {
            side == Orientation::Cw
        };
        if crossing_is_right {
            inside = !inside;
        }
    }
    if inside {
        Placement::Inside
    } else {
        Placement::Outside
    }
}

fn face_polygon(mesh: &Mesh, face: FaceId, out: &mut Vec<[f64; 3]>) {
    out.clear();
    for half_edge in mesh.face_loop(face) {
        if let Some(p) = mesh
            .to_vertex(half_edge)
            .and_then(|v| mesh.vertex_position(v))
        {
            out.push(promote(*p));
        }
    }
}

fn face_triangles(
    mesh: &Mesh,
    face: FaceId,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[CornerId; 3]>,
    out: &mut Vec<[[f64; 3]; 3]>,
) {
    out.clear();
    let _ = mesh.face_triangles_into(face, strategy, buffer);
    for corners in buffer.iter() {
        let mut triangle = [[0.0_f64; 3]; 3];
        let mut live = true;
        for (slot, corner) in triangle.iter_mut().zip(corners) {
            match mesh
                .to_vertex(*corner)
                .and_then(|v| mesh.vertex_position(v))
            {
                Some(p) => *slot = promote(*p),
                None => live = false,
            }
        }
        if live {
            out.push(triangle);
        }
    }
}

fn triangle_normal(t: &[[f64; 3]; 3]) -> [f64; 3] {
    let u = sub(t[1], t[0]);
    let v = sub(t[2], t[0]);
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

/// True when the triangle has exactly zero area (promoted f32 positions
/// make the zero test exact).
fn triangle_is_flat(t: &[[f64; 3]; 3]) -> bool {
    triangle_normal(t) == [0.0, 0.0, 0.0]
}

/// True when every point lies exactly on the plane of `plane` (exact
/// [`orient3d`] signs).
fn on_plane(plane: &[[f64; 3]; 3], points: &[[f64; 3]]) -> bool {
    points
        .iter()
        .all(|&p| orient3d(plane[0], plane[1], plane[2], p) == Orientation3d::Coplanar)
}

/// True when some triangle pair is exactly coplanar (non-degenerate A
/// triangle whose plane contains all of B's corners).
fn has_coplanar_triangle_pair(
    triangles_a: &[[[f64; 3]; 3]],
    triangles_b: &[[[f64; 3]; 3]],
) -> bool {
    triangles_a.iter().any(|ta| {
        !triangle_is_flat(ta)
            && triangles_b.iter().any(|tb| {
                tb.iter()
                    .all(|&p| orient3d(ta[0], ta[1], ta[2], p) == Orientation3d::Coplanar)
            })
    })
}

fn project_triangle(t: &[[f64; 3]; 3], axis: usize) -> [[f64; 2]; 3] {
    [
        project_point(t[0], axis),
        project_point(t[1], axis),
        project_point(t[2], axis),
    ]
}

fn projected_flat(t: &[[f64; 2]; 3]) -> bool {
    orient2d(t[0], t[1], t[2]) == Orientation::Collinear
}

/// Exact separating-axis test: the interiors of two non-degenerate 2D
/// triangles intersect iff no edge line of either separates them (the
/// other triangle entirely in the closed opposite half-plane).
fn interiors_overlap_2d(p: &[[f64; 2]; 3], q: &[[f64; 2]; 3]) -> bool {
    !separates(p, q) && !separates(q, p)
}

fn separates(p: &[[f64; 2]; 3], q: &[[f64; 2]; 3]) -> bool {
    for i in 0..3 {
        let a = p[i];
        let b = p[(i + 1) % 3];
        let interior_side = orient2d(a, b, p[(i + 2) % 3]);
        if interior_side == Orientation::Collinear {
            continue;
        }
        if q.iter().all(|&v| orient2d(a, b, v) != interior_side) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeshBuilder;
    use crate::boolean::{BooleanBvh, BooleanScratch};

    /// Builds an axis-aligned box mesh spanning `min..max`.
    fn box_mesh(min: [f32; 3], max: [f32; 3]) -> Mesh {
        let positions = [
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
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
            builder.add_face(&face).expect("valid box face");
        }
        builder.build().expect("valid box").mesh
    }

    fn contacts_for(mesh_a: &Mesh, mesh_b: &Mesh) -> (Vec<CoplanarContact>, CoplanarStats) {
        let mut scratch = BooleanScratch::new();
        let strategy = FaceTriangulation::Fan;
        let bvh_a = BooleanBvh::build(mesh_a, strategy, &mut scratch);
        let bvh_b = BooleanBvh::build(mesh_b, strategy, &mut scratch);
        let mut pairs = Vec::new();
        bvh_a.query_overlaps(&bvh_b, &mut scratch, &mut pairs);
        let mut out = Vec::new();
        let mut diagnostics = BooleanDiagnostics::default();
        let stats = collect_coplanar_contacts(
            mesh_a,
            mesh_b,
            &pairs,
            strategy,
            &mut scratch,
            &mut out,
            &mut diagnostics,
        );
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        (out, stats)
    }

    #[test]
    fn touching_boxes_report_one_opposed_contact() {
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let (contacts, stats) = contacts_for(&mesh_a, &mesh_b);
        assert_eq!(contacts.len(), 1, "exactly the shared wall");
        let contact = &contacts[0];
        assert!(contact.opposed, "touching solids face each other");
        assert_eq!(contact.axis, 0, "the shared plane is x = 1");
        assert_eq!(contact.polygon_a.len(), 4);
        assert_eq!(contact.polygon_b.len(), 4);
        // The coplanar top/bottom/front/back pairs graze along shared
        // edges and are skipped as zero-area.
        assert!(stats.grazing_pairs > 0, "edge-sharing coplanar pairs graze");
        assert_eq!(stats.deferred_pairs, 0);
    }

    #[test]
    fn flush_inner_box_reports_same_normal_contacts() {
        // The small box sits inside the unit box with three walls flush.
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([0.5, 0.0, 0.0], [1.0, 0.5, 0.5]);
        let (contacts, _) = contacts_for(&mesh_a, &mesh_b);
        assert_eq!(contacts.len(), 3, "three flush wall planes");
        assert!(
            contacts.iter().all(|c| !c.opposed),
            "flush overlapping boundaries share outward normals"
        );
    }

    #[test]
    fn transversal_overlap_reports_no_contacts() {
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([0.5, 0.5, 0.5], [1.5, 1.5, 1.5]);
        let (contacts, stats) = contacts_for(&mesh_a, &mesh_b);
        assert!(contacts.is_empty());
        assert_eq!(stats.coplanar_face_pairs, 0);
    }

    #[test]
    fn contact_detection_is_deterministic() {
        let mesh_a = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mesh_b = box_mesh([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let (first, first_stats) = contacts_for(&mesh_a, &mesh_b);
        let (second, second_stats) = contacts_for(&mesh_a, &mesh_b);
        assert_eq!(first, second);
        assert_eq!(first_stats, second_stats);
    }

    #[test]
    fn point_in_polygon_places_exactly() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(
            place_point_in_polygon([0.5, 0.5], &square),
            Placement::Inside
        );
        assert_eq!(
            place_point_in_polygon([2.0, 0.5], &square),
            Placement::Outside
        );
        assert_eq!(
            place_point_in_polygon([1.0, 0.5], &square),
            Placement::OnBoundary
        );
        assert_eq!(
            place_point_in_polygon([0.0, 0.0], &square),
            Placement::OnBoundary
        );
        // Rows through vertices count consistently.
        assert_eq!(
            place_point_in_polygon([0.5, 0.0], &square),
            Placement::OnBoundary
        );
        assert_eq!(
            place_point_in_polygon([-1.0, 0.0], &square),
            Placement::Outside
        );
        assert_eq!(
            place_point_in_polygon([-1.0, 1.0], &square),
            Placement::Outside
        );
    }

    #[test]
    fn separating_axis_distinguishes_overlap_from_touch() {
        let a = [[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]];
        let overlapping = [[0.5, 0.5], [2.5, 0.5], [0.5, 2.5]];
        assert!(interiors_overlap_2d(&a, &overlapping));
        // Sharing exactly one edge: zero-area intersection.
        let edge_touch = [[2.0, 0.0], [0.0, 2.0], [2.0, 2.0]];
        assert!(!interiors_overlap_2d(&a, &edge_touch));
        // Identical triangles overlap themselves.
        assert!(interiors_overlap_2d(&a, &a));
        let disjoint = [[5.0, 5.0], [6.0, 5.0], [5.0, 6.0]];
        assert!(!interiors_overlap_2d(&a, &disjoint));
    }
}
