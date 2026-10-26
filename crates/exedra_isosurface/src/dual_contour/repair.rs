// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Repair of folded slivers left by transition emission.
//!
//! Where cells of different sizes meet, a patch can join three vertices that
//! lie almost on one line. The resulting sliver has next to no area, but its
//! middle vertex lands on either side of the line, so half the time the
//! sliver faces backwards and darkens angle-weighted normals. Flipping its
//! long edge with the neighbor across it replaces the sliver with two
//! well-shaped triangles over the same region. A crease tagged on the long
//! edge moves to the sliver's short edges, which run through the middle
//! vertex along the same line.

use alloc::vec::Vec;
use exedra_math::{cross, dot, sub};
use exedra_mesh::{FaceId, HalfEdgeId, Mesh, attr, op};

use super::triangle_is_nondegenerate;

/// Height over the long edge, as a fraction of its length, below which a
/// triangle counts as a sliver.
const SLIVER_HEIGHT: f64 = 0.1;

/// Flips the long edge of every sliver that faces against its neighbor
/// across that edge, when the two replacement triangles both face with the
/// neighbor. Returns the number of flips.
pub(super) fn flip_folded_slivers(mesh: &mut Mesh) -> usize {
    let faces = mesh.faces().collect::<Vec<_>>();
    let mut session = mesh.edit();
    let mut flips = 0;
    for face in faces {
        let Some(edge) = folded_sliver_edge(session.mesh(), face) else {
            continue;
        };
        // A crease along the long edge runs on through the middle vertex,
        // over the sliver's short edges, which the flip keeps.
        let mesh = session.mesh();
        let sharpness = mesh.edge_sharpness(edge).unwrap_or(0.0);
        let short = mesh
            .next(edge)
            .and_then(|next| Some([next, mesh.next(next)?]));
        if op::flip_edge(&mut session, edge).is_err() {
            continue;
        }
        flips += 1;
        if sharpness > 0.0
            && let Some(short) = short
        {
            for edge in short {
                let kept = session.mesh().edge_sharpness(edge).unwrap_or(0.0);
                op::set_edge_sharpness(&mut session, edge, kept.max(sharpness))
                    .expect("the flip keeps perimeter half-edges live");
            }
        }
    }
    let _: () = session.finish();
    flips
}

/// The long edge of `face` when it is a folded sliver that a flip repairs.
fn folded_sliver_edge(mesh: &Mesh, face: FaceId) -> Option<HalfEdgeId> {
    let edges = mesh.face_loop(face).collect::<Vec<_>>();
    if edges.len() != 3 {
        return None;
    }
    let to = |edge| mesh.vertex_position(mesh.to_vertex(edge)?).copied();
    let from = |edge| mesh.vertex_position(mesh.from_vertex(edge)?).copied();
    let length2 = |edge| Some(squared(sub(to(edge)?, from(edge)?).map(f64::from)));
    let mut long = edges[0];
    for &edge in &edges[1..] {
        if length2(edge)? > length2(long)? {
            long = edge;
        }
    }
    // The sliver is (a, b, c) and its neighbor (b, a, d); a flip makes
    // (c, d, b) and (d, c, a).
    let (a, b, c) = (from(long)?, to(long)?, to(mesh.next(long)?)?);
    let twin = mesh.twin(long)?;
    let neighbor = mesh.face(twin)?;
    if neighbor == FaceId::OUTSIDE {
        return None;
    }
    let d = to(mesh.next(twin)?)?;
    let sliver = normal(a, b, c);
    let span = length2(long)?;
    if squared(sliver) >= SLIVER_HEIGHT * SLIVER_HEIGHT * span * span {
        return None;
    }
    let facing = normal(b, a, d);
    if dot(sliver, facing) >= 0.0 {
        return None;
    }
    let repaired = [[c, d, b], [d, c, a]];
    if !repaired.iter().all(|&[p, q, r]| {
        triangle_is_nondegenerate([p, q, r]) && dot(normal(p, q, r), facing) > 0.0
    }) {
        return None;
    }
    // Keep region boundaries where they are.
    if let Some(regions) = mesh.attrs().dense(attr::FACE_REGION)
        && regions.get(face.as_id()) != regions.get(neighbor.as_id())
    {
        return None;
    }
    Some(long)
}

