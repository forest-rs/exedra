// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_mesh::{CornerId, FaceId, Mesh};
use exedra_triangulate::predicates::{Orientation, orient2d};

/// Restores omitted boundary samples without changing the robust triangle surface.
/// Projection-only collinearity on a nonplanar face is insufficient: inserting
/// such a point would choose a different surface, so that case is refused.
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
            for x in 0..3 {
                let y = (x + 1) % 3;
                if orient2d([pa[x], pa[y]], [pb[x], pb[y]], [p[x], p[y]]) != Orientation::Collinear
                {
                    return Err(SectionError::Triangulation);
                }
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
