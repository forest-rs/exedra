// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic incremental 3D Delaunay tetrahedralization.
//!
//! A [`Tetrahedralization`] starts from a rectilinear [grid](Tetrahedralization::grid)
//! and grows by [inserting](Tetrahedralization::insert) points inside it. Every
//! decision uses the exact-sign [`orient3d`] and [`insphere`] predicates from
//! [`exedra_math::predicates`], so the result is a valid Delaunay
//! tetrahedralization for any finite input, and identical input bits produce
//! identical tetrahedra on every platform.
//!
//! The crate knows nothing about meshes, fields, or tolerances. Inputs are
//! coordinates; outputs are [`VertexId`]s in insertion order and
//! [`TetId`]s with explicit adjacency.
//!
//! ## Construction
//!
//! [`Tetrahedralization::grid`] takes one strictly increasing coordinate list
//! per axis and splits every cell into the six Freudenthal tetrahedra around
//! its main diagonal. Each cell is an exact axis-aligned box, so its corners
//! are exactly cospherical whatever the coordinates round to; the split is
//! therefore exactly Delaunay without jitter or perturbation, and its hull is
//! the grid's box.
//!
//! [`Tetrahedralization::insert`] adds one point by Bowyer–Watson: locate the
//! tetrahedron holding it with a deterministic stochastic walk, grow the cavity
//! of tetrahedra whose circumspheres strictly contain it, and connect the
//! cavity's boundary to the new vertex. A point on the hull splits the hull
//! face it lies on. Points outside the hull are refused, and a point equal to
//! an existing vertex returns that vertex.
//!
//! ## Orientation
//!
//! Every tetrahedron's vertices `[a, b, c, d]` satisfy
//! `orient3d(a, b, c, d) == Below`, Shewchuk's positive orientation, which is
//! also the orientation [`insphere`] expects. [`Tetrahedralization::face`]
//! returns the face opposite vertex `i` wound counter-clockwise seen from
//! outside the tetrahedron, so faces between two classes of tetrahedra form a
//! consistently oriented surface.
//!
//! # Example
//!
//! ```
//! use exedra_tetrahedralize::Tetrahedralization;
//!
//! let axis = [0.0, 1.0];
//! let mut tets = Tetrahedralization::grid(&axis, &axis, &axis).expect("valid grid");
//! assert_eq!(tets.tetrahedron_count(), 6);
//!
//! let center = tets.insert([0.5, 0.5, 0.5]).expect("inside the box");
//! assert_eq!(tets.point(center), [0.5, 0.5, 0.5]);
//! assert!(tets.validate().is_ok());
//! ```

#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

use exedra_math::predicates::{InSphere, Orientation3d, insphere, orient3d};

/// Index of a vertex, stable for the life of the tetrahedralization.
///
/// Grid vertices come first, numbered `(i * ny + j) * nz + k`; inserted
/// vertices follow in insertion order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VertexId(u32);

impl VertexId {
    /// Position of this vertex in [`Tetrahedralization::points`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of a tetrahedron.
///
/// Valid until the next insertion, which may remove it and reuse its index.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TetId(u32);

impl TetId {
    /// Dense index of this tetrahedron's slot, below
    /// [`Tetrahedralization::slot_count`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Where a point lies in a tetrahedralization.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// Inside or on the boundary of this tetrahedron.
    Tetrahedron(TetId),
    /// Exactly at this vertex.
    Vertex(VertexId),
    /// Outside the hull.
    Outside,
}

/// Why a grid or an insertion was refused.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TetError {
    /// A coordinate list has fewer than two values, or values that are not
    /// finite and strictly increasing. `axis` is 0, 1 or 2.
    InvalidAxis {
        /// The offending axis.
        axis: usize,
    },
    /// The grid or insertion would exceed `u32` vertex or tetrahedron indices.
    TooLarge,
    /// The point has a non-finite coordinate.
    NonFinite,
    /// The point lies outside the hull, where insertion would change it.
    OutsideHull,
}