fn normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f64; 3] {
    let [a, b, c] = [a, b, c].map(|p| p.map(f64::from));
    cross(sub(b, a), sub(c, a))
}

fn squared(v: [f64; 3]) -> f64 {
    dot(v, v)
}

#[cfg(test)]
mod tests {
    use super::flip_folded_slivers;
    use exedra_mesh::{FaceBuildAttrs, MeshBuilder};

    /// A flat unit square. Above its diagonal `a-b` lies `(a, b, e)`;
    /// below, `(a, d, c)` and `(d, b, c)` meet at a middle vertex `c` that
    /// slid just across the diagonal, so the sliver `(a, c, b)` between them
    /// faces down.
    fn folded(sharpness: f32) -> exedra_mesh::Mesh {
        let mut builder = MeshBuilder::new();
        let a = builder.push_vertex([0.0, 0.0, 0.0]);
        let b = builder.push_vertex([1.0, 1.0, 0.0]);
        let c = builder.push_vertex([0.48, 0.52, 0.0]);
        let d = builder.push_vertex([1.0, 0.0, 0.0]);
        let e = builder.push_vertex([0.0, 1.0, 0.0]);
        let (upper, sliver) = ([sharpness, 0.0, 0.0], [0.0, 0.0, sharpness]);
        let with = |sharp| FaceBuildAttrs {
            edge_sharpness: Some(sharp),
            ..FaceBuildAttrs::default()
        };
        builder
            .add_face_with_attrs(&[a, b, e], &with(&upper))
            .unwrap();
        builder
            .add_face_with_attrs(&[a, c, b], &with(&sliver))
            .unwrap();
        builder
            .add_face_with_attrs(&[a, d, c], &FaceBuildAttrs::default())
            .unwrap();
        builder
            .add_face_with_attrs(&[d, b, c], &FaceBuildAttrs::default())
            .unwrap();
        builder.build().unwrap().mesh
    }

    fn up_facing(mesh: &exedra_mesh::Mesh) -> usize {
        mesh.faces()
            .filter(|&face| {
                let p = mesh
                    .face_loop(face)
                    .map(|h| *mesh.vertex_position(mesh.to_vertex(h).unwrap()).unwrap())
                    .collect::<alloc::vec::Vec<_>>();
                (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                    - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0])
                    > 0.0
            })
            .count()
    }

    #[test]
    fn a_folded_sliver_flips_into_two_facing_triangles() {
        let mut mesh = folded(0.0);
        assert_eq!(up_facing(&mesh), 3);
        assert_eq!(flip_folded_slivers(&mut mesh), 1);
        assert_eq!(up_facing(&mesh), 4);
        assert!(mesh.validate_deep().is_empty());
        assert_eq!(flip_folded_slivers(&mut mesh), 0, "nothing left to repair");
    }

    #[test]
    fn a_crease_on_the_long_edge_moves_to_the_short_edges() {
        let mut mesh = folded(1.0);
        assert_eq!(flip_folded_slivers(&mut mesh), 1);
        let sharp = mesh
            .half_edges()
            .filter(|&h| mesh.edge_sharpness(h).unwrap_or(0.0) > 0.0)
            .map(|h| {
                let mut ends = [mesh.from_vertex(h).unwrap(), mesh.to_vertex(h).unwrap()]
                    .map(|v| mesh.vertex_position(v).unwrap()[0]);
                ends.sort_by(f32::total_cmp);
                ends
            })
            .collect::<alloc::vec::Vec<_>>();
        // Both halves of the crease, through the middle vertex at x = 0.48.
        assert!(
            sharp.contains(&[0.0, 0.48]) && sharp.contains(&[0.48, 1.0]),
            "{sharp:?}"
        );
    }
}
