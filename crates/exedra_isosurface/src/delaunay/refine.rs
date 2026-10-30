// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Restricted Delaunay refinement over one set of protected creases.
//!
//! Each tetrahedron takes the field's sign at its orthocenter, the center of
//! its weighted circumsphere. A face between an inside and an outside
//! tetrahedron is restricted: its dual edge, between the two orthocenters,
//! crosses the surface. The restricted faces form the mesh. A face that is
//! too large, badly shaped, too far from the surface, or folded against its
//! neighbors is refined by inserting the point where its dual edge meets
//! the surface (Chew; Boissonnat and Oudot), and every surface vertex's
//! faces must close into one disk around it.

use alloc::collections::BinaryHeap;
use alloc::vec::Vec;

use hashbrown::{HashMap, HashSet};

use exedra_math::{cross, dot, sub};
use exedra_tetrahedralize::{TetId, Tetrahedralization, VertexId};

use super::crease::{BallId, Creases, Protection};
use super::sample::{Sampler, circumcenter, distance, orthocenter, unit};
use super::{Refinements, Settings};

/// Which side of the surface a vertex lies on, or on it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Side {
    Outside,
    On,
}

/// An orthocenter and the field's value there.
#[derive(Copy, Clone, Debug)]
struct Center {
    point: [f64; 3],
    value: f64,
}

/// A restricted face: the tetrahedron inside, the index of the face in it,
/// and the tetrahedron outside.
#[derive(Copy, Clone, Debug)]
struct Restricted {
    inside: TetId,
    face: usize,
    outside: TetId,
}

/// Why a face is refined.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Reason {
    /// A corner is not on the surface.
    OffSurface,
    Size,
    Shape,
    Distance,
    Folded,
    Disk,
}

/// A face queued for refinement, valid while its inside tetrahedron's slot
/// keeps the generation it had.
#[derive(Copy, Clone, Debug)]
struct Candidate {
    tet: TetId,
    face: usize,
    generation: u32,
    /// Where to refine, once the face has been judged in full; a face bad
    /// in size or shape alone is judged only when its turn comes, since
    /// many such faces are gone by then.
    judged: Option<([f64; 3], Reason)>,
}

/// The outcome of judging a restricted face.
enum Verdict {
    Good,
    Bad(f64, [f64; 3], Reason),
    /// Refinement needs a point inside this ball, which can still split.
    Protected(BallId),
}

/// One pass of refinement: its surface, the balls it found too large, and
/// its work.
pub(super) struct Pass {
    pub(super) positions: Vec<[f64; 3]>,
    pub(super) triangles: Vec<[u32; 3]>,
    /// Crease segments that are edges of the surface.
    pub(super) creases: Vec<[u32; 2]>,
    /// Crease segments that are not, though their balls could split.
    pub(super) broken_creases: usize,
    /// Crease segments that are not, with balls at the minimum step.
    pub(super) unresolved_creases: usize,
    /// Surface vertices on no face.
    pub(super) isolated_vertices: usize,
    pub(super) requests: Vec<BallId>,
    pub(super) refinements: Refinements,
    pub(super) disk_failures: usize,
    pub(super) hit_point_limit: bool,
    pub(super) tetrahedra: usize,
}

/// The tetrahedralization being refined, with what the pass knows about its
/// vertices and tetrahedron slots.
struct Scaffold<'a> {
    sampler: &'a Sampler<'a>,
    settings: &'a Settings,
    creases: &'a Creases,
    guard: Protection,
    tets: Tetrahedralization,
    sides: Vec<Side>,
    /// Per vertex: the protecting ball of a crease sample.
    balls: Vec<Option<BallId>>,
    normals: Vec<Option<[f64; 3]>>,
    /// Surface points found for faces, by their sorted corners. A face's
    /// dual edge lies on one line whatever tetrahedra share it, so a point
    /// found once holds while the edge still reaches it.
    surface_points: HashMap<[VertexId; 3], [f64; 3]>,
    /// Per slot: the walk that last visited it, for finding stars.
    visits: Vec<u32>,
    epoch: u32,
    scratch: Vec<TetId>,
    /// Per slot: the orthocenter, once computed for the slot's generation.
    centers: Vec<Option<Center>>,
    /// Scratch for evaluating many orthocenters in one query.
    batch: (Vec<TetId>, Vec<[f64; 3]>, Vec<f64>),
    generations: Vec<u32>,
    /// Whether this is the last pass, which refines to the end whatever
    /// balls it finds too large, rather than stopping to split them.
    last: bool,
    /// Seeds left out for crowding, held for surface they turn out to be
    /// needed for.
    spare: Vec<[f64; 3]>,
}

