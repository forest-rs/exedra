// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_mesh::{CornerId, FaceId, Mesh};
use exedra_triangulate::predicates::{Orientation, orient2d};

/// Allowance, in f32 epsilons of coordinate magnitude, for a stored boundary
/// sample to count as collinear with its neighbours. Narrowing one point
/// moves each coordinate by at most half an ulp; four epsilons cover the
/// sample and both segment endpoints with margin.
const COLLINEAR_STORAGE_ULPS: f64 = 4.0;

/// True when `p` lies on the segment `a`–`b` exactly in every coordinate
/// projection, or within f32 storage rounding of it in 3-D.
fn on_segment_after_storage(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> bool {
    let exact = (0..3).all(|x| {
        let y = (x + 1) % 3;
        orient2d([a[x], a[y]], [b[x], b[y]], [p[x], p[y]]) == Orientation::Collinear
    });
    if exact {
        return true;
    }
    let e = sub(b, a);
    let length_squared = dot(e, e);
    if length_squared.partial_cmp(&0.0) != Some(core::cmp::Ordering::Greater) {
        return false;
    }
    let d = sub(p, a);
    let t = dot(d, e) / length_squared;
    let offset = sub(d, scale(e, t));
    let magnitude = a
        .iter()
        .chain(&b)
        .chain(&p)
        .fold(0.0_f64, |m, c| m.max(c.abs()));
    let tolerance = COLLINEAR_STORAGE_ULPS * f64::from(f32::EPSILON) * magnitude;
    dot(offset, offset) <= tolerance * tolerance
}

/// Restores omitted boundary samples without changing the robust triangle surface.
///
/// A boundary sample the robust triangulation dropped as a degenerate ear is
/// reinserted when it lies on the segment between its kept neighbours: exactly,
/// or within f32 storage rounding of that segment. Stored coordinates of an
/// exactly collinear point sit up to a few f32 units in the last place off
/// the line, so far from the origin exact predicates alone would refuse
/// faces that are planar before storage. The allowance is
/// [`COLLINEAR_STORAGE_ULPS`] f32 epsilons of the largest coordinate
/// magnitude involved; the surface moves by at most that much.
///
/// Projection-only collinearity on a genuinely nonplanar face is still
/// refused: inserting such a point would choose a different surface. So is a
/// reinsertion that would flip a split triangle's orientation.
pub(super) fn restore_boundary(
    mesh: &Mesh,
    face: FaceId,
    triangles: &mut Vec<[CornerId; 3]>,
) -> Result<(), SectionError> {
    let boundary: Vec<_> = mesh.face_loop(face).collect();
    let used: BTreeSet<_> = triangles.iter().flatten().copied().collect();
    if used.len() == boundary.len() {
        return Ok(());
    }
    let mut owners = BTreeMap::new();
    for (index, t) in triangles.iter().enumerate() {
        for j in 0..3 {
            owners.insert((t[j], t[(j + 1) % 3]), index);
        }
    }
    let position = |c| {
        mesh.vertex_position(mesh.to_vertex(c).expect("validated corner"))
            .expect("validated position")
            .map(f64::from)
    };
    for i in 0..boundary.len() {
        if !used.contains(&boundary[i]) || used.contains(&boundary[(i + 1) % boundary.len()]) {
            continue;
        }
        let mut chain = alloc::vec![boundary[i]];
        for step in 1..boundary.len() {
            let c = boundary[(i + step) % boundary.len()];
            chain.push(c);
            if used.contains(&c) {
                break;
            }
        }
        let a = chain[0];
        let b = *chain.last().ok_or(SectionError::Triangulation)?;
        let pa = position(a);
        let pb = position(b);
        let axis = (0..3)
            .max_by(|&i, &j| (pb[i] - pa[i]).abs().total_cmp(&(pb[j] - pa[j]).abs()))
            .expect("three axes");
        let mut previous = pa[axis];
        for &c in &chain[1..chain.len() - 1] {
            let p = position(c);
            if !on_segment_after_storage(pa, pb, p) {
                return Err(SectionError::Triangulation);
            }
            if !((previous < p[axis] && p[axis] < pb[axis])
                || (previous > p[axis] && p[axis] > pb[axis]))
            {
                return Err(SectionError::Triangulation);
            }
            previous = p[axis];
        }
        let index = *owners.get(&(a, b)).ok_or(SectionError::Triangulation)?;
        let old = triangles[index];
        let third = *old
            .iter()
            .find(|&&c| c != a && c != b)
            .ok_or(SectionError::Triangulation)?;
        // Splitting must keep every piece facing the way the old triangle did;
        // a sample within storage rounding of the edge can otherwise fold a
        // sliver piece over when the third corner is nearly on that edge too.
        let normal_of = |t: [CornerId; 3]| {
            let [p0, p1, p2] = t.map(position);
            cross(sub(p1, p0), sub(p2, p0))
        };
        let old_normal = normal_of(old);
        if chain.windows(2).any(|pair| {
            dot(normal_of([pair[0], pair[1], third]), old_normal).partial_cmp(&0.0)
                != Some(core::cmp::Ordering::Greater)
        }) {
            return Err(SectionError::Triangulation);
        }
        for j in 0..3 {
            owners.remove(&(old[j], old[(j + 1) % 3]));
        }
        for (part, pair) in chain.windows(2).enumerate() {
            let triangle = [pair[0], pair[1], third];
            let target = if part == 0 {
                triangles[index] = triangle;
                index
            } else {
                let index = triangles.len();
                triangles.push(triangle);
                index
            };
            for j in 0..3 {
                owners.insert((triangle[j], triangle[(j + 1) % 3]), target);
            }
        }
    }
    Ok(())
}
