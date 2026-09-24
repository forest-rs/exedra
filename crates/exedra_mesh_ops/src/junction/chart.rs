// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Skin texture coordinates that continue every tube's own chart.
//!
//! Ring vertices keep the UVs their tube faces give them. New vertices take
//! the discrete harmonic extension of those values over the skin, so the
//! chart is continuous across every skin/tube boundary and smooth inside.
//!
//! A tube's chart usually jumps across one ring edge, its U seam. That jump
//! cannot vanish inside the skin: it travels along a cut, a path of faces
//! from the ring edge to one *sink* face: the face farthest from ring 0
//! (typically the parent, so the sink falls between the branches), then
//! farthest from every ring.
//! Edges the cut crosses carry the jump in the harmonic equations, so the
//! extension is smooth across the cut, and corner UVs of faces on the cut
//! are lifted onto one side of it. The texture is therefore continuous
//! except across one side of each cut, where it jumps by exactly the tube's
//! own seam jump (invisible when that jump is a whole number of texture
//! repeats), and inside the sink face, which absorbs whatever the tubes'
//! jumps fail to cancel.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec;
use alloc::vec::Vec;

use super::smooth::conjugate_gradient;
use super::{JunctionError, JunctionFace, JunctionVertex};

/// Per ring and ring edge `k -> k + 1`, the UVs at both ends as the tube face
/// across that edge holds them.
pub(super) type RingUvs = [Vec<[[f64; 2]; 2]>];

/// A face and one of its half-edges `(face, from, to)`, by graph node.
type Crossing = (usize, usize, usize);

/// Checks caller ring UVs: one finite pair per ring edge.
pub(super) fn check(ring_uvs: &RingUvs, rings: &[Vec<[f64; 3]>]) -> Result<(), JunctionError> {
    if ring_uvs.len() != rings.len() {
        return Err(JunctionError::InvalidRingUvs { ring: None });
    }
    for (ring, (uvs, points)) in ring_uvs.iter().zip(rings).enumerate() {
        if uvs.len() != points.len() || !uvs.iter().flatten().flatten().all(|c| c.is_finite()) {
            return Err(JunctionError::InvalidRingUvs { ring: Some(ring) });
        }
    }
    Ok(())
}