impl fmt::Display for TetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAxis { axis } => write!(
                f,
                "grid axis {axis} needs at least two finite, strictly increasing coordinates"
            ),
            Self::TooLarge => f.write_str("tetrahedralization exceeds u32 indices"),
            Self::NonFinite => f.write_str("point has a non-finite coordinate"),
            Self::OutsideHull => f.write_str("point lies outside the hull"),
        }
    }
}

impl core::error::Error for TetError {}

/// A broken invariant found by [`Tetrahedralization::validate`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Violation {
    /// A tetrahedron is not positively oriented.
    Orientation(TetId),
    /// Two tetrahedra disagree about sharing a face.
    Adjacency(TetId),
    /// A neighbor's opposite vertex lies strictly inside a circumsphere.
    NotDelaunay(TetId),
    /// A vertex's recorded tetrahedron is dead or does not contain it.
    VertexStar(VertexId),
}

/// Work done by a tetrahedralization so far.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TetStats {
    /// Points inserted as new vertices.
    pub insertions: usize,
    /// Insertions that matched an existing vertex.
    pub coincident: usize,
    /// Tetrahedra visited while locating points.
    pub walk_steps: usize,
    /// Tetrahedra removed into cavities.
    pub cavity_tetrahedra: usize,
    /// The largest single cavity.
    pub largest_cavity: usize,
    /// Insphere tests made while growing cavities.
    pub insphere_tests: usize,
}

const NONE: u32 = u32::MAX;

/// The face opposite vertex `i`, wound so that `orient3d(face, v[i])` is
/// `Below`: its counter-clockwise normal points away from `v[i]`.
const FACES: [[usize; 3]; 4] = [[1, 3, 2], [0, 2, 3], [0, 3, 1], [0, 1, 2]];

#[derive(Copy, Clone, Debug)]
struct Tet {
    vertices: [u32; 4],
    neighbors: [u32; 4],
}

impl Tet {
    const DEAD: Self = Self {
        vertices: [NONE; 4],
        neighbors: [NONE; 4],
    };

    fn alive(&self) -> bool {
        self.vertices[0] != NONE
    }
}

/// An incremental Delaunay tetrahedralization of points in a box.
#[derive(Clone, Debug)]
pub struct Tetrahedralization {
    axes: [Vec<f64>; 3],
    points: Vec<[f64; 3]>,
    /// A live tetrahedron at each vertex, where walks start.
    vertex_tets: Vec<u32>,
    tets: Vec<Tet>,
    free: Vec<u32>,
    live: usize,
    hint: u32,
    rng: u64,
    stats: TetStats,
    // Per-insertion scratch, kept to avoid reallocating.
    marks: Vec<u32>,
    epoch: u32,
    cavity: Vec<u32>,
    boundary: Vec<(u32, usize)>,
    links: Vec<([u32; 2], u32, usize)>,
}

impl Tetrahedralization {
    /// Tetrahedralizes the rectilinear grid with these coordinates per axis.
    ///
    /// Each list needs at least two finite, strictly increasing values. Every
    /// cell splits into six tetrahedra around its diagonal from its least to
    /// its greatest corner.
    pub fn grid(x: &[f64], y: &[f64], z: &[f64]) -> Result<Self, TetError> {
        for (axis, values) in [x, y, z].into_iter().enumerate() {
            if values.len() < 2
                || values.iter().any(|v| !v.is_finite())
                || values.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(TetError::InvalidAxis { axis });
            }
        }
        let [nx, ny, nz] = [x.len(), y.len(), z.len()];
        let cells = (nx - 1)
            .checked_mul(ny - 1)
            .and_then(|n| n.checked_mul(nz - 1))
            .and_then(|n| n.checked_mul(6))
            .ok_or(TetError::TooLarge)?;
        let vertex_count = nx
            .checked_mul(ny)
            .and_then(|n| n.checked_mul(nz))
            .ok_or(TetError::TooLarge)?;
        if u32::try_from(cells).is_err() || u32::try_from(vertex_count).is_err() {
            return Err(TetError::TooLarge);
        }