/// Meshes the surface within `hull`, protecting `creases` and starting from
/// `seeds` on the surface.
pub(super) fn refine(
    sampler: &Sampler<'_>,
    settings: &Settings,
    hull: [[f64; 2]; 3],
    creases: &Creases,
    seeds: &[[f64; 3]],
    last: bool,
) -> Pass {
    let tets =
        Tetrahedralization::grid(&hull[0], &hull[1], &hull[2]).expect("a finite, ordered hull");
    let count = tets.points().len();
    let mut scaffold = Scaffold {
        sampler,
        settings,
        creases,
        guard: Protection::new(settings.max_step),
        tets,
        sides: alloc::vec![Side::Outside; count],
        balls: alloc::vec![None; count],
        normals: alloc::vec![None; count],
        surface_points: HashMap::new(),
        visits: Vec::new(),
        epoch: 0,
        scratch: Vec::new(),
        centers: Vec::new(),
        batch: (Vec::new(), Vec::new(), Vec::new()),
        generations: Vec::new(),
        last,
        spare: Vec::new(),
    };
    for (center, radius, ball) in creases.balls(settings) {
        scaffold.guard.add(center, radius, ball);
        if let Ok(v) = scaffold.tets.insert_weighted(center, radius * radius) {
            scaffold.added(v, Some(ball));
        }
    }
    for &seed in seeds {
        if scaffold.guard.covering(seed).is_some() {
            continue;
        }
        if scaffold.crowded(seed, settings.seed_spacing) {
            scaffold.spare.push(seed);
        } else if let Ok(v) = scaffold.tets.insert(seed) {
            scaffold.added(v, None);
        }
    }
    scaffold.run()
}

