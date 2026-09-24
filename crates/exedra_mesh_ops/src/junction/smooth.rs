// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opt-in skin smoothing: refine the coarse skin into rows between the rings,
//! then fair every new vertex with a discrete thin-plate energy whose
//! boundary includes each tube's wall direction.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    JunctionError, JunctionFace, JunctionSmoothing, JunctionVertex, add, dot, narrow, norm,
    normalize, scale, strip_faces, sub,
};

/// A refined, faired skin: new vertex positions and faces.
pub(super) struct Smoothed {
    pub(super) skin_vertices: Vec<[f64; 3]>,
    pub(super) faces: Vec<JunctionFace>,
}

/// Refines and fairs a coarse skin.
///
/// `tangents[ring][index]` is the unit direction in which the tube wall runs
/// into ring vertex `index`, continuing toward the skin.
pub(super) fn smooth(
    rings: &[Vec<[f64; 3]>],
    coarse_vertices: &[[f64; 3]],
    coarse_faces: &[JunctionFace],
    tangents: &[Vec<[f64; 3]>],
    smoothing: &JunctionSmoothing,
) -> Result<Smoothed, JunctionError> {
    let mut refine = Refine {
        rings,
        rows: smoothing.rows.get() as usize,
        vertices: coarse_vertices.to_vec(),
        splits: BTreeMap::new(),
    };
    let mut faces = Vec::new();
    for (index, face) in coarse_faces.iter().enumerate() {
        let refined = refine
            .face(&face.vertices)
            .ok_or(JunctionError::UnrefinableFace { face: index })?;
        for vertices in refined {
            faces.push(JunctionFace {
                vertices,
                uvs: None,
                origin: face.origin.clone(),
            });
        }
    }
    let mut vertices = refine.vertices;
    fair(rings, &mut vertices, &faces, tangents)?;
    for vertex in &mut vertices {
        // Stored at the mesh's f32 precision, so the exact checks see the
        // positions the mesh will hold.
        *vertex = narrow(*vertex).map(f64::from);
    }
    Ok(Smoothed {
        skin_vertices: vertices,
        faces,
    })
}

struct Refine<'a> {
    rings: &'a [Vec<[f64; 3]>],
    rows: usize,
    vertices: Vec<[f64; 3]>,
    /// Interior points of each split edge, keyed by its ascending endpoints
    /// and listed from the lower endpoint.
    splits: BTreeMap<(JunctionVertex, JunctionVertex), Vec<JunctionVertex>>,
}