        let mut points = Vec::with_capacity(vertex_count);
        for &px in x {
            for &py in y {
                for &pz in z {
                    points.push([px, py, pz]);
                }
            }
        }
        let id = |i: usize, j: usize, k: usize| index_u32((i * ny + j) * nz + k);
        let mut tets = Vec::with_capacity(cells);
        for i in 0..nx - 1 {
            for j in 0..ny - 1 {
                for k in 0..nz - 1 {
                    for order in [
                        [0, 1, 2],
                        [0, 2, 1],
                        [1, 0, 2],
                        [1, 2, 0],
                        [2, 0, 1],
                        [2, 1, 0],
                    ] {
                        let mut corner = [i, j, k];
                        let mut vertices = [0; 4];
                        vertices[0] = id(i, j, k);
                        for (step, axis) in order.into_iter().enumerate() {
                            corner[axis] += 1;
                            vertices[step + 1] = id(corner[0], corner[1], corner[2]);
                        }
                        let [a, b, c, d] = vertices.map(|v| points[v as usize]);
                        if orient3d(a, b, c, d) == Orientation3d::Above {
                            vertices.swap(0, 1);
                        }
                        tets.push(Tet {
                            vertices,
                            neighbors: [NONE; 4],
                        });
                    }
                }
            }
        }
        link_faces(&mut tets);
        let mut vertex_tets = alloc::vec![NONE; points.len()];
        for (index, tet) in tets.iter().enumerate() {
            for &vertex in &tet.vertices {
                vertex_tets[vertex as usize] = index_u32(index);
            }
        }