/// Writes corner UVs to every face.
pub(super) fn assign(
    rings: &[Vec<[f64; 3]>],
    skin_vertices: usize,
    faces: &mut [JunctionFace],
    ring_uvs: &RingUvs,
) {
    let mut ring_base = Vec::with_capacity(rings.len());
    let mut nodes = skin_vertices;
    for ring in rings {
        ring_base.push(nodes);
        nodes += ring.len();
    }
    let node = |vertex: JunctionVertex| match vertex {
        JunctionVertex::Skin(index) => index,
        JunctionVertex::Ring { ring, index } => ring_base[ring] + index,
    };
    let ring_edge = |a: JunctionVertex, b: JunctionVertex| match (a, b) {
        (
            JunctionVertex::Ring { ring, index },
            JunctionVertex::Ring {
                ring: other,
                index: next,
            },
        ) if ring == other && next == (index + 1) % rings[ring].len() => Some((ring, index)),
        _ => None,
    };

    // Directed-edge jumps: moving from `a` to `b`, the lifted value is
    // `x[b] + jump(a, b)`. Ring edges carry the tube's own jumps.
    let mut jumps: BTreeMap<(usize, usize), [f64; 2]> = BTreeMap::new();
    let mut add_jump = |a: usize, b: usize, j: [f64; 2]| {
        let forward = jumps.entry((a, b)).or_insert([0.0; 2]);
        forward[0] += j[0];
        forward[1] += j[1];
        let backward = jumps.entry((b, a)).or_insert([0.0; 2]);
        backward[0] -= j[0];
        backward[1] -= j[1];
    };

    let mut values = vec![[0.0; 2]; nodes];
    let mut sources = Vec::new();
    for (ring, uvs) in ring_uvs.iter().enumerate() {
        let len = uvs.len();
        for k in 0..len {
            values[ring_base[ring] + k] = uvs[k][0];
            let next = uvs[(k + 1) % len][0];
            let jump = [uvs[k][1][0] - next[0], uvs[k][1][1] - next[1]];
            if jump != [0.0; 2] {
                sources.push((ring, k, jump));
            }
        }
    }

    // Dual graph over non-ring edges.
    let mut edge_faces: BTreeMap<(usize, usize), Vec<Crossing>> = BTreeMap::new();
    let mut touches_ring = vec![false; faces.len()];
    for (f, face) in faces.iter().enumerate() {
        let n = face.vertices.len();
        for k in 0..n {
            let (a, b) = (face.vertices[k], face.vertices[(k + 1) % n]);
            if ring_edge(a, b).is_some() {
                touches_ring[f] = true;
                continue;
            }
            let (a, b) = (node(a), node(b));
            edge_faces
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((f, a, b));
        }
    }
    let mut adjacent: Vec<Vec<Crossing>> = vec![Vec::new(); faces.len()];
    for list in edge_faces.values() {
        if let [(f, a, b), (g, c, d)] = list.as_slice() {
            adjacent[*f].push((*g, *a, *b));
            adjacent[*g].push((*f, *c, *d));
        }
    }

    // Sink: farthest from ring 0 (typically the parent, so the sink falls
    // between the branches), then farthest from every ring, then with the
    // most new vertices.
    let depth = bfs(&adjacent, touches_ring.iter().copied());
    let from_first = bfs(
        &adjacent,
        faces.iter().map(|face| {
            face.vertices
                .iter()
                .any(|v| matches!(v, JunctionVertex::Ring { ring: 0, .. }))
        }),
    );
    let skin = |f: usize| {
        faces[f]
            .vertices
            .iter()
            .filter(|v| matches!(v, JunctionVertex::Skin(_)))
            .count()
    };
    let sink = (0..faces.len())
        .max_by(|&f, &g| {
            (from_first[f], depth[f], skin(f))
                .cmp(&(from_first[g], depth[g], skin(g)))
                .then(g.cmp(&f))
        })
        .expect("a skin has faces");
    let parents = bfs_parents(&adjacent, sink);

    for (ring, k, jump) in sources {
        let len = rings[ring].len();
        let (a, b) = (ring_base[ring] + k, ring_base[ring] + (k + 1) % len);
        add_jump(a, b, jump);
        let from = JunctionVertex::Ring { ring, index: k };
        let mut face = faces
            .iter()
            .position(|face| {
                let n = face.vertices.len();
                (0..n).any(|m| {
                    face.vertices[m] == from
                        && ring_edge(face.vertices[m], face.vertices[(m + 1) % n]).is_some()
                })
            })
            .expect("every ring edge has a skin face");
        // Crossing from face f over its half-edge a -> b carries the jump
        // on, so every face on the cut keeps a zero loop sum.
        let carried = [-jump[0], -jump[1]];
        while face != sink {
            let Some((next, a, b)) = parents[face] else {
                break;
            };
            add_jump(a, b, carried);
            face = next;
        }
    }

    // Harmonic extension: for each new vertex a,
    // sum over neighbours b of (x[b] + jump(a, b) - x[a]) = 0.
    let free = skin_vertices;
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for face in faces.iter() {
        let n = face.vertices.len();
        for k in 0..n {
            let (a, b) = (node(face.vertices[k]), node(face.vertices[(k + 1) % n]));
            neighbours[a].push(b);
            neighbours[b].push(a);
        }
    }
    for list in &mut neighbours {
        list.sort_unstable();
        list.dedup();
    }
    let jump = |a: usize, b: usize| jumps.get(&(a, b)).copied().unwrap_or([0.0; 2]);
    let rows = (0..free)
        .map(|a| {
            let mut row = vec![(a, neighbours[a].len() as f64)];
            row.extend(neighbours[a].iter().map(|&b| (b, -1.0)));
            row
        })
        .collect::<Vec<_>>();
    #[expect(
        clippy::needless_range_loop,
        reason = "each axis indexes both values and jumps"
    )]
    for axis in 0..2 {
        let rhs = (0..free)
            .map(|a| {
                neighbours[a]
                    .iter()
                    .map(|&b| {
                        let fixed = if b >= free { values[b][axis] } else { 0.0 };
                        fixed + jump(a, b)[axis]
                    })
                    .sum::<f64>()
            })
            .collect::<Vec<_>>();
        let solution = conjugate_gradient(&rows, free, &rhs, vec![0.0; free]);
        for (a, value) in solution.into_iter().enumerate() {
            values[a][axis] = value;
        }
    }

    // Lift each face's corners from a base corner along its edges. A face
    // with a ring edge starts there, so its ring corners match the tube.
    for face in faces.iter_mut() {
        let n = face.vertices.len();
        let base = (0..n)
            .find(|&k| ring_edge(face.vertices[k], face.vertices[(k + 1) % n]).is_some())
            .or_else(|| (0..n).find(|&k| matches!(face.vertices[k], JunctionVertex::Ring { .. })))
            .unwrap_or(0);
        let mut uvs = vec![[0.0; 2]; n];
        let mut current = node(face.vertices[base]);
        let mut lifted = values[current];
        uvs[base] = lifted;
        for step in 1..n {
            let k = (base + step) % n;
            let next = node(face.vertices[k]);
            let j = jump(current, next);
            for axis in 0..2 {
                lifted[axis] += values[next][axis] - values[current][axis] + j[axis];
            }
            uvs[k] = lifted;
            current = next;
        }
        face.uvs = Some(uvs);
    }
}