impl Refine<'_> {
    fn position(&self, vertex: JunctionVertex) -> [f64; 3] {
        position(self.rings, &self.vertices, vertex)
    }

    fn is_ring_edge(&self, a: JunctionVertex, b: JunctionVertex) -> bool {
        match (a, b) {
            (
                JunctionVertex::Ring { ring, index },
                JunctionVertex::Ring {
                    ring: other,
                    index: next,
                },
            ) => ring == other && next == (index + 1) % self.rings[ring].len(),
            _ => false,
        }
    }

    fn push(&mut self, position: [f64; 3]) -> JunctionVertex {
        self.vertices.push(position);
        JunctionVertex::Skin(self.vertices.len() - 1)
    }

    /// The `rows + 1` points of the non-ring edge from `a` to `b`, shared by
    /// both faces on the edge. New points start evenly spaced on the segment.
    fn split(&mut self, a: JunctionVertex, b: JunctionVertex) -> Vec<JunctionVertex> {
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        if !self.splits.contains_key(&(low, high)) {
            let (p, q) = (self.position(low), self.position(high));
            let interior = (1..self.rows)
                .map(|r| {
                    let t = r as f64 / self.rows as f64;
                    self.push(add(p, scale(sub(q, p), t)))
                })
                .collect();
            self.splits.insert((low, high), interior);
        }
        let interior = &self.splits[&(low, high)];
        let mut points = Vec::with_capacity(self.rows + 1);
        points.push(a);
        if a == low {
            points.extend(interior.iter().copied());
        } else {
            points.extend(interior.iter().rev().copied());
        }
        points.push(b);
        points
    }

    /// Refines one coarse face into rows stacked from its ring edge.
    ///
    /// Coarse faces come in three shapes, told apart by their ring edges,
    /// which are never split because the tube faces own them:
    /// bridge quads (two opposite ring edges), bridge triangles (one ring
    /// edge and an apex on the other ring), and crotch quads (one ring edge,
    /// a bridge side edge, and two spokes to the crotch center).
    ///
    /// Returns `None` for any other shape, which the coarse construction
    /// does not produce; splitting its edges would leave T-vertices.
    fn face(&mut self, vertices: &[JunctionVertex]) -> Option<Vec<Vec<JunctionVertex>>> {
        let n = vertices.len();
        if self.rows == 1 {
            return Some(vec![vertices.to_vec()]);
        }
        let ring_edges = (0..n)
            .filter(|&k| self.is_ring_edge(vertices[k], vertices[(k + 1) % n]))
            .collect::<Vec<_>>();
        // Rotate so a ring edge comes first; rotation keeps the orientation.
        let first = ring_edges.first().copied().unwrap_or(0);
        let v = (0..n)
            .map(|k| vertices[(first + k) % n])
            .collect::<Vec<_>>();
        let rows = match (n, ring_edges.len()) {
            (4, 2) => {
                let left = self.split(v[0], v[3]);
                let right = self.split(v[1], v[2]);
                (0..=self.rows)
                    .map(|r| vec![left[r], right[r]])
                    .collect::<Vec<_>>()
            }
            (3, 1) => {
                let left = self.split(v[0], v[2]);
                let right = self.split(v[1], v[2]);
                let mut rows = (0..self.rows)
                    .map(|r| vec![left[r], right[r]])
                    .collect::<Vec<_>>();
                rows.push(vec![v[2]]);
                rows
            }
            (4, 1) => self.crotch_rows(&v),
            _ => return None,
        };
        Some(
            rows.windows(2)
                .flat_map(|pair| strip_faces(&pair[0], &pair[1]))
                .map(|(face, _)| face)
                .collect(),
        )
    }

    /// Rows of a crotch quad `(low, high, next low, center)`: from the ring
    /// edge and bridge side edge down to the crotch center, each row a little
    /// shorter than the one before it.
    fn crotch_rows(&mut self, v: &[JunctionVertex]) -> Vec<Vec<JunctionVertex>> {
        let k = self.rows;
        let side = self.split(v[1], v[2]);
        let left = self.split(v[0], v[3]);
        let right = self.split(v[2], v[3]);
        let mut top = vec![v[0]];
        top.extend(side);
        let top_points = top.iter().map(|&p| self.position(p)).collect::<Vec<_>>();
        let center = self.position(v[3]);
        let mut rows = vec![top];
        for r in 1..k {
            // Segments shrink evenly from the top row's k + 1 to one.
            let segments = (((k + 1) * (k - r) + k / 2) / k).max(1);
            let mut row = vec![left[r]];
            for j in 1..segments {
                let on_top = along(&top_points, j as f64 / segments as f64);
                let t = r as f64 / k as f64;
                row.push(self.push(add(on_top, scale(sub(center, on_top), t))));
            }
            row.push(right[r]);
            rows.push(row);
        }
        rows.push(vec![v[3]]);
        rows
    }
}

/// Point at fraction `t` of a polyline's length.
fn along(points: &[[f64; 3]], t: f64) -> [f64; 3] {
    let lengths = points
        .windows(2)
        .map(|w| norm(sub(w[1], w[0])))
        .collect::<Vec<_>>();
    let total = lengths.iter().sum::<f64>();
    let mut remaining = t * total;
    for (w, length) in points.windows(2).zip(&lengths) {
        if remaining <= *length && *length > 0.0 {
            return add(w[0], scale(sub(w[1], w[0]), remaining / length));
        }
        remaining -= length;
    }
    *points.last().expect("rows are nonempty")
}

fn position(rings: &[Vec<[f64; 3]>], vertices: &[[f64; 3]], vertex: JunctionVertex) -> [f64; 3] {
    match vertex {
        JunctionVertex::Ring { ring, index } => rings[ring][index],
        JunctionVertex::Skin(index) => vertices[index],
    }
}