impl Scaffold<'_> {
    /// Records a vertex the last insertion made, if it is new.
    fn added(&mut self, v: VertexId, ball: Option<BallId>) -> bool {
        if v.index() < self.sides.len() {
            return false;
        }
        self.sides.push(Side::On);
        self.balls.push(ball);
        self.normals.push(None);
        for &t in self.tets.new_tetrahedra() {
            let slot = t.index();
            if slot >= self.centers.len() {
                self.centers.resize(slot + 1, None);
                self.generations.resize(slot + 1, 0);
            }
            self.centers[slot] = None;
            self.generations[slot] = self.generations[slot].wrapping_add(1);
        }
        true
    }

    /// Whether a surface vertex of the tetrahedron holding `p` lies within
    /// `radius` of it.
    fn crowded(&self, p: [f64; 3], radius: f64) -> bool {
        match self.tets.locate(p) {
            exedra_tetrahedralize::Location::Tetrahedron(t) => {
                self.tets.vertices(t).iter().any(|&v| {
                    self.sides[v.index()] == Side::On && distance(self.tets.point(v), p) < radius
                })
            }
            _ => true,
        }
    }

    fn center(&mut self, t: TetId) -> Center {
        let slot = t.index();
        if slot >= self.centers.len() {
            self.centers.resize(slot + 1, None);
            self.generations.resize(slot + 1, 0);
        }
        if let Some(center) = self.centers[slot] {
            return center;
        }
        let point = self.orthocenter(t);
        let center = Center {
            point,
            value: self.sampler.value(point),
        };
        self.centers[slot] = Some(center);
        center
    }

    fn orthocenter(&self, t: TetId) -> [f64; 3] {
        let v = self.tets.vertices(t);
        orthocenter(
            v.map(|v| self.tets.point(v)),
            v.map(|v| self.tets.weight(v)),
        )
    }

    /// Computes the centers of `tets` not yet known, and the field there in
    /// one query, which costs a compiled field far less than one per point.
    fn prefetch_centers(&mut self, tets: &[TetId]) {
        let (mut wanted, mut points, mut values) = core::mem::take(&mut self.batch);
        wanted.clear();
        points.clear();
        for &t in tets {
            let slot = t.index();
            if slot >= self.centers.len() {
                self.centers.resize(slot + 1, None);
                self.generations.resize(slot + 1, 0);
            }
            if self.centers[slot].is_none() {
                wanted.push(t);
                points.push(self.orthocenter(t));
            }
        }
        values.clear();
        values.resize(points.len(), 0.0);
        self.sampler.values(&points, &mut values);
        for ((t, &point), &value) in wanted.iter().zip(&points).zip(&values) {
            self.centers[t.index()] = Some(Center { point, value });
        }
        self.batch = (wanted, points, values);
    }

    /// The restricted face across face `i` of `t`, if its two tetrahedra
    /// lie on opposite sides.
    fn restricted(&mut self, t: TetId, i: usize) -> Option<Restricted> {
        let neighbor = self.tets.neighbors(t)[i]?;
        let (a, b) = (
            self.center(t).value < 0.0,
            self.center(neighbor).value < 0.0,
        );
        if a == b {
            return None;
        }
        if a {
            return Some(Restricted {
                inside: t,
                face: i,
                outside: neighbor,
            });
        }
        let j = self
            .tets
            .neighbors(neighbor)
            .iter()
            .position(|&n| n == Some(t))?;
        Some(Restricted {
            inside: neighbor,
            face: j,
            outside: t,
        })
    }

    fn face(&self, r: Restricted) -> [VertexId; 3] {
        self.tets.face(r.inside, r.face)
    }

    fn on_crease(&self, face: [VertexId; 3]) -> bool {
        face.iter().any(|v| self.balls[v.index()].is_some())
    }

    /// The field's unit normal at a vertex, computed once.
    fn normal(&mut self, v: VertexId) -> [f64; 3] {
        if let Some(n) = self.normals[v.index()] {
            return n;
        }
        let n = unit(self.sampler.gradient(self.tets.point(v)).gradient).unwrap_or([0.0; 3]);
        self.normals[v.index()] = Some(n);
        n
    }

    /// Where the face's dual edge meets the surface.
    fn surface_center(&mut self, r: Restricted) -> Option<[f64; 3]> {
        let face = self.face(r);
        let p = face.map(|v| self.tets.point(v));
        let (inside, outside) = (self.center(r.inside), self.center(r.outside));
        let mut key = face;
        key.sort_unstable();
        if let Some(&c) = self.surface_points.get(&key) {
            let ab = sub(outside.point, inside.point);
            let t = dot(sub(c, inside.point), ab) / dot(ab, ab);
            if (0.0..=1.0).contains(&t) {
                return Some(c);
            }
        }
        let quick = if self.on_crease(face) {
            None
        } else {
            self.newton_center(p, inside, outside)
        };
        let c = quick.or_else(|| self.searched_center(p, inside, outside))?;
        self.surface_points.insert(key, c);
        Some(c)
    }

    /// One Newton step from an unweighted face's circumcenter along its
    /// normal, the face's dual line, accepted only when it lands on the
    /// surface within the distance tolerance, between the two
    /// orthocenters, and on this face's crossing. Most faces are good, and
    /// this finds their surface point in a few evaluations instead of a
    /// bracketed search.
    fn newton_center(&self, p: [[f64; 3]; 3], inside: Center, outside: Center) -> Option<[f64; 3]> {
        let settings = self.settings;
        let m = circumcenter(p);
        let n = unit(cross(sub(p[1], p[0]), sub(p[2], p[0])))?;
        let sample = self.sampler.gradient(m);
        let slope = dot(sample.gradient, n);
        if slope == 0.0 {
            return None;
        }
        let step = sample.value / slope;
        if step.is_nan() || step.abs() > settings.max_distance {
            return None;
        }
        let c: [f64; 3] = core::array::from_fn(|k| m[k] - step * n[k]);
        let ab = sub(outside.point, inside.point);
        let t = dot(sub(c, inside.point), ab) / dot(ab, ab);
        if !(0.0..=1.0).contains(&t) {
            return None;
        }
        // On the surface, and rising toward the outside orthocenter: the
        // crossing this face's dual edge makes, not a nearby sheet's.
        let at = self.sampler.gradient(c);
        (at.value.abs() <= settings.on_search && dot(at.gradient, ab) > 0.0).then_some(c)
    }

    /// The dual edge clipped to a neighborhood of the face, searched for its
    /// crossing; or, when the clipped edge does not cross, the face's
    /// circumcenter projected onto the surface by Newton steps.
    fn searched_center(
        &self,
        p: [[f64; 3]; 3],
        inside: Center,
        outside: Center,
    ) -> Option<[f64; 3]> {
        let settings = self.settings;
        let sampler = self.sampler;
        let anchor = circumcenter(p);
        let reach = 2.0 * settings.max_size;
        let clip = |c: Center| {
            let d = sub(c.point, anchor);
            let length = exedra_math::norm(d);
            if length <= reach {
                (c.point, c.value)
            } else {
                let q: [f64; 3] = core::array::from_fn(|k| anchor[k] + d[k] * reach / length);
                (q, sampler.value(q))
            }
        };
        let (a, fa) = clip(inside);
        let (b, fb) = clip(outside);
        let c = if fa < 0.0 && fb >= 0.0 {
            let c = sampler.search(a, fa, b, fb, settings.tolerance);
            // The search stops within the tolerance of the crossing, short
            // of it where the edge runs through a kink, as beside a crease.
            // One Newton step along the edge from there lands on it, exactly
            // on a flat piece.
            let sample = sampler.gradient(c);
            let ab = sub(b, a);
            let step = sample.value / dot(sample.gradient, ab);
            let q: [f64; 3] = core::array::from_fn(|k| c[k] - step * ab[k]);
            if step.is_finite()
                && distance(q, c) <= settings.tolerance
                && sampler.value(q).abs() < sample.value.abs()
            {
                q
            } else {
                c
            }
        } else {
            let mut x = anchor;
            for _ in 0..8 {
                let sample = sampler.gradient(x);
                let g2 = dot(sample.gradient, sample.gradient);
                if g2.is_nan() || g2 <= 0.0 {
                    return None;
                }
                let step = sample.value / g2;
                x = core::array::from_fn(|k| x[k] - step * sample.gradient[k]);
            }
            x
        };
        (sampler.value(c).abs() <= settings.on_search && distance(c, anchor) <= reach).then_some(c)
    }

    /// The largest weighted distance from `c` to the face's corners: the
    /// radius of its surface Delaunay ball.
    fn weighted_size(&self, face: [VertexId; 3], c: [f64; 3]) -> f64 {
        face.iter()
            .map(|&v| {
                let d = sub(c, self.tets.point(v));
                exedra_math::Real::sqrt((dot(d, d) - self.tets.weight(v)).max(0.0))
            })
            .fold(0.0, f64::max)
    }

    fn judge(&mut self, r: Restricted) -> Verdict {
        let settings = self.settings;
        let face = self.face(r);
        let Some(c) = self.surface_center(r) else {
            return Verdict::Good;
        };
        let p = face.map(|v| self.tets.point(v));
        let size = self.weighted_size(face, c);
        let shortest = (0..3)
            .map(|e| distance(p[e], p[(e + 1) % 3]))
            .fold(f64::INFINITY, f64::min);
        let on_crease = self.on_crease(face);
        let folded = !on_crease && {
            let normals = face.map(|v| self.normal(v));
            (0..3).any(|e| dot(normals[e], normals[(e + 1) % 3]) < settings.normal_limit)
        };
        // A face on a crease spans a kink, so it is measured by the field at
        // its centroid; elsewhere by how far its surface center strays from
        // its circumcenter.
        let strays = if on_crease {
            let m: [f64; 3] = core::array::from_fn(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0);
            let sample = self.sampler.gradient(m);
            let slope = exedra_math::norm(sample.gradient).max(1.0e-12);
            sample.value.abs() / slope > settings.max_distance
        } else {
            distance(c, circumcenter(p)) > settings.max_distance
        };
        let reason = if face.iter().any(|&v| self.sides[v.index()] != Side::On) {
            Some(Reason::OffSurface)
        } else if size < settings.min_size {
            // Below the resolution floor only the disk condition refines,
            // so degenerate spots such as tangencies cannot run away.
            None
        } else if size > settings.max_size {
            Some(Reason::Size)
        } else if p.iter().map(|&q| distance(c, q)).fold(0.0, f64::max) / shortest
            > settings.max_ratio
        {
            // Shape is judged by plain distance: next to a large ball the
            // weighted size is small whatever the face's shape, and a bad
            // face there shows that the ball should split.
            Some(Reason::Shape)
        } else if strays {
            Some(Reason::Distance)
        } else if folded {
            Some(Reason::Folded)
        } else {
            None
        };
        match reason {
            None => Verdict::Good,
            Some(reason) => match self.guard.covering(c) {
                // A ball that can still split is too large; one at the
                // minimum step already protects its crease, so the face
                // stands.
                Some(ball) if self.creases.splittable(ball, settings) => Verdict::Protected(ball),
                Some(_) => Verdict::Good,
                None => Verdict::Bad(size, c, reason),
            },
        }
    }

    /// Queues face `i` of `t` when it is restricted and bad, or records the
    /// ball refinement would need to enter.
    fn examine(&mut self, work: &mut Work, t: TetId, i: usize) {
        let Some(r) = self.restricted(t, i) else {
            return;
        };
        let (size, judged) = match self.prejudge(r) {
            Some(size) => (size, None),
            None => match self.judge(r) {
                Verdict::Good => return,
                Verdict::Protected(ball) => {
                    work.requests.push(ball);
                    return;
                }
                Verdict::Bad(size, center, reason) => (size, Some((center, reason))),
            },
        };
        let face = self.face(r);
        work.store.push(Candidate {
            tet: r.inside,
            face: r.face,
            generation: self.generation(r.inside),
            judged,
        });
        // Faces on creases first: a protecting ball that is too large shows
        // up before the rest of the surface refines.
        work.queue
            .push((self.on_crease(face), size.to_bits(), work.store.len() - 1));
    }

    /// The face's size about its circumcenter, when that alone shows it too
    /// large or badly shaped, or off the surface; such a face needs no
    /// field evaluation until it is refined.
    fn prejudge(&self, r: Restricted) -> Option<f64> {
        let settings = self.settings;
        let face = self.face(r);
        let p = face.map(|v| self.tets.point(v));
        let m = circumcenter(p);
        let size = self.weighted_size(face, m);
        if face.iter().any(|&v| self.sides[v.index()] != Side::On) {
            return Some(size);
        }
        let reach = p.iter().map(|&q| distance(m, q)).fold(0.0, f64::max);
        let shortest = (0..3)
            .map(|e| distance(p[e], p[(e + 1) % 3]))
            .fold(f64::INFINITY, f64::min);
        (size >= settings.min_size
            && (size > settings.max_size || reach / shortest > settings.max_ratio))
            .then_some(size)
    }

    /// Inserts the surface point `center` of face `face` of `tet`, unless a
    /// vertex lies within half the face's reach of it, and examines the
    /// tetrahedra the insertion made.
    fn insert(
        &mut self,
        work: &mut Work,
        tet: TetId,
        face: usize,
        center: [f64; 3],
        reason: Reason,
    ) -> bool {
        let corners = self.tets.face(tet, face);
        let reach = corners
            .iter()
            .map(|&v| distance(center, self.tets.point(v)))
            .fold(0.0, f64::max);
        // Repairing a disk accepts closer points than refining for quality,
        // so it can always make progress.
        let spacing = if reason == Reason::Disk { 0.2 } else { 0.5 };
        match self
            .tets
            .insert_spaced(center, 0.0, corners[0], spacing * reach)
        {
            Ok(v) if self.added(v, None) => {
                work.refinements.count(reason);
                self.examine_new(work);
                true
            }
            _ => {
                work.refinements.crowded += 1;
                false
            }
        }
    }

    /// Examines the tetrahedra the last insertion made.
    fn examine_new(&mut self, work: &mut Work) {
        work.dirty.push(false);
        let created = self.tets.new_tetrahedra().to_vec();
        self.prefetch_centers(&created);
        for &t in &created {
            for v in self.tets.vertices(t) {
                work.mark(v);
            }
        }
        for t in created {
            for i in 0..4 {
                self.examine(work, t, i);
            }
        }
    }

    /// Inserts the spare seeds near surface vertices that lie on no face,
    /// where a piece of surface fits inside one vertex's cell, and returns
    /// how many it inserted.
    fn reseed(&mut self, work: &mut Work, index: &[u32]) -> usize {
        let reach = 1.5 * self.settings.seed_spacing;
        let isolated: Vec<[f64; 3]> = (0..self.sides.len())
            .filter(|&v| self.sides[v] == Side::On && index[v] == u32::MAX)
            .map(|v| self.tets.points()[v])
            .collect();
        let near: Vec<[f64; 3]> = self
            .spare
            .iter()
            .copied()
            .filter(|&p| isolated.iter().any(|&q| distance(p, q) < reach))
            .collect();
        let mut count = 0;
        for p in near {
            if let Ok(v) = self.tets.insert(p)
                && self.added(v, None)
            {
                self.examine_new(work);
                count += 1;
            }
        }
        self.spare
            .retain(|p| !isolated.iter().any(|&q| distance(*p, q) < reach));
        count
    }

    /// Refines queued faces, largest first, until none is left, a ball
    /// needs splitting, or the point limit is reached.
    fn drain(&mut self, work: &mut Work) -> bool {
        while let Some((_, _, index)) = work.queue.pop() {
            if self.tets.points().len() >= self.settings.max_points {
                return false;
            }
            let candidate = work.store[index];
            if !self.tets.is_live(candidate.tet)
                || self.generation(candidate.tet) != candidate.generation
                || self
                    .restricted(candidate.tet, candidate.face)
                    .is_none_or(|r| r.inside != candidate.tet)
            {
                work.refinements.stale += 1;
                continue;
            }
            if !self.last && !work.requests.is_empty() {
                // The pass will start again once the balls split.
                break;
            }
            let (center, reason) = match candidate.judged {
                Some(judged) => judged,
                None => {
                    let r = Restricted {
                        inside: candidate.tet,
                        face: candidate.face,
                        outside: self.tets.neighbors(candidate.tet)[candidate.face]
                            .expect("a restricted face has two sides"),
                    };
                    match self.judge(r) {
                        Verdict::Good => continue,
                        Verdict::Protected(ball) => {
                            work.requests.push(ball);
                            continue;
                        }
                        Verdict::Bad(_, center, reason) => (center, reason),
                    }
                }
            };
            self.insert(work, candidate.tet, candidate.face, center, reason);
        }
        true
    }

    /// Refines around vertices whose faces do not close into one disk.
    /// Returns how many there were and whether any was refined.
    fn close_disks(&mut self, work: &mut Work) -> (usize, bool) {
        let settings = self.settings;
        let mut checked = core::mem::take(&mut work.dirty_list);
        checked.sort_unstable();
        let mut violations = Vec::new();
        for v in checked {
            work.dirty[v.index()] = false;
            if self.tets.is_hidden(v) || self.sides[v.index()] != Side::On {
                continue;
            }
            if !self.is_disk_at(v) {
                violations.push(v);
            }
        }
        let count = violations.len();
        let mut any = false;
        for v in violations {
            // Refining an earlier violation may have changed this one.
            let umbrella = self.umbrella(v);
            if umbrella.is_empty() || is_disk(v, umbrella.iter().map(|&(face, _)| face)) {
                continue;
            }
            // Checked again once its surroundings change.
            work.mark(v);
            // A protected vertex whose faces do not close has a ball
            // reaching past the surface's local features: split it.
            if let Some(ball) = self.balls[v.index()]
                && self.creases.splittable(ball, settings)
            {
                work.requests.push(ball);
                continue;
            }
            // Otherwise refine its largest face that takes a point.
            let mut options: Vec<(f64, [f64; 3], Restricted)> = umbrella
                .into_iter()
                .filter_map(|(face, r)| {
                    let c = self.surface_center(r)?;
                    Some((self.weighted_size(face, c), c, r))
                })
                .collect();
            options.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut blocked = None;
            let mut refined = false;
            for (_, center, r) in options {
                if let Some(ball) = self.guard.covering(center) {
                    blocked.get_or_insert(ball);
                    continue;
                }
                if self.insert(work, r.inside, r.face, center, Reason::Disk) {
                    refined = true;
                    break;
                }
            }
            any |= refined;
            if !refined
                && let Some(ball) = blocked
                && self.creases.splittable(ball, settings)
            {
                work.requests.push(ball);
            }
        }
        (count, any)
    }

    fn run(mut self) -> Pass {
        let settings = self.settings;
        let mut work = Work {
            queue: BinaryHeap::new(),
            store: Vec::new(),
            requests: Vec::new(),
            refinements: Refinements::default(),
            dirty: alloc::vec![false; self.sides.len()],
            dirty_list: Vec::new(),
        };
        let all: Vec<TetId> = self.tets.tetrahedra().collect();
        self.prefetch_centers(&all);
        for &t in &all {
            for v in self.tets.vertices(t) {
                work.mark(v);
            }
            for i in 0..4 {
                self.examine(&mut work, t, i);
            }
        }
        let mut hit_point_limit = false;
        let mut disk_failures = 0;
        let (positions, triangles, index) = loop {
            for _ in 0..=settings.max_disk_rounds {
                if !self.drain(&mut work) {
                    hit_point_limit = true;
                    break;
                }
                if (!self.last && !work.requests.is_empty()) || work.dirty_list.is_empty() {
                    break;
                }
                let (failures, refined) = self.close_disks(&mut work);
                disk_failures = failures;
                if failures == 0 || !refined || (!self.last && !work.requests.is_empty()) {
                    break;
                }
            }
            let surface = self.surface();
            if hit_point_limit || self.reseed(&mut work, &surface.2) == 0 {
                break surface;
            }
        };
        // A surface vertex on no face stands for surface the mesh lost.
        let isolated_vertices = (0..self.sides.len())
            .filter(|&v| self.sides[v] == Side::On && index[v] == u32::MAX)
            .filter(|&v| {
                // Crease samples and points inside their balls may be
                // hidden by a neighboring ball.
                self.balls[v].is_none() && self.guard.covering(self.tets.points()[v]).is_none()
            })
            .count();
        let (creases, cut) = self.crease_edges(&triangles, &index);
        // A crease the surface cuts across has balls too large to hold it,
        // unless they are already at the minimum step.
        let splittable = |ball: &BallId| self.creases.splittable(*ball, self.settings);
        let (broken, unresolved): (Vec<_>, Vec<_>) = cut
            .into_iter()
            .partition(|(a, b)| splittable(a) || splittable(b));
        let mut requests = work.requests;
        // Splitting helps where the crease meets a surface; where none
        // formed at all, smaller balls gain nothing.
        if !self.last && !triangles.is_empty() {
            requests.extend(broken.iter().flat_map(|&(a, b)| [a, b]).filter(splittable));
        }
        requests.sort_unstable();
        requests.dedup();
        Pass {
            positions,
            triangles,
            creases,
            broken_creases: broken.len(),
            unresolved_creases: unresolved.len(),
            isolated_vertices,
            requests,
            refinements: work.refinements,
            disk_failures,
            hit_point_limit,
            tetrahedra: self.tets.tetrahedron_count(),
        }
    }

    fn generation(&self, t: TetId) -> u32 {
        self.generations.get(t.index()).copied().unwrap_or(0)
    }

    /// Whether `v`'s restricted faces, if any, close into one disk.
    fn is_disk_at(&mut self, v: VertexId) -> bool {
        let umbrella = self.umbrella(v);
        umbrella.is_empty() || is_disk(v, umbrella.iter().map(|&(face, _)| face))
    }

    /// The restricted faces around `v`, from their inside tetrahedra.
    fn umbrella(&mut self, v: VertexId) -> Vec<([VertexId; 3], Restricted)> {
        let mut faces = Vec::new();
        let star = self.star(v);
        for &t in &star {
            for i in 0..4 {
                if self.tets.vertices(t)[i] == v {
                    continue;
                }
                if let Some(r) = self.restricted(t, i)
                    && r.inside == t
                {
                    faces.push((self.face(r), r));
                }
            }
        }
        self.scratch = star;
        faces
    }

    /// The tetrahedra around `v`, found across the faces through it, each
    /// visit marked in its slot so the walk stays linear.
    fn star(&mut self, v: VertexId) -> Vec<TetId> {
        let mut star = core::mem::take(&mut self.scratch);
        star.clear();
        let Some(first) = self.tets.tetrahedron_at(v) else {
            return star;
        };
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.visits.fill(0);
            self.epoch = 1;
        }
        let slots = self.tets.slot_count();
        if self.visits.len() < slots {
            self.visits.resize(slots, 0);
        }
        self.visits[first.index()] = self.epoch;
        star.push(first);
        let mut next = 0;
        while let Some(&t) = star.get(next) {
            next += 1;
            let (vertices, neighbors) = (self.tets.vertices(t), self.tets.neighbors(t));
            for i in 0..4 {
                // The faces through `v` are those opposite the other corners.
                if vertices[i] == v {
                    continue;
                }
                if let Some(n) = neighbors[i]
                    && self.visits[n.index()] != self.epoch
                {
                    self.visits[n.index()] = self.epoch;
                    star.push(n);
                }
            }
        }
        star
    }

    /// The restricted faces as indexed triangles over the vertices they use,
    /// wound outward, and each vertex's index among them, if it has one.
    fn surface(&mut self) -> (Vec<[f64; 3]>, Vec<[u32; 3]>, Vec<u32>) {
        let mut index = alloc::vec![u32::MAX; self.sides.len()];
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        let all: Vec<TetId> = self.tets.tetrahedra().collect();
        for t in all {
            for i in 0..4 {
                let Some(r) = self.restricted(t, i) else {
                    continue;
                };
                if r.inside != t {
                    continue;
                }
                let face = self.face(r).map(|v| {
                    let slot = &mut index[v.index()];
                    if *slot == u32::MAX {
                        *slot = narrow(positions.len());
                        positions.push(self.tets.point(v));
                    }
                    *slot
                });
                triangles.push(face);
            }
        }
        (positions, triangles, index)
    }

    /// The crease segments that are edges of `triangles`, and those that
    /// are not: a segment missing, or with an end off the surface, leaves
    /// the crease unprotected there.
    fn crease_edges(
        &self,
        triangles: &[[u32; 3]],
        index: &[u32],
    ) -> (Vec<[u32; 2]>, Vec<(BallId, BallId)>) {
        let mut at: HashMap<BallId, u32> = HashMap::new();
        for (v, ball) in self.balls.iter().enumerate() {
            if let Some(ball) = ball
                && index[v] != u32::MAX
            {
                at.insert(*ball, index[v]);
            }
        }
        let mut edges: HashSet<[u32; 2]> = HashSet::new();
        for t in triangles {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                edges.insert([a.min(b), a.max(b)]);
            }
        }
        let mut creases = Vec::new();
        let mut broken = Vec::new();
        for (a, b) in self.creases.segments() {
            match (at.get(&a), at.get(&b)) {
                (Some(&i), Some(&j)) if edges.contains(&[i.min(j), i.max(j)]) => {
                    creases.push([i, j]);
                }
                _ => broken.push((a, b)),
            }
        }
        (creases, broken)
    }
}