/// Multi-source breadth-first depth over the dual graph.
fn bfs(adjacent: &[Vec<Crossing>], sources: impl Iterator<Item = bool>) -> Vec<usize> {
    let mut depth = vec![usize::MAX; adjacent.len()];
    let mut queue = VecDeque::new();
    for (f, source) in sources.enumerate() {
        if source {
            depth[f] = 0;
            queue.push_back(f);
        }
    }
    while let Some(f) = queue.pop_front() {
        for &(g, _, _) in &adjacent[f] {
            if depth[g] == usize::MAX {
                depth[g] = depth[f] + 1;
                queue.push_back(g);
            }
        }
    }
    depth
        .into_iter()
        .map(|d| if d == usize::MAX { 0 } else { d })
        .collect()
}

/// Breadth-first tree toward `root`: each face's next face on its path to
/// the root and the half-edge `(a, b)` of the face that the step crosses.
fn bfs_parents(adjacent: &[Vec<Crossing>], root: usize) -> Vec<Option<Crossing>> {
    let mut parents = vec![None; adjacent.len()];
    let mut seen = vec![false; adjacent.len()];
    seen[root] = true;
    let mut queue = VecDeque::from([root]);
    while let Some(f) = queue.pop_front() {
        for &(g, _, _) in &adjacent[f] {
            if seen[g] {
                continue;
            }
            seen[g] = true;
            // The step from g toward f crosses g's half-edge on the shared
            // edge; find it from g's own adjacency.
            let crossing = adjacent[g]
                .iter()
                .find(|(h, _, _)| *h == f)
                .map(|&(_, a, b)| (f, a, b));
            parents[g] = crossing;
            queue.push_back(g);
        }
    }
    parents
}

/// Harmonic weights of the ring vertices at every new skin vertex.
///
/// Entry `v` lists `((ring, index), weight)` for new vertex `v`, heaviest
/// first and summing to one: the discrete harmonic function over the skin
/// graph that is one at that ring vertex and zero at the others, the same
/// extension [`assign`] uses for UVs. Uniform umbrella weights keep every
/// weight non-negative, so interpolated data is never extrapolated.
pub(super) fn ring_weights(
    ring_lens: &[usize],
    skin_vertices: usize,
    faces: &[JunctionFace],
) -> Vec<Vec<((usize, usize), f64)>> {
    let mut ring_base = Vec::with_capacity(ring_lens.len());
    let mut nodes = skin_vertices;
    for len in ring_lens {
        ring_base.push(nodes);
        nodes += len;
    }
    let node = |vertex: JunctionVertex| match vertex {
        JunctionVertex::Skin(index) => index,
        JunctionVertex::Ring { ring, index } => ring_base[ring] + index,
    };
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for face in faces {
        let n = face.vertices.len();
        for k in 0..n {
            let (a, b) = (node(face.vertices[k]), node(face.vertices[(k + 1) % n]));
            neighbours[a].push(b);
            neighbours[b].push(a);
        }
    }
    for list in &mut neighbours {
        list.sort_unstable();
        list.dedup();
    }
    let free = skin_vertices;
    let rows = (0..free)
        .map(|a| {
            let mut row = vec![(a, neighbours[a].len() as f64)];
            row.extend(neighbours[a].iter().map(|&b| (b, -1.0)));
            row
        })
        .collect::<Vec<_>>();
    let mut weights = vec![Vec::new(); free];
    for (ring, &len) in ring_lens.iter().enumerate() {
        for index in 0..len {
            let source = ring_base[ring] + index;
            // A ring vertex no new vertex neighbours has no influence.
            let rhs = (0..free)
                .map(|a| f64::from(u8::from(neighbours[a].contains(&source))))
                .collect::<Vec<_>>();
            if rhs.iter().all(|&b| b == 0.0) {
                continue;
            }
            let solution = conjugate_gradient(&rows, free, &rhs, vec![0.0; free]);
            for (a, weight) in solution.into_iter().enumerate() {
                if weight > 1e-9 {
                    weights[a].push(((ring, index), weight));
                }
            }
        }
    }
    for list in &mut weights {
        let total = list.iter().map(|(_, w)| w).sum::<f64>();
        for (_, weight) in list.iter_mut() {
            *weight /= total;
        }
        list.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    }
    weights
}