/// Fairs the new vertices with a discrete thin-plate energy.
///
/// The energy is the sum of squared umbrella Laplacians over every new
/// vertex and every ring vertex. A ring vertex's umbrella includes its two
/// ring neighbours and one ghost neighbour inside its tube, placed against
/// the wall tangent at the ring vertex's mean skin edge length, so
/// minimizing the energy makes the skin leave each ring along its tube wall.
/// Ring vertices and ghosts are fixed; the minimizer is unique and is found
/// with conjugate gradients on the normal equations.
fn fair(
    rings: &[Vec<[f64; 3]>],
    vertices: &mut [[f64; 3]],
    faces: &[JunctionFace],
    tangents: &[Vec<[f64; 3]>],
) -> Result<(), JunctionError> {
    // Graph nodes: free vertices first, then ring vertices, then ghosts.
    let free = vertices.len();
    let mut ring_base = Vec::with_capacity(rings.len());
    let mut nodes = free;
    for ring in rings {
        ring_base.push(nodes);
        nodes += ring.len();
    }
    let fixed_rings = nodes;
    let ghost_base = nodes;
    nodes += fixed_rings - free;
    let node = |vertex: JunctionVertex| match vertex {
        JunctionVertex::Skin(index) => index,
        JunctionVertex::Ring { ring, index } => ring_base[ring] + index,
    };

    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); fixed_rings];
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

    let mut positions = vec![[0.0; 3]; nodes];
    positions[..free].copy_from_slice(vertices);
    for (ring, points) in rings.iter().enumerate() {
        for (index, point) in points.iter().enumerate() {
            let at = ring_base[ring] + index;
            positions[at] = *point;
            // Mean length of the edges into the skin sets the ghost's
            // distance, so the umbrella sees comparable spacing on both
            // sides of the ring.
            let skin = neighbours[at]
                .iter()
                .filter(|&&other| other < free)
                .map(|&other| norm(sub(vertices[other], *point)))
                .collect::<Vec<_>>();
            let spacing = if skin.is_empty() {
                neighbours[at]
                    .iter()
                    .map(|&other| norm(sub(ring_point(rings, &ring_base, other), *point)))
                    .fold(0.0, f64::max)
            } else {
                skin.iter().sum::<f64>() / skin.len() as f64
            };
            let ghost = ghost_base + (at - free);
            positions[ghost] = sub(*point, scale(tangents[ring][index], spacing));
            neighbours[at].push(ghost);
        }
    }

    // Umbrella rows for free and ring vertices: L(v) = x_v - mean(x_u).
    let rows = (0..fixed_rings)
        .map(|v| {
            let weight = 1.0 / neighbours[v].len() as f64;
            let mut row = vec![(v, 1.0)];
            row.extend(neighbours[v].iter().map(|&u| (u, -weight)));
            row
        })
        .collect::<Vec<_>>();

    #[expect(
        clippy::needless_range_loop,
        reason = "each axis indexes every node's position"
    )]
    for axis in 0..3 {
        // Minimize |A_f x_f + A_b x_b|^2 over the free coordinates.
        let mut rhs = vec![0.0; rows.len()];
        for (r, row) in rows.iter().enumerate() {
            rhs[r] = -row
                .iter()
                .filter(|(u, _)| *u >= free)
                .map(|(u, w)| w * positions[*u][axis])
                .sum::<f64>();
        }
        let initial = (0..free).map(|v| positions[v][axis]).collect::<Vec<_>>();
        let solution = least_squares(&rows, free, &rhs, initial);
        for (v, value) in solution.into_iter().enumerate() {
            positions[v][axis] = value;
        }
    }
    if !positions[..free].iter().flatten().all(|c| c.is_finite()) {
        return Err(JunctionError::SmoothingDiverged);
    }
    vertices.copy_from_slice(&positions[..free]);
    Ok(())
}

fn ring_point(rings: &[Vec<[f64; 3]>], ring_base: &[usize], node: usize) -> [f64; 3] {
    let ring = ring_base
        .iter()
        .rposition(|&base| base <= node)
        .expect("ring nodes follow the free nodes");
    rings[ring][node - ring_base[ring]]
}