/// The refinement queue and what a pass has found so far.
struct Work {
    queue: BinaryHeap<(bool, u64, usize)>,
    store: Vec<Candidate>,
    requests: Vec<BallId>,
    refinements: Refinements,
    /// Per vertex: whether its faces changed since its disk was checked.
    dirty: Vec<bool>,
    dirty_list: Vec<VertexId>,
}

impl Work {
    fn mark(&mut self, v: VertexId) {
        if let Some(flag) = self.dirty.get_mut(v.index())
            && !*flag
        {
            *flag = true;
            self.dirty_list.push(v);
        }
    }
}

/// Whether faces around `v` form one closed disk: their edges opposite `v`
/// chain into a single cycle.
fn is_disk(v: VertexId, faces: impl Iterator<Item = [VertexId; 3]>) -> bool {
    let mut edges: Vec<(VertexId, VertexId)> = Vec::new();
    for face in faces {
        let Some(at) = face.iter().position(|&w| w == v) else {
            return false;
        };
        let (a, b) = (face[(at + 1) % 3], face[(at + 2) % 3]);
        if edges.iter().any(|&(x, _)| x == a) {
            return false;
        }
        edges.push((a, b));
    }
    let Some(&(start, _)) = edges.first() else {
        return false;
    };
    let mut at = start;
    for step in 0..edges.len() {
        let Some(&(_, next)) = edges.iter().find(|&&(x, _)| x == at) else {
            return false;
        };
        at = next;
        if at == start {
            return step + 1 == edges.len();
        }
    }
    false
}

impl Refinements {
    fn count(&mut self, reason: Reason) {
        match reason {
            Reason::OffSurface => self.off_surface += 1,
            Reason::Size => self.size += 1,
            Reason::Shape => self.shape += 1,
            Reason::Distance => self.distance += 1,
            Reason::Folded => self.folded += 1,
            Reason::Disk => self.disk += 1,
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "vertex counts fit u32 indices"
)]
fn narrow(index: usize) -> u32 {
    index as u32
}