        let live = tets.len();
        Ok(Self {
            axes: [x.to_vec(), y.to_vec(), z.to_vec()],
            points,
            vertex_tets,
            marks: alloc::vec![0; live],
            tets,
            free: Vec::new(),
            live,
            hint: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
            stats: TetStats::default(),
            epoch: 0,
            cavity: Vec::new(),
            boundary: Vec::new(),
            links: Vec::new(),
        })
    }

    /// Every vertex position, indexed by [`VertexId::index`].
    #[must_use]
    pub fn points(&self) -> &[[f64; 3]] {
        &self.points
    }

    /// The position of `vertex`.
    #[must_use]
    pub fn point(&self, vertex: VertexId) -> [f64; 3] {
        self.points[vertex.index()]
    }

    /// Number of live tetrahedra.
    #[must_use]
    pub fn tetrahedron_count(&self) -> usize {
        self.live
    }

    /// Number of tetrahedron slots, live or free; every [`TetId::index`] is
    /// below it, so callers can size per-tetrahedron tables.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.tets.len()
    }

    /// The live tetrahedra, in slot order.
    pub fn tetrahedra(&self) -> impl Iterator<Item = TetId> + '_ {
        self.tets
            .iter()
            .enumerate()
            .filter(|(_, tet)| tet.alive())
            .map(|(index, _)| TetId(index_u32(index)))
    }

    /// The vertices of `tet`, positively oriented.
    #[must_use]
    pub fn vertices(&self, tet: TetId) -> [VertexId; 4] {
        self.tets[tet.index()].vertices.map(VertexId)
    }

    /// The neighbor across the face opposite each vertex, or `None` on the
    /// hull.
    #[must_use]
    pub fn neighbors(&self, tet: TetId) -> [Option<TetId>; 4] {
        self.tets[tet.index()]
            .neighbors
            .map(|n| (n != NONE).then_some(TetId(n)))
    }

    /// The face of `tet` opposite its vertex `i`, counter-clockwise seen from
    /// outside `tet`.
    #[must_use]
    pub fn face(&self, tet: TetId, i: usize) -> [VertexId; 3] {
        let vertices = self.tets[tet.index()].vertices;
        FACES[i].map(|k| VertexId(vertices[k]))
    }

    /// Work done so far.
    #[must_use]
    pub fn stats(&self) -> TetStats {
        self.stats
    }

    /// Finds where `point` lies, walking from the nearest grid vertex.
    #[must_use]
    pub fn locate(&self, point: [f64; 3]) -> Location {
        if point.iter().any(|v| !v.is_finite()) {
            return Location::Outside;
        }
        let mut rng = self.rng;
        let (location, _) = self.walk(point, &mut rng);
        location
    }

    /// Inserts `point`, returning its vertex.
    ///
    /// A point equal to an existing vertex returns that vertex and changes
    /// nothing. A point on the hull splits the hull face it lies on.
    pub fn insert(&mut self, point: [f64; 3]) -> Result<VertexId, TetError> {
        if point.iter().any(|v| !v.is_finite()) {
            return Err(TetError::NonFinite);
        }
        let mut rng = self.rng;
        let (location, steps) = self.walk(point, &mut rng);
        self.rng = rng;
        self.stats.walk_steps += steps;
        let start = match location {
            Location::Outside => return Err(TetError::OutsideHull),
            Location::Vertex(vertex) => {
                self.stats.coincident += 1;
                return Ok(vertex);
            }
            Location::Tetrahedron(tet) => tet.0,
        };
        let id = u32::try_from(self.points.len())
            .ok()
            .filter(|&id| id != NONE)
            .ok_or(TetError::TooLarge)?;
        self.points.push(point);
        self.vertex_tets.push(NONE);
        self.carve(start, point);
        self.fill(id, point)?;
        self.stats.insertions += 1;
        Ok(VertexId(id))
    }

    /// Checks orientation, adjacency, and the local Delaunay property of
    /// every tetrahedron, and that every vertex records a tetrahedron at it.
    ///
    /// This visits every face and costs as much as the tetrahedralization is
    /// large; it is meant for tests and debugging.
    pub fn validate(&self) -> Result<(), Violation> {
        for (index, tet) in self.tets.iter().enumerate() {
            if !tet.alive() {
                continue;
            }
            let id = TetId(index_u32(index));
            let [a, b, c, d] = tet.vertices.map(|v| self.points[v as usize]);
            if orient3d(a, b, c, d) != Orientation3d::Below {
                return Err(Violation::Orientation(id));
            }
            for (face, &neighbor) in FACES.iter().zip(&tet.neighbors) {
                if neighbor == NONE {
                    continue;
                }
                let other = self
                    .tets
                    .get(neighbor as usize)
                    .filter(|other| other.alive())
                    .ok_or(Violation::Adjacency(id))?;
                let Some(back) = other.neighbors.iter().position(|&n| n as usize == index) else {
                    return Err(Violation::Adjacency(id));
                };
                let mut mine = face.map(|k| tet.vertices[k]);
                let mut theirs = FACES[back].map(|k| other.vertices[k]);
                mine.sort_unstable();
                theirs.sort_unstable();
                if mine != theirs {
                    return Err(Violation::Adjacency(id));
                }
                let opposite = self.points[other.vertices[back] as usize];
                if insphere(a, b, c, d, opposite) == InSphere::Inside {
                    return Err(Violation::NotDelaunay(id));
                }
            }
        }
        for (vertex, &tet) in self.vertex_tets.iter().enumerate() {
            let recorded = self.tets.get(tet as usize);
            if !recorded.is_some_and(|tet| tet.alive() && tet.vertices.contains(&index_u32(vertex)))
            {
                return Err(Violation::VertexStar(VertexId(index_u32(vertex))));
            }
        }
        Ok(())
    }

    /// Visibility walk toward `point` with a randomized face order, which
    /// terminates in a Delaunay tetrahedralization. It starts at the grid
    /// vertex nearest `point`, so its length depends on the local density
    /// rather than on the order of insertions.
    fn walk(&self, point: [f64; 3], rng: &mut u64) -> (Location, usize) {
        let mut tet = self.vertex_tets[self.nearest_grid_vertex(point)];
        if !self.tets[tet as usize].alive() {
            tet = self.hint;
        }
        let mut steps = 0;
        'walk: loop {
            steps += 1;
            let first = usize::try_from(xorshift(rng) % 4).unwrap_or(0);
            let current = self.tets[tet as usize];
            for turn in 0..4 {
                let i = (first + turn) % 4;
                let [a, b, c] = FACES[i].map(|k| self.points[current.vertices[k] as usize]);
                if orient3d(a, b, c, point) == Orientation3d::Above {
                    let next = current.neighbors[i];
                    if next == NONE {
                        return (Location::Outside, steps);
                    }
                    tet = next;
                    continue 'walk;
                }
            }
            if let Some(&vertex) = current
                .vertices
                .iter()
                .find(|&&v| self.points[v as usize] == point)
            {
                return (Location::Vertex(VertexId(vertex)), steps);
            }
            return (Location::Tetrahedron(TetId(tet)), steps);
        }
    }

    /// The grid vertex nearest `point`, clamped to the grid.
    fn nearest_grid_vertex(&self, point: [f64; 3]) -> usize {
        let [i, j, k] = [0, 1, 2].map(|axis| {
            let values = &self.axes[axis];
            let above = values.partition_point(|&v| v < point[axis]);
            if above == 0 {
                0
            } else if above == values.len()
                || point[axis] - values[above - 1] < values[above] - point[axis]
            {
                above - 1
            } else {
                above
            }
        });
        let [_, ny, nz] = self.axes.each_ref().map(Vec::len);
        (i * ny + j) * nz + k
    }

    /// Grows the cavity of tetrahedra whose circumspheres strictly contain
    /// `point`, starting from the tetrahedron that holds it, and records the
    /// cavity's boundary faces.
    fn carve(&mut self, start: u32, point: [f64; 3]) {
        self.epoch = self.epoch.wrapping_add(2);
        if self.epoch < 2 {
            self.marks.fill(0);
            self.epoch = 2;
        }
        let (inside, outside) = (self.epoch, self.epoch + 1);
        self.cavity.clear();
        self.boundary.clear();
        self.cavity.push(start);
        self.marks[start as usize] = inside;
        let mut next = 0;
        while let Some(&tet) = self.cavity.get(next) {
            next += 1;
            for i in 0..4 {
                let neighbor = self.tets[tet as usize].neighbors[i];
                if neighbor != NONE && self.marks[neighbor as usize] != outside {
                    if self.marks[neighbor as usize] == inside {
                        continue;
                    }
                    self.stats.insphere_tests += 1;
                    let [a, b, c, d] = self.tets[neighbor as usize]
                        .vertices
                        .map(|v| self.points[v as usize]);
                    if insphere(a, b, c, d, point) == InSphere::Inside {
                        self.marks[neighbor as usize] = inside;
                        self.cavity.push(neighbor);
                        continue;
                    }
                    self.marks[neighbor as usize] = outside;
                }
                self.boundary.push((tet, i));
            }
        }
        self.stats.cavity_tetrahedra += self.cavity.len();
        self.stats.largest_cavity = self.stats.largest_cavity.max(self.cavity.len());
    }

    /// Replaces the cavity with tetrahedra joining its boundary to vertex
    /// `id`. Hull faces that `point` lies on are split rather than coned.
    fn fill(&mut self, id: u32, point: [f64; 3]) -> Result<(), TetError> {
        self.links.clear();
        let mut first = NONE;
        for b in 0..self.boundary.len() {
            let (old, i) = self.boundary[b];
            let source = self.tets[old as usize];
            let outside = source.neighbors[i];
            if outside == NONE {
                let [a, b, c] = FACES[i].map(|k| self.points[source.vertices[k] as usize]);
                if orient3d(a, b, c, point) == Orientation3d::Coplanar {
                    continue;
                }
            }
            let mut vertices = source.vertices;
            vertices[i] = id;
            let mut neighbors = [NONE; 4];
            neighbors[i] = outside;
            let tet = self.allocate(Tet {
                vertices,
                neighbors,
            })?;
            debug_assert_eq!(
                {
                    let [a, b, c, d] = vertices.map(|v| self.points[v as usize]);
                    orient3d(a, b, c, d)
                },
                Orientation3d::Below,
                "cavity is star-shaped from the new vertex"
            );
            if outside != NONE {
                for slot in &mut self.tets[outside as usize].neighbors {
                    if *slot == old {
                        *slot = tet;
                    }
                }
            }
            for &vertex in &vertices {
                self.vertex_tets[vertex as usize] = tet;
            }
            for j in (0..4).filter(|&j| j != i) {
                let mut edge = [0; 2];
                let mut n = 0;
                for (k, &v) in vertices.iter().enumerate() {
                    if k != i && k != j {
                        edge[n] = v;
                        n += 1;
                    }
                }
                edge.sort_unstable();
                self.links.push((edge, tet, j));
            }
            if first == NONE {
                first = tet;
            }
        }
        // Faces through the new vertex pair up by their other two vertices;
        // a face left unpaired lies in a hull face the new vertex split.
        self.links.sort_unstable_by_key(|&(edge, _, _)| edge);
        let mut k = 0;
        while k < self.links.len() {
            let (edge, tet, slot) = self.links[k];
            if let Some(&(other_edge, other, other_slot)) = self.links.get(k + 1)
                && other_edge == edge
            {
                self.tets[tet as usize].neighbors[slot] = other;
                self.tets[other as usize].neighbors[other_slot] = tet;
                k += 2;
            } else {
                k += 1;
            }
        }
        for c in 0..self.cavity.len() {
            let old = self.cavity[c];
            self.tets[old as usize] = Tet::DEAD;
            self.free.push(old);
        }
        self.live -= self.cavity.len();
        self.hint = first;
        Ok(())
    }

    fn allocate(&mut self, tet: Tet) -> Result<u32, TetError> {
        self.live += 1;
        // Slots freed by this insertion's own cavity are released only after
        // it is filled, so neighbor pointers to them stay unambiguous.
        if let Some(index) = self.free.pop() {
            self.tets[index as usize] = tet;
            return Ok(index);
        }
        let index = u32::try_from(self.tets.len())
            .ok()
            .filter(|&index| index != NONE)
            .ok_or(TetError::TooLarge)?;
        self.tets.push(tet);
        self.marks.push(0);
        Ok(index)
    }
}

/// Pairs up the faces of freshly built tetrahedra; unpaired faces are hull.
fn link_faces(tets: &mut [Tet]) {
    let mut faces: Vec<([u32; 3], u32, usize)> = Vec::with_capacity(tets.len() * 4);
    for (index, tet) in tets.iter().enumerate() {
        for (i, face) in FACES.iter().enumerate() {
            let mut key = face.map(|k| tet.vertices[k]);
            key.sort_unstable();
            faces.push((key, index_u32(index), i));
        }
    }
    faces.sort_unstable_by_key(|&(key, _, _)| key);
    let mut k = 0;
    while k < faces.len() {
        let (key, tet, slot) = faces[k];
        if let Some(&(other_key, other, other_slot)) = faces.get(k + 1)
            && other_key == key
        {
            tets[tet as usize].neighbors[slot] = other;
            tets[other as usize].neighbors[other_slot] = tet;
            k += 2;
        } else {
            k += 1;
        }
    }
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Narrows an index already checked to fit `u32`.
fn index_u32(index: usize) -> u32 {
    u32::try_from(index).expect("indices are checked to fit u32")
}

#[cfg(test)]
mod tests;