/// Conjugate gradients on the normal equations `AᵀA x = Aᵀb`, where `A` is
/// the sparse `rows` restricted to the first `free` columns: the least-squares
/// fairing energy has more rows (ring umbrellas) than unknowns.
fn least_squares(
    rows: &[Vec<(usize, f64)>],
    free: usize,
    rhs: &[f64],
    initial: Vec<f64>,
) -> Vec<f64> {
    let apply = |x: &[f64]| {
        rows.iter()
            .map(|row| {
                row.iter()
                    .filter(|(u, _)| *u < free)
                    .map(|(u, w)| w * x[*u])
                    .sum::<f64>()
            })
            .collect::<Vec<_>>()
    };
    let apply_transpose = |y: &[f64]| {
        let mut out = vec![0.0; free];
        for (row, value) in rows.iter().zip(y) {
            for &(u, w) in row {
                if u < free {
                    out[u] += w * value;
                }
            }
        }
        out
    };
    let mut x = initial;
    let ax = apply(&x);
    let residual = rhs.iter().zip(&ax).map(|(b, a)| b - a).collect::<Vec<_>>();
    let mut r = apply_transpose(&residual);
    let mut p = r.clone();
    let mut rr = dot_n(&r, &r);
    let scale = dot_n(&apply_transpose(rhs), &apply_transpose(rhs)).max(f64::MIN_POSITIVE);
    for _ in 0..4 * free.max(1) {
        if rr <= 1e-26 * scale {
            break;
        }
        let ap = apply(&p);
        let step = rr / dot_n(&ap, &ap).max(f64::MIN_POSITIVE);
        let atap = apply_transpose(&ap);
        for v in 0..free {
            x[v] += step * p[v];
            r[v] -= step * atap[v];
        }
        let next = dot_n(&r, &r);
        let beta = next / rr;
        rr = next;
        for v in 0..free {
            p[v] = r[v] + beta * p[v];
        }
    }
    x
}

/// Conjugate gradients for a symmetric positive definite system `A x = b`,
/// where `A` is the sparse `rows` (one per unknown) restricted to the first
/// `free` columns.
///
/// Used for the Laplacian systems of the chart and layer weights, which are
/// SPD as given; the normal equations of [`least_squares`] would square
/// their condition number. The iteration is a fixed sequence of
/// floating-point operations, so results are deterministic.
pub(super) fn conjugate_gradient(
    rows: &[Vec<(usize, f64)>],
    free: usize,
    rhs: &[f64],
    initial: Vec<f64>,
) -> Vec<f64> {
    let apply = |x: &[f64]| {
        rows.iter()
            .map(|row| {
                row.iter()
                    .filter(|(u, _)| *u < free)
                    .map(|(u, w)| w * x[*u])
                    .sum::<f64>()
            })
            .collect::<Vec<_>>()
    };
    let mut x = initial;
    let ax = apply(&x);
    let mut r = rhs.iter().zip(&ax).map(|(b, a)| b - a).collect::<Vec<_>>();
    let mut p = r.clone();
    let mut rr = dot_n(&r, &r);
    let scale = dot_n(rhs, rhs).max(f64::MIN_POSITIVE);
    for _ in 0..4 * free.max(1) {
        if rr <= 1e-26 * scale {
            break;
        }
        let ap = apply(&p);
        let step = rr / dot_n(&p, &ap).max(f64::MIN_POSITIVE);
        for v in 0..free {
            x[v] += step * p[v];
            r[v] -= step * ap[v];
        }
        let next = dot_n(&r, &r);
        let beta = next / rr;
        rr = next;
        for v in 0..free {
            p[v] = r[v] + beta * p[v];
        }
    }
    x
}

fn dot_n(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Unit wall tangents along each ring's inward axis, for rings whose tube
/// walls are not known.
pub(super) fn axial_tangents(rings: &[Vec<[f64; 3]>], axes: &[[f64; 3]]) -> Vec<Vec<[f64; 3]>> {
    rings
        .iter()
        .zip(axes)
        .map(|(ring, axis)| vec![scale(*axis, -1.0); ring.len()])
        .collect()
}

/// Checks caller tangents and normalizes them: one per ring vertex, finite,
/// and pointing from the ring toward the junction side of its plane.
pub(super) fn checked_tangents(
    tangents: &[Vec<[f64; 3]>],
    rings: &[Vec<[f64; 3]>],
    axes: &[[f64; 3]],
) -> Result<Vec<Vec<[f64; 3]>>, JunctionError> {
    if tangents.len() != rings.len() {
        return Err(JunctionError::InvalidSmoothing { ring: None });
    }
    tangents
        .iter()
        .zip(rings)
        .zip(axes)
        .enumerate()
        .map(|(ring, ((tangents, points), axis))| {
            if tangents.len() != points.len() {
                return Err(JunctionError::InvalidSmoothing { ring: Some(ring) });
            }
            tangents
                .iter()
                .map(|t| {
                    normalize(*t)
                        .filter(|t| t.iter().all(|c| c.is_finite()) && dot(*t, *axis) < 0.0)
                        .ok_or(JunctionError::InvalidSmoothing { ring: Some(ring) })
                })
                .collect()
        })
        .collect()
}
