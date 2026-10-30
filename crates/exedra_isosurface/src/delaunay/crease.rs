// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Creases: traced at adaptive spacing and protected by balls.
//!
//! Creases are traced from seeds on the grid, then from around each corner
//! found, so every crease between two corners is found, even an edge too
//! short for the grid to seed. Tracing stops only at samples of the same
//! crease, told apart by their ties and by the crease running on between
//! them, so the creases on the two sides of a fin or along a rod thinner
//! than a ball are traced apart.
//!
//! Following Cheng, Dey and Levine's `DelPSC`, every crease sample becomes a
//! weighted vertex whose weight is its protecting ball's squared radius.
//! Neighboring balls along a crease overlap without containing each other's
//! centers, corners get balls of their own before the creases meeting
//! there, and no ball reaches across to another crease. Refinement then
//! never needs a point inside a ball unless the ball is too large, in which
//! case that stretch of crease is split.

use alloc::vec::Vec;

use exedra_math::{cross, dot, sub};
use hashbrown::HashMap;

use super::Settings;
use super::sample::{Sampler, distance, following, midpoint, norm, unit};

/// A crease sample's protecting ball: which line it belongs to and where
/// along it. Corners have no line.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct BallId {
    pub(super) line: u32,
    pub(super) index: u32,
}

impl BallId {
    const CORNER: u32 = u32::MAX;

    pub(super) fn corner(index: usize) -> Self {
        Self {
            line: Self::CORNER,
            index: narrow(index),
        }
    }
}

/// Traced creases: polylines of samples on them, and the corners where they
/// meet or end.
#[derive(Clone, Debug, Default)]
pub(super) struct Creases {
    pub(super) lines: Vec<Vec<[f64; 3]>>,
    /// Whether each line is a closed loop, its last sample followed by its
    /// first.
    closed: Vec<bool>,
    pub(super) corners: Vec<Corner>,
    /// Whether tracing the creases leaving corners stopped at the sample
    /// budget, so some may be missing.
    pub(super) hit_sample_limit: bool,
}

/// A corner and the line ends it protects.
#[derive(Clone, Debug)]
pub(super) struct Corner {
    point: [f64; 3],
    /// Lines ending here, each with whether at its start.
    ends: Vec<(usize, bool)>,
}

impl Corner {
    /// The sample nearest the corner on each line ending here.
    fn neighbors<'a>(&'a self, lines: &'a [Vec<[f64; 3]>]) -> impl Iterator<Item = [f64; 3]> + 'a {
        self.ends.iter().map(|&(l, start)| {
            let line = &lines[l];
            if start { line[0] } else { line[line.len() - 1] }
        })
    }

    /// The ball reaching partway into each neighboring sample's ball without
    /// containing its center.
    fn radius(&self, lines: &[Vec<[f64; 3]>], settings: &Settings) -> f64 {
        let nearest = self
            .neighbors(lines)
            .map(|q| distance(q, self.point))
            .fold(f64::INFINITY, f64::min);
        let radius = if nearest.is_finite() {
            settings.protect * nearest
        } else {
            settings.min_step
        };
        radius.max(0.5 * settings.min_step)
    }
}

/// How a traced half of a crease ended.
enum End {
    /// At a corner where another crease meets it.
    Corner([f64; 3]),
    /// Anywhere else: where the crease fades, turns back, or meets samples
    /// already taken.
    Open,
}

impl Creases {
    /// Traces a crease through each seed not already covered, then each
    /// crease leaving a corner that the seeds missed, protects the corners,
    /// and spaces the samples.
    pub(super) fn trace(sampler: &Sampler<'_>, seeds: &[[f64; 3]], settings: &Settings) -> Self {
        let mut creases = Self::default();
        let mut found: Vec<[f64; 3]> = Vec::new();
        let mut traced = Traced::new(settings);
        // The stretches tracing left between a line and the corner it
        // stopped at; a seed on one would start a stray line.
        let mut gaps: Vec<([f64; 3], [f64; 3])> = Vec::new();
        // For each line's start and end, the corner tracing stopped at.
        let mut stops: Vec<[Option<usize>; 2]> = Vec::new();
        for &seed in seeds {
            if traced.holds(sampler, seed, 1.0, tie_normal(sampler, seed))
                || gaps
                    .iter()
                    .any(|&(a, b)| segment_distance(seed, a, b) < 0.5 * settings.min_step)
            {
                continue;
            }
            let (line, ends) = trace(sampler, seed, settings, &traced);
            let mut stop = [None; 2];
            for (k, (end, last)) in ends
                .into_iter()
                .zip([line[0], line[line.len() - 1]])
                .enumerate()
            {
                if let End::Corner(c) = end {
                    let index = found
                        .iter()
                        .position(|&f| distance(f, c) <= settings.min_step)
                        .unwrap_or_else(|| {
                            found.push(c);
                            found.len() - 1
                        });
                    stop[k] = Some(index);
                    gaps.push((last, c));
                }
            }
            stops.push(stop);
            traced.add(sampler, &line);
            creases.lines.push(line);
        }
        // A seed on a corner traces no crease of its own.
        let at_corner = |line: &[[f64; 3]]| {
            line.len() == 1
                && found
                    .iter()
                    .any(|&c| distance(c, line[0]) <= settings.min_step)
        };
        (creases.lines, stops) = core::mem::take(&mut creases.lines)
            .into_iter()
            .zip(stops)
            .filter(|(line, _)| !at_corner(line))
            .unzip();
        creases.join_continuations(sampler, &mut stops, &found, settings);
        creases.protect_corners(sampler, &found, &stops, settings);
        creases.space(sampler, settings);
        creases
    }

    /// Every protecting ball: center, radius and identity.
    pub(super) fn balls(
        &self,
        settings: &Settings,
    ) -> impl Iterator<Item = ([f64; 3], f64, BallId)> + '_ {
        let settings = *settings;
        let lines = &self.lines;
        let corners =
            self.corners.iter().enumerate().map(move |(index, c)| {
                (c.point, c.radius(lines, &settings), BallId::corner(index))
            });
        let closed = &self.closed;
        let samples = self.lines.iter().enumerate().flat_map(move |(l, line)| {
            (0..line.len()).map(move |i| {
                (
                    line[i],
                    ball_radius(line, closed[l], i, &settings),
                    BallId {
                        line: narrow(l),
                        index: narrow(i),
                    },
                )
            })
        });
        corners.chain(samples)
    }

    /// Every pair of protecting balls a crease runs between: neighboring
    /// samples along each line, around each loop, and from each corner to
    /// the end of each line meeting there.
    pub(super) fn segments(&self) -> Vec<(BallId, BallId)> {
        let mut segments = Vec::new();
        for (l, line) in self.lines.iter().enumerate() {
            let n = line.len();
            let ball = |i: usize| BallId {
                line: narrow(l),
                index: narrow(i % n),
            };
            segments.extend((0..segments_of(n, self.closed[l])).map(|i| (ball(i), ball(i + 1))));
        }
        for (c, corner) in self.corners.iter().enumerate() {
            for &(l, start) in &corner.ends {
                let index = if start { 0 } else { self.lines[l].len() - 1 };
                let end = BallId {
                    line: narrow(l),
                    index: narrow(index),
                };
                segments.push((BallId::corner(c), end));
            }
        }
        segments
    }

    /// The number of crease samples, corners included.
    pub(super) fn samples(&self) -> usize {
        self.corners.len() + self.lines.iter().map(Vec::len).sum::<usize>()
    }

    /// Whether the crease segments beside `ball` are long enough to split;
    /// for a corner, the gaps to its neighboring samples.
    pub(super) fn splittable(&self, ball: BallId, settings: &Settings) -> bool {
        if ball.line == BallId::CORNER {
            return self.corners.get(ball.index as usize).is_some_and(|c| {
                c.neighbors(&self.lines)
                    .any(|q| distance(q, c.point) > 2.0 * settings.min_step)
            });
        }
        let Some(line) = self.lines.get(ball.line as usize) else {
            return false;
        };
        let [before, after] =
            steps_beside(line, self.closed[ball.line as usize], ball.index as usize);
        before
            .into_iter()
            .chain(after)
            .any(|step| step > 2.0 * settings.min_step)
    }

    /// Halves the crease segments on both sides of each requested ball, down
    /// to the minimum step, and spaces the samples again, unless the samples
    /// already fill their budget. Returns the segments split.
    pub(super) fn split(
        &mut self,
        sampler: &Sampler<'_>,
        requests: &[BallId],
        settings: &Settings,
    ) -> usize {
        // A budget on all samples, so splitting cannot run away.
        if self.samples() >= settings.max_crease_samples {
            return 0;
        }
        let mut count = 0;
        for (l, line) in self.lines.iter_mut().enumerate() {
            let closed = self.closed[l];
            let mut cut = alloc::vec![false; line.len()];
            for ball in requests.iter().filter(|b| b.line as usize == l) {
                let i = ball.index as usize;
                if let Some(before) = segment_before(line.len(), closed, i) {
                    cut[before] = true;
                }
                cut[i] = true;
            }
            count += subdivide(sampler, line, closed, &cut, settings);
        }
        // A corner shrinks by taking a sample halfway to each neighbor.
        for ball in requests.iter().filter(|b| b.line == BallId::CORNER) {
            let Some(corner) = self.corners.get(ball.index as usize) else {
                continue;
            };
            for &(l, start) in &corner.ends {
                let line = &mut self.lines[l];
                let end = if start { line[0] } else { line[line.len() - 1] };
                if distance(end, corner.point) <= 2.0 * settings.min_step {
                    continue;
                }
                let along = tie_normal(sampler, end);
                if let Some(m) =
                    sampler.crease_point_along(midpoint(end, corner.point), settings.cell, along)
                {
                    if start {
                        line.insert(0, m);
                    } else {
                        line.push(m);
                    }
                    count += 1;
                }
            }
        }
        self.space(sampler, settings);
        count
    }

    /// Joins lines that one crease was traced into from several seeds: open
    /// ends close together that point straight at each other, with no
    /// corner or third end nearby, continue the same crease.
    fn join_continuations(
        &mut self,
        sampler: &Sampler<'_>,
        stops: &mut Vec<[Option<usize>; 2]>,
        corners: &[[f64; 3]],
        settings: &Settings,
    ) {
        // An end's position, the direction the line leaves through it, and
        // its last step.
        let end = |line: &[[f64; 3]], start: bool| -> Option<([f64; 3], [f64; 3], f64)> {
            if line.len() < 2 {
                return None;
            }
            let (p, q) = if start {
                (line[0], line[1])
            } else {
                (line[line.len() - 1], line[line.len() - 2])
            };
            Some((p, unit(sub(p, q))?, distance(p, q).max(settings.min_step)))
        };
        'join: loop {
            for a in 0..self.lines.len() {
                for b in a + 1..self.lines.len() {
                    for (a_start, b_start) in
                        [(false, true), (false, false), (true, true), (true, false)]
                    {
                        let (Some(ea), Some(eb)) =
                            (end(&self.lines[a], a_start), end(&self.lines[b], b_start))
                        else {
                            continue;
                        };
                        let gap = distance(ea.0, eb.0);
                        let reach = 2.5 * ea.2.max(eb.2);
                        let across = sub(eb.0, ea.0);
                        let facing = dot(ea.1, eb.1) < -0.9
                            && (gap == 0.0 || (dot(across, ea.1) > 0.0 && dot(across, eb.1) < 0.0));
                        if gap >= reach || !facing {
                            continue;
                        }
                        // The same crease: its tie's normal carries on.
                        if !same_tie(tie_normal(sampler, ea.0), tie_normal(sampler, eb.0)) {
                            continue;
                        }
                        let near =
                            |p: [f64; 3]| distance(p, ea.0) < reach || distance(p, eb.0) < reach;
                        // The two lines' other ends are no third crease: a
                        // short line traced into a loop's gap has both its
                        // ends there.
                        let crowded = corners.iter().any(|&c| near(c))
                            || (0..self.lines.len())
                                .filter(|&l| l != a && l != b)
                                .any(|l| {
                                    [true, false].into_iter().any(|start| {
                                        end(&self.lines[l], start).is_some_and(|(p, ..)| near(p))
                                    })
                                });
                        if crowded {
                            continue;
                        }
                        let mut second = self.lines.swap_remove(b);
                        let [mut first_stops, mut second_stops] = [stops[a], stops.swap_remove(b)];
                        let first = &mut self.lines[a];
                        if a_start {
                            first.reverse();
                            first_stops.reverse();
                        }
                        if !b_start {
                            second.reverse();
                            second_stops.reverse();
                        }
                        first.extend(second);
                        stops[a] = [first_stops[0], second_stops[1]];
                        continue 'join;
                    }
                }
            }
            break;
        }
    }

    /// Protects the corners before the lines meeting there.
    ///
    /// Corners are those tracing stopped at, plus one for each group of
    /// open ends close together, as where a crease fades out or tracing
    /// lost it. Each line loses its samples near a corner it ends at, and
    /// the corner's ball reaches partway into each such line's nearest
    /// remaining ball without containing its center.
    fn protect_corners(
        &mut self,
        sampler: &Sampler<'_>,
        found: &[[f64; 3]],
        stops: &[[Option<usize>; 2]],
        settings: &Settings,
    ) {
        let step_at = |line: &[[f64; 3]], start: bool| end_step(line, start, settings);
        // Lines whose ends meet, each heading on towards the other, close
        // into loops. A closing step much shorter than its neighbors loses
        // its last sample, so the steps stay graded around the loop.
        let mut closed: Vec<bool> = self
            .lines
            .iter_mut()
            .map(|line| {
                let gap = |line: &[[f64; 3]]| distance(line[0], line[line.len() - 1]);
                let n = line.len();
                let closed = n > 2
                    && gap(line) < 1.5 * step_at(line, true).min(step_at(line, false))
                    && dot(sub(line[0], line[n - 1]), sub(line[n - 1], line[n - 2])) > 0.0
                    && dot(sub(line[n - 1], line[0]), sub(line[0], line[1])) > 0.0;
                while closed && line.len() > 3 && gap(line) < 0.5 * step_at(line, false) {
                    line.pop();
                }
                closed
            })
            .collect();
        let lines = &self.lines;
        // Ends of lines that are not closed loops: line, whether at its
        // start, point, and last step.
        let mut ends = Vec::new();
        // The corner each end's tracing stopped at, if any.
        let mut stopped = Vec::new();
        for (l, line) in lines.iter().enumerate() {
            if !closed[l] {
                ends.push((l, true, line[0], step_at(line, true)));
                ends.push((l, false, line[line.len() - 1], step_at(line, false)));
                stopped.extend(stops[l]);
            }
        }
        // Each end belongs to the nearest corner its crease runs on to, among
        // those within reach and the one its tracing stopped at, which may
        // lie past a nearer one; the rest group around a first end, without
        // chaining.
        let mut corners: Vec<[f64; 3]> = found.to_vec();
        let mut groups: Vec<Vec<usize>> = alloc::vec![Vec::new(); corners.len()];
        let mut loose: Vec<(usize, Vec<usize>)> = Vec::new();
        let normals: Vec<Option<[f64; 3]>> = ends
            .iter()
            .map(|&(_, _, p, _)| tie_normal(sampler, p))
            .collect();
        // The direction each end's line leaves through it; for a single
        // sample, the crease's tangent there, its start facing back.
        let heading = |e: usize| {
            let (l, start, p, _) = ends[e];
            let line = &lines[l];
            if line.len() < 2 {
                let tangent = frame(sampler, p, normals[e])?[2];
                return Some(if start { tangent.map(|v| -v) } else { tangent });
            }
            unit(sub(p, if start { line[1] } else { line[line.len() - 2] }))
        };
        // A corner an end meets lies ahead of it, on its crease: how far
        // the way there strays from the crease, if it meets it.
        let meets = |c: [f64; 3], e: usize| {
            let p = ends[e].2;
            if heading(e).is_some_and(|d| dot(sub(c, p), d) <= 0.0) {
                return None;
            }
            normals[e].map_or(Some(0.0), |n| strays(sampler, p, c, n, settings))
        };
        for (e, &(_, _, p, step)) in ends.iter().enumerate() {
            let reach = 2.5 * step.max(settings.max_step);
            // The nearest of those the crease runs most nearly straight to,
            // so an end beside a thin fin keeps to its own side.
            let candidates: Vec<(usize, f64)> = (0..corners.len())
                .filter(|&c| stopped[e] == Some(c) || distance(corners[c], p) < reach)
                .filter_map(|c| Some((c, meets(corners[c], e)?)))
                .collect();
            let straightest = candidates
                .iter()
                .map(|&(_, stray)| stray)
                .fold(f64::INFINITY, f64::min);
            let nearest = candidates
                .iter()
                .filter(|&&(_, stray)| stray <= straightest + 2.5 * settings.max_distance)
                .map(|&(c, _)| c)
                .min_by(|&a, &b| distance(corners[a], p).total_cmp(&distance(corners[b], p)));
            if let Some(c) = nearest {
                groups[c].push(e);
            } else if let Some(group) = loose.iter_mut().find(|(seed, _)| {
                // Each heads towards the other, by more than ends side by
                // side, as where parallel creases run out of the bounds.
                let q = ends[*seed].2;
                let toward = |from: usize, to: [f64; 3]| {
                    let across = unit(sub(to, ends[from].2));
                    match (heading(from), across) {
                        (Some(d), Some(across)) => dot(across, d) > 0.1,
                        _ => true,
                    }
                };
                distance(q, p) < 1.5 * ends[*seed].3.max(step) && toward(e, q) && toward(*seed, p)
            }) {
                group.1.push(e);
            } else {
                loose.push((e, alloc::vec![e]));
            }
        }
        for (_, members) in loose {
            let corner = corner_position(sampler, lines, &ends, &members, settings);
            // Ends that stopped short of a corner tracing found elsewhere.
            let known = (0..found.len()).find(|&c| {
                distance(corners[c], corner) <= settings.min_step
                    && members.iter().all(|&e| meets(corners[c], e).is_some())
            });
            match known {
                Some(c) => groups[c].extend(members),
                None => {
                    corners.push(corner);
                    groups.push(members);
                }
            }
        }
        self.complete_corners(
            sampler,
            &mut corners,
            &mut groups,
            &mut ends,
            &mut closed,
            settings,
        );

        let lines = &self.lines;
        let mut trim = alloc::vec![(0_usize, 0_usize); lines.len()];
        let mut protected = Vec::with_capacity(corners.len());
        for (corner, members) in corners.into_iter().zip(groups) {
            // Trim samples close to the corner.
            for &e in &members {
                let (l, start, _, step) = ends[e];
                let line = &lines[l];
                let (head, tail) = trim[l];
                let mut cut = 0;
                while head + tail + cut < line.len() {
                    let index = if start {
                        head + cut
                    } else {
                        line.len() - 1 - tail - cut
                    };
                    if distance(line[index], corner) >= 0.75 * step {
                        break;
                    }
                    cut += 1;
                }
                if start {
                    trim[l].0 += cut;
                } else {
                    trim[l].1 += cut;
                }
            }
            protected.push(Corner {
                point: corner,
                ends: members.iter().map(|&e| (ends[e].0, ends[e].1)).collect(),
            });
        }
        // A line trimmed away entirely between two corners is a short edge
        // between them, and keeps the crease point midway; spacing fills it
        // out. Other lines trimmed away are dropped, and the rest renumbered.
        let mut attached = alloc::vec![[None; 2]; self.lines.len()];
        for (c, corner) in protected.iter().enumerate() {
            for &(l, start) in &corner.ends {
                attached[l][usize::from(!start)] = Some(c);
            }
        }
        let mut renumber = alloc::vec![None; self.lines.len()];
        let mut kept = Vec::with_capacity(self.lines.len());
        self.closed.clear();
        for (l, (mut line, (head, tail))) in core::mem::take(&mut self.lines)
            .into_iter()
            .zip(trim)
            .enumerate()
        {
            if head + tail < line.len() {
                line.truncate(line.len() - tail);
                line.drain(..head);
            } else if let [Some(a), Some(b)] = attached[l]
                && a != b
                && let Some(middle) = short_edge(
                    sampler,
                    protected[a].point,
                    protected[b].point,
                    &line,
                    settings,
                )
            {
                line = alloc::vec![middle];
            } else {
                continue;
            }
            renumber[l] = Some(kept.len());
            kept.push(line);
            self.closed.push(closed[l]);
        }
        self.lines = kept;
        for corner in &mut protected {
            corner.ends = corner
                .ends
                .iter()
                .filter_map(|&(l, start)| Some((renumber[l]?, start)))
                .collect();
        }
        self.corners = protected;
    }

    /// Traces the creases leaving each corner that no line attached there
    /// follows yet.
    ///
    /// Probes around the corner find a point on each crease leaving it.
    /// One leading away in neither the direction nor the tie of any line
    /// attached there starts an arc, kept when it runs to another corner
    /// and follows no line already traced: an edge too short for the grid
    /// to seed, as across a thin plate. Corners such arcs find have their
    /// creases traced in turn.
    fn complete_corners(
        &mut self,
        sampler: &Sampler<'_>,
        corners: &mut Vec<[f64; 3]>,
        groups: &mut Vec<Vec<usize>>,
        ends: &mut Vec<(usize, bool, [f64; 3], f64)>,
        closed: &mut Vec<bool>,
        settings: &Settings,
    ) {
        // A budget on all samples, so discovery cannot run away where
        // corners keep finding more.
        let mut samples = corners.len() + self.lines.iter().map(Vec::len).sum::<usize>();
        let mut c = 0;
        while c < corners.len() {
            if samples >= settings.max_crease_samples {
                self.hit_sample_limit = true;
                return;
            }
            let corner = corners[c];
            for port in ports(sampler, corner, corners, settings) {
                let normal = tie_normal(sampler, port);
                let leaving = unit(sub(port, corner));
                // An end on the corner itself follows no crease from it.
                let followed = groups[c].iter().any(|&e| {
                    let along = unit(sub(ends[e].2, corner));
                    let aligned = match (leaving, along) {
                        (Some(a), Some(b)) => dot(a, b) > 0.8,
                        _ => false,
                    };
                    aligned && same_tie(normal, tie_normal(sampler, ends[e].2))
                });
                if followed {
                    continue;
                }
                let Some(Arc {
                    samples: line,
                    from: start,
                    to: other,
                }) = arc(sampler, corner, port, settings)
                else {
                    continue;
                };
                // Already traced: a line with the arc's tie runs between
                // its corners, or along it, or between a line's end and its
                // corner. One nearby point of another tie is no evidence:
                // creases leave a corner close together, or pass close by.
                let normals: Vec<Option<[f64; 3]>> =
                    line.iter().map(|&p| tie_normal(sampler, p)).collect();
                let middle = normals[line.len() / 2];
                let known = [start, other].map(|x| {
                    corners
                        .iter()
                        .position(|&k| distance(k, x) <= settings.min_step)
                });
                let tolerance = 0.25 * settings.min_step + 2.5 * settings.max_distance;
                let near =
                    |p: [f64; 3], a: [f64; 3], b: [f64; 3]| segment_distance(p, a, b) < tolerance;
                // Shared corners and ties only make a line a candidate; it
                // must also pass through the arc's middle, as the two rims
                // of a lens do not.
                let joined = match known {
                    [Some(a), Some(b)] => {
                        let through = line[line.len() / 2];
                        groups[a].iter().any(|&e| {
                            let l = ends[e].0;
                            let existing = &self.lines[l];
                            same_tie(middle, tie_normal(sampler, existing[existing.len() / 2]))
                                && groups[b].iter().any(|&f| {
                                    ends[f].0 == l
                                        && f != e
                                        && (near(through, ends[e].2, corners[a])
                                            || near(through, ends[f].2, corners[b])
                                            || existing
                                                .windows(2)
                                                .any(|s| near(through, s[0], s[1])))
                                })
                        })
                    }
                    _ => false,
                };
                let on_lines = |p: [f64; 3], normal: Option<[f64; 3]>| {
                    let tied = |q: [f64; 3]| same_tie(normal, tie_normal(sampler, q));
                    self.lines.iter().any(|existing| {
                        existing
                            .windows(2)
                            .any(|s| near(p, s[0], s[1]) && tied(s[0]))
                    }) || groups.iter().enumerate().any(|(k, members)| {
                        members
                            .iter()
                            .any(|&e| near(p, ends[e].2, corners[k]) && tied(ends[e].2))
                    })
                };
                let path: Vec<[f64; 3]> = [start]
                    .into_iter()
                    .chain(line.iter().copied())
                    .chain([other])
                    .collect();
                // Path segment `k` ends at sample `k`, or at `other` after
                // the last.
                let on_arc = |q: [f64; 3]| {
                    path.windows(2).enumerate().any(|(k, s)| {
                        near(q, s[0], s[1])
                            && same_tie(normals[k.min(line.len() - 1)], tie_normal(sampler, q))
                    })
                };
                // Every crease meets the others at a corner, so only points
                // away from the arc's corners count.
                let away = |p: &[f64; 3]| {
                    distance(*p, start).min(distance(*p, other)) >= settings.min_step
                };
                if joined
                    || line
                        .iter()
                        .zip(&normals)
                        .filter(|(p, _)| away(p))
                        .any(|(&p, &normal)| on_lines(p, normal))
                    || self
                        .lines
                        .iter()
                        .flatten()
                        .filter(|q| away(q))
                        .any(|&q| on_arc(q))
                {
                    continue;
                }
                let mut corner_at = |x: [f64; 3], known: Option<usize>| {
                    known.unwrap_or_else(|| {
                        corners.push(x);
                        groups.push(Vec::new());
                        corners.len() - 1
                    })
                };
                let near = corner_at(start, known[0]);
                let far = corner_at(other, known[1]);
                if near == far {
                    continue;
                }
                samples +=
                    line.len() + usize::from(known[0].is_none()) + usize::from(known[1].is_none());
                let l = self.lines.len();
                for (start, at) in [(true, near), (false, far)] {
                    let point = if start { line[0] } else { line[line.len() - 1] };
                    ends.push((l, start, point, end_step(&line, start, settings)));
                    groups[at].push(ends.len() - 1);
                }
                self.lines.push(line);
                closed.push(false);
            }
            c += 1;
        }
    }

    /// Spaces each line's samples: no step longer than allows a ball to
    /// reach another crease, and neighboring steps differing by at most
    /// half again, so ball radii vary slowly and no ball swallows its
    /// neighbor's center.
    fn space(&mut self, sampler: &Sampler<'_>, settings: &Settings) {
        for _ in 0..32 {
            let gaps = self.feature_gaps(settings);
            let mut changed = 0;
            for (l, line) in self.lines.iter_mut().enumerate() {
                let closed = self.closed[l];
                let n = line.len();
                // Segment `i` runs from sample `i` to the next.
                let steps: Vec<f64> = (0..segments_of(n, closed))
                    .map(|i| distance(line[i], line[(i + 1) % n]))
                    .collect();
                let mut long = alloc::vec![false; n];
                for i in 0..steps.len() {
                    // A ball is `protect` times its longer step; keep it
                    // within half the gap to another crease.
                    let room = 0.5 * gaps[l][i].min(gaps[l][(i + 1) % n]) / settings.protect;
                    long[i] |= steps[i] > room;
                    let Some(before) = segment_before(n, closed, i) else {
                        continue;
                    };
                    if steps[i] > 1.5 * steps[before] {
                        long[i] = true;
                    } else if steps[before] > 1.5 * steps[i] {
                        long[before] = true;
                    }
                }
                changed += subdivide(sampler, line, closed, &long, settings);
            }
            // A corner's gap to each line ending there spaced like the
            // line's own first step.
            for corner in &self.corners {
                for &(l, start) in &corner.ends {
                    let line = &mut self.lines[l];
                    let (end, next) = if start {
                        (line[0], line.get(1).copied())
                    } else {
                        (
                            line[line.len() - 1],
                            line.len().checked_sub(2).map(|i| line[i]),
                        )
                    };
                    let gap = distance(end, corner.point);
                    let step = next.map_or(settings.max_step, |n| distance(end, n));
                    if gap <= 1.5 * step.max(settings.min_step) || gap <= 2.0 * settings.min_step {
                        continue;
                    }
                    if let Some(m) = sampler.crease_point_along(
                        midpoint(end, corner.point),
                        settings.cell,
                        tie_normal(sampler, end),
                    ) {
                        if start {
                            line.insert(0, m);
                        } else {
                            line.push(m);
                        }
                        changed += 1;
                    }
                }
            }
            if changed == 0 {
                return;
            }
        }
    }

    /// For each sample, the distance to the nearest sample of another line,
    /// or of the same line more than a few steps along it.
    fn feature_gaps(&self, settings: &Settings) -> Vec<Vec<f64>> {
        let mut index = Protection::new(settings.max_step);
        for (l, line) in self.lines.iter().enumerate() {
            for (i, &p) in line.iter().enumerate() {
                index.add(
                    p,
                    0.0,
                    BallId {
                        line: narrow(l),
                        index: narrow(i),
                    },
                );
            }
        }
        self.lines
            .iter()
            .enumerate()
            .map(|(l, line)| {
                let n = line.len();
                (0..n)
                    .map(|i| {
                        index
                            .nearest(line[i], 2.0 * settings.max_step, |ball| {
                                if ball.line as usize != l {
                                    return true;
                                }
                                let along = (ball.index as usize).abs_diff(i);
                                let along = if self.closed[l] {
                                    along.min(n - along)
                                } else {
                                    along
                                };
                                along > 3
                            })
                            .unwrap_or(f64::INFINITY)
                    })
                    .collect()
            })
            .collect()
    }
}

/// Splits each marked segment of `line`, segment `i` running from sample
/// `i` to the next, at the crease point nearest its middle, unless it is
/// already within twice the minimum step. Returns the segments split.
fn subdivide(
    sampler: &Sampler<'_>,
    line: &mut Vec<[f64; 3]>,
    closed: bool,
    marked: &[bool],
    settings: &Settings,
) -> usize {
    if !marked.contains(&true) {
        return 0;
    }
    let n = line.len();
    let mut count = 0;
    let mut next = Vec::with_capacity(n * 2);
    for i in 0..n {
        next.push(line[i]);
        let following = line[(i + 1) % n];
        if marked.get(i).copied().unwrap_or(false)
            && i < segments_of(n, closed)
            && distance(line[i], following) > 2.0 * settings.min_step
            && let Some(m) = sampler.crease_point_along(
                midpoint(line[i], following),
                settings.cell,
                tie_normal(sampler, line[i]),
            )
        {
            next.push(m);
            count += 1;
        }
    }
    *line = next;
    count
}

/// Where creases ending at `members` meet when tracing found no corner
/// there: the field's corner, where it and two ties are zero, when Newton
/// steps from their tangent lines' nearest point find one; otherwise the
/// point on a crease nearest that estimate, or failing that the nearest of
/// the ends themselves, so the corner always lies on the surface.
fn corner_position(
    sampler: &Sampler<'_>,
    lines: &[Vec<[f64; 3]>],
    ends: &[(usize, bool, [f64; 3], f64)],
    members: &[usize],
    settings: &Settings,
) -> [f64; 3] {
    #[expect(clippy::cast_precision_loss, reason = "a handful of endpoints")]
    let count = members.len() as f64;
    let centroid = members.iter().fold([0.0; 3], |sum, &e| {
        let p = ends[e].2;
        core::array::from_fn(|k| sum[k] + p[k] / count)
    });
    // Least squares: the point nearest every crease's tangent line.
    let mut m = [[0.0; 3]; 3];
    let mut rhs = [0.0; 3];
    let mut directions = 0;
    for &e in members {
        let (l, start, p, _) = ends[e];
        let line = &lines[l];
        if line.len() < 2 {
            continue;
        }
        let q = if start { line[1] } else { line[line.len() - 2] };
        let Some(d) = unit(sub(p, q)) else {
            continue;
        };
        for i in 0..3 {
            for j in 0..3 {
                let projector = f64::from(u8::from(i == j)) - d[i] * d[j];
                m[i][j] += projector;
                rhs[i] += projector * p[j];
            }
        }
        directions += 1;
    }
    let determinant = dot(m[0], cross(m[1], m[2]));
    let mut estimate = centroid;
    if directions >= 2 && determinant.abs() > 1.0e-9 {
        let solve = |column: usize| {
            let mut a = m;
            for row in 0..3 {
                a[row][column] = rhs[row];
            }
            dot(a[0], cross(a[1], a[2])) / determinant
        };
        let solved = [solve(0), solve(1), solve(2)];
        if distance(solved, centroid) <= 3.0 * settings.max_step {
            estimate = solved;
        }
    }
    let on_surface = |p: [f64; 3]| sampler.value(p).abs() <= settings.on_surface;
    sampler
        .corner_point(estimate, settings.max_step)
        .filter(|&p| on_surface(p))
        .or_else(|| {
            sampler
                .crease_point(estimate, settings.max_step)
                .filter(|&p| on_surface(p))
        })
        .unwrap_or_else(|| {
            members
                .iter()
                .map(|&e| ends[e].2)
                .min_by(|a, b| distance(*a, estimate).total_cmp(&distance(*b, estimate)))
                .unwrap_or(centroid)
        })
}

/// Whether two tie normals could belong to one crease: they lie within
/// about 25 degrees, either way round, or one is unknown.
fn same_tie(a: Option<[f64; 3]>, b: Option<[f64; 3]>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => dot(a, b).abs() > 0.9,
        _ => true,
    }
}

/// The samples traced so far, each with its tie normal, so that a crease
/// stops only at samples of its own: across a fin thinner than a ball,
/// the crease on the far side is traced too.
struct Traced {
    balls: Protection,
    normals: Vec<Vec<Option<[f64; 3]>>>,
    settings: Settings,
}

impl Traced {
    fn new(settings: &Settings) -> Self {
        Self {
            balls: Protection::new(settings.max_step),
            normals: Vec::new(),
            settings: *settings,
        }
    }

    fn add(&mut self, sampler: &Sampler<'_>, line: &[[f64; 3]]) {
        let id = narrow(self.normals.len());
        for (index, &p) in line.iter().enumerate() {
            let ball = BallId {
                line: id,
                index: narrow(index),
            };
            self.balls
                .add(p, ball_radius(line, false, index, &self.settings), ball);
        }
        self.normals
            .push(line.iter().map(|&p| tie_normal(sampler, p)).collect());
    }

    /// Whether a ball scaled by `scale` holds `p`, on a crease whose tie
    /// has unit normal `normal`, and the crease runs on to the ball's
    /// sample: parallel creases closer than a ball, as along a thin rod,
    /// are traced apart even where their ties are parallel too.
    fn holds(
        &self,
        sampler: &Sampler<'_>,
        p: [f64; 3],
        scale: f64,
        normal: Option<[f64; 3]>,
    ) -> bool {
        self.balls.around(p, 1).any(|&(q, r, ball)| {
            let theirs = self.normals[ball.line as usize][ball.index as usize];
            distance(p, q) < scale * r
                && same_tie(normal, theirs)
                && normal.or(theirs).is_none_or(|n| {
                    distance(p, q) == 0.0 || strays(sampler, p, q, n, &self.settings).is_some()
                })
        })
    }
}

/// The unit normal of the tie nearest `p`: for a point on a crease, the
/// crease's own, the difference of its two pieces' normals.
fn tie_normal(sampler: &Sampler<'_>, p: [f64; 3]) -> Option<[f64; 3]> {
    unit(sampler.crease(p).ties[0]?.function.gradient)
}

/// The step from a line's start or end to the sample beside it, at least
/// the minimum step.
fn end_step(line: &[[f64; 3]], start: bool, settings: &Settings) -> f64 {
    if line.len() < 2 {
        return settings.min_step;
    }
    let (a, b) = if start {
        (line[0], line[1])
    } else {
        (line[line.len() - 1], line[line.len() - 2])
    };
    distance(a, b).max(settings.min_step)
}

/// A point on each crease leaving `corner`, two minimum steps out, or less
/// where another of `corners` is close: the creases that probes in the 26
/// directions to a cube's neighbors land on nearer `corner` than any other,
/// one point for each direction and tie.
fn ports(
    sampler: &Sampler<'_>,
    corner: [f64; 3],
    corners: &[[f64; 3]],
    settings: &Settings,
) -> Vec<[f64; 3]> {
    let nearest = corners
        .iter()
        .map(|&k| distance(k, corner))
        .filter(|&d| d > 0.0)
        .fold(f64::INFINITY, f64::min);
    let reach = (2.0 * settings.min_step).min(0.4 * nearest);
    let mut ports: Vec<[f64; 3]> = Vec::new();
    for index in 0..27_i32 {
        let direction = [index % 3 - 1, index / 3 % 3 - 1, index / 9 - 1].map(f64::from);
        let Some(direction) = unit(direction) else {
            continue;
        };
        let probe: [f64; 3] = core::array::from_fn(|k| corner[k] + reach * direction[k]);
        let Some(x) = sampler.crease_point(probe, reach) else {
            continue;
        };
        // A crease of the surface: on it, and where its tie shapes the
        // field, not where two pieces that do not happen to be equal.
        let crease = sampler.crease(x);
        let genuine = crease.field.value.abs() <= settings.on_surface
            && crease.ties[0].is_some_and(|tie| tie.distance <= settings.tolerance);
        let own = corners
            .iter()
            .all(|&k| k == corner || distance(x, k) > distance(x, corner));
        if !genuine
            || !own
            || !(0.5 * reach..=2.0 * reach).contains(&distance(x, corner))
            || !settings.reach.contains(x)
        {
            continue;
        }
        let (Some(leaving), normal) = (unit(sub(x, corner)), tie_normal(sampler, x)) else {
            continue;
        };
        let known = ports.iter().any(|&p| {
            unit(sub(p, corner)).is_some_and(|d| dot(d, leaving) > 0.9)
                && same_tie(normal, tie_normal(sampler, p))
        });
        if !known {
            ports.push(x);
        }
    }
    ports
}

/// The crease through `port`, near `corner`, traced both ways within a short
/// walk: its samples in order, and the corners it runs from and to, when
/// one of them is not `corner`.
///
/// The arc runs from a corner tracing found, which may be one beside
/// `corner`, or else from `corner` itself if the crease runs straight on to
/// it.
fn arc(
    sampler: &Sampler<'_>,
    corner: [f64; 3],
    port: [f64; 3],
    settings: &Settings,
) -> Option<Arc> {
    let short = Settings {
        max_crease_samples: 64,
        ..*settings
    };
    let (mut line, [behind, ahead]) = trace(sampler, port, &short, &Traced::new(settings));
    let far = |end: &End| match *end {
        End::Corner(x) if distance(x, corner) > settings.min_step => Some(x),
        _ => None,
    };
    let (start, other) = match (far(&ahead), far(&behind)) {
        (Some(x), _) => (behind, x),
        (None, Some(x)) => {
            line.reverse();
            (ahead, x)
        }
        (None, None) => return None,
    };
    // Where tracing back stopped at a corner, the arc starts there, which
    // may be a corner beside this one; where it stopped short, its crease
    // must run straight on to this corner.
    let start = match start {
        End::Corner(x) => x,
        End::Open => {
            let normal = tie_normal(sampler, line[0])?;
            strays(sampler, line[0], corner, normal, settings)?;
            corner
        }
    };
    (distance(start, other) > settings.min_step).then_some(Arc {
        samples: line,
        from: start,
        to: other,
    })
}

/// A crease traced between two corners.
struct Arc {
    samples: Vec<[f64; 3]>,
    from: [f64; 3],
    to: [f64; 3],
}

/// The crease point midway between corners `a` and `b` on the crease that
/// `line` traced between them, when they are far enough apart to put one
/// between, and the line ran through the middle half of the way or past
/// the middle; not a line beside one corner.
fn short_edge(
    sampler: &Sampler<'_>,
    a: [f64; 3],
    b: [f64; 3],
    line: &[[f64; 3]],
    settings: &Settings,
) -> Option<[f64; 3]> {
    let gap = distance(a, b);
    if gap <= 2.0 * settings.min_step {
        return None;
    }
    let center = midpoint(a, b);
    // How far along the way from `a` to `b` each sample lies.
    let along = |p: [f64; 3]| dot(sub(p, a), sub(b, a)) / (gap * gap);
    let past = line.iter().any(|&p| along(p) < 0.5) && line.iter().any(|&p| along(p) > 0.5);
    let sample = line
        .iter()
        .copied()
        .filter(|&p| past || distance(p, center) < 0.25 * gap)
        .min_by(|p, q| distance(*p, center).total_cmp(&distance(*q, center)))?;
    let middle = sampler.crease_point_along(center, 0.5 * gap, tie_normal(sampler, sample))?;
    (distance(middle, a) > 0.25 * gap && distance(middle, b) > 0.25 * gap).then_some(middle)
}

/// How far the straight way from `end` to `corner` strays from the crease
/// through `end`, whose tie has unit `normal`, if the crease runs on to the
/// corner: halfway there, the surface and a tie continuing the crease both
/// lie within the sag a chord of that length may have, plus a little.
///
/// A crease ending at a corner nearby passes; one running past it, through
/// the solid, or to a corner on a neighboring crease far enough away, does
/// not. The stray tells a corner on the crease from one beside it.
fn strays(
    sampler: &Sampler<'_>,
    end: [f64; 3],
    corner: [f64; 3],
    normal: [f64; 3],
    settings: &Settings,
) -> Option<f64> {
    let gap = distance(end, corner);
    let tolerance = 0.05 * gap + 2.5 * settings.max_distance;
    let crease = sampler.crease(midpoint(end, corner));
    let off_surface = crease.field.value.abs() / norm(crease.field.gradient);
    let off_crease = following(crease, Some(normal))?.distance;
    let stray = off_surface.max(off_crease);
    (stray <= tolerance).then_some(stray)
}

/// The distance from `p` to the segment from `a` to `b`.
fn segment_distance(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = sub(b, a);
    let length2 = dot(ab, ab);
    let t = if length2 > 0.0 {
        (dot(sub(p, a), ab) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(p, core::array::from_fn(|k| a[k] + t * ab[k]))
}

/// A protecting ball overlapping its neighbors along the crease.
fn ball_radius(line: &[[f64; 3]], closed: bool, index: usize, settings: &Settings) -> f64 {
    let reach = steps_beside(line, closed, index)
        .into_iter()
        .flatten()
        .fold(0.0, f64::max)
        .max(settings.min_step);
    settings.protect * reach
}

/// The number of segments between the `n` samples of a line.
fn segments_of(n: usize, closed: bool) -> usize {
    if closed { n } else { n.saturating_sub(1) }
}

/// The segment ending at sample `i`, if any.
fn segment_before(n: usize, closed: bool, i: usize) -> Option<usize> {
    match i.checked_sub(1) {
        Some(before) => Some(before),
        None => closed.then(|| n - 1),
    }
}

/// The lengths of the segments ending and starting at sample `i`.
fn steps_beside(line: &[[f64; 3]], closed: bool, i: usize) -> [Option<f64>; 2] {
    let n = line.len();
    let before = segment_before(n, closed, i).map(|b| distance(line[b], line[i]));
    let after = (i < segments_of(n, closed)).then(|| distance(line[i], line[(i + 1) % n]));
    [before, after]
}

/// Walks a crease both ways from `seed`, projecting each step back onto it.
///
/// Steps start at the maximum and halve until the chord sags no more than
/// the distance tolerance from the crease, the pieces' normals and the
/// crease's direction turn by no more than the turning limit, and the step
/// stays clear of lines already traced, down to the minimum step. Each half stops at a corner, where a second tie reaches
/// zero, or where the crease ends, stalls, or meets samples already taken.
/// Returns the samples in order along the crease and how each half ended.
fn trace(
    sampler: &Sampler<'_>,
    seed: [f64; 3],
    settings: &Settings,
    traced: &Traced,
) -> (Vec<[f64; 3]>, [End; 2]) {
    let mut halves: [Vec<[f64; 3]>; 2] = [Vec::new(), Vec::new()];
    let mut ends = [End::Open, End::Open];
    for (half, direction) in [1.0, -1.0].into_iter().enumerate() {
        let mut x = seed;
        let mut previous: Option<[f64; 3]> = None;
        // The normal of the tie being followed, from the seed's nearest.
        let mut normal: Option<[f64; 3]> = None;
        'walk: for _ in 0..settings.max_crease_samples {
            let Some(frame) = frame(sampler, x, normal) else {
                break;
            };
            normal = Some(frame[1]);
            let sign = previous.map_or(direction, |p| dot(p, frame[2]).signum());
            let tangent = frame[2].map(|v| v * sign);
            let mut step = settings.max_step;
            let y = loop {
                let predicted: [f64; 3] = core::array::from_fn(|k| x[k] + step * tangent[k]);
                let reach = settings.cell.max(step);
                if let Some(y) = sampler.crease_point_along(predicted, reach, normal) {
                    // Stop short of a line already traced, so the two can
                    // be joined end to end.
                    if step > settings.min_step && traced.holds(sampler, y, 0.9, normal) {
                        step *= 0.5;
                        continue;
                    }
                    let middle = midpoint(x, y);
                    let sag = sampler
                        .crease_point_along(middle, reach, normal)
                        .map(|m| distance(m, middle));
                    let turned = deviation(sampler, x, y, normal)
                        .is_none_or(|angle| angle > settings.max_turn);
                    if (sag.is_some_and(|sag| sag <= settings.max_distance) && !turned)
                        || step <= settings.min_step
                    {
                        break y;
                    }
                } else if step <= settings.min_step {
                    break 'walk;
                }
                step *= 0.5;
            };
            let gap = distance(x, y);
            if let Some(corner) = corner_ahead(sampler, x, y, gap, settings) {
                ends[half] = End::Corner(corner);
                break;
            }
            if sampler.value(y).abs() > settings.on_surface
                || gap < 0.4 * settings.min_step
                || !settings.reach.contains(y)
            {
                break;
            }
            if traced.holds(sampler, y, 0.9, normal)
                || distance(seed, y) < 0.5 * gap
                || halves.iter().flatten().any(|&q| distance(q, y) < 0.5 * gap)
            {
                break;
            }
            halves[half].push(y);
            previous = Some(tangent);
            x = y;
        }
    }
    let [forward, backward] = halves;
    let [ahead, behind] = ends;
    (
        backward
            .into_iter()
            .rev()
            .chain([seed])
            .chain(forward)
            .collect(),
        [behind, ahead],
    )
}

/// The corner between `x` and `y` on a crease, if another crease meets it
/// there: where the second-nearest tie reaches zero within the step.
fn corner_ahead(
    sampler: &Sampler<'_>,
    x: [f64; 3],
    y: [f64; 3],
    gap: f64,
    settings: &Settings,
) -> Option<[f64; 3]> {
    let second = sampler.crease(y).ties[1]?;
    if second.distance >= gap {
        return None;
    }
    let corner = sampler.corner_point(y, gap.max(settings.min_step))?;
    // The corner lies about where the step crossed the second tie.
    let near = distance(corner, x) <= 1.5 * gap.max(settings.min_step);
    (near && sampler.value(corner).abs() <= settings.on_surface).then_some(corner)
}

/// The unit field normal, tie normal and crease tangent at `p`.
fn frame(sampler: &Sampler<'_>, p: [f64; 3], normal: Option<[f64; 3]>) -> Option<[[f64; 3]; 3]> {
    let crease = sampler.crease(p);
    let tie = following(crease, normal)?;
    let (field, tie) = (crease.field.gradient, tie.function.gradient);
    Some([unit(field)?, unit(tie)?, unit(cross(field, tie))?])
}

/// The largest angle through which the crease's tangent, the field's
/// normal, or the tie's normal turns between two crease points.
fn deviation(
    sampler: &Sampler<'_>,
    x: [f64; 3],
    y: [f64; 3],
    normal: Option<[f64; 3]>,
) -> Option<f64> {
    let a = frame(sampler, x, normal)?;
    let b = frame(sampler, y, Some(a[1]))?;
    // Directions only: a flipped tie or tangent is not a turn.
    Some(
        (0..3)
            .map(|k| exedra_math::Real::acos(dot(a[k], b[k]).abs().min(1.0)))
            .fold(0.0, f64::max),
    )
}

/// A ball's center, radius and identity.
type Ball = ([f64; 3], f64, BallId);

/// Protecting balls, found by point.
pub(super) struct Protection {
    cell: f64,
    cells: HashMap<[i64; 3], Vec<Ball>>,
}

impl Protection {
    /// An empty set of balls, binned in cubes of side `cell`, which should
    /// be about the largest radius.
    pub(super) fn new(cell: f64) -> Self {
        Self {
            cell,
            cells: HashMap::new(),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "bins of finite points near the origin"
    )]
    fn key(&self, p: [f64; 3]) -> [i64; 3] {
        p.map(|v| {
            let q = v / self.cell;
            let whole = q as i64;
            if (whole as f64) > q { whole - 1 } else { whole }
        })
    }

    pub(super) fn add(&mut self, p: [f64; 3], radius: f64, ball: BallId) {
        let key = self.key(p);
        self.cells.entry(key).or_default().push((p, radius, ball));
    }

    /// Every ball in the cubes within `rings` of `p`'s.
    fn around(&self, p: [f64; 3], rings: i64) -> impl Iterator<Item = &Ball> {
        let [i, j, k] = self.key(p);
        (-rings..=rings).flat_map(move |a| {
            (-rings..=rings).flat_map(move |b| {
                (-rings..=rings)
                    .flat_map(move |c| self.cells.get(&[i + a, j + b, k + c]).into_iter().flatten())
            })
        })
    }

    /// The ball holding `p`, if any; the least by identity when several
    /// do. Radii are at most the cube side.
    pub(super) fn covering(&self, p: [f64; 3]) -> Option<BallId> {
        self.around(p, 1)
            .filter(|&&(q, r, _)| distance(p, q) < r)
            .map(|&(_, _, ball)| ball)
            .min()
    }

    /// The distance from `p` to the nearest center that `keep` accepts,
    /// if one lies within `reach`, which should be at most twice the cube
    /// side.
    fn nearest(&self, p: [f64; 3], reach: f64, keep: impl Fn(BallId) -> bool) -> Option<f64> {
        self.around(p, 2)
            .filter(|&&(_, _, ball)| keep(ball))
            .map(|&(q, _, _)| distance(p, q))
            .filter(|&d| d < reach)
            .min_by(f64::total_cmp)
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "counts bounded by the point limit"
)]
fn narrow(index: usize) -> u32 {
    index as u32
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra_implicit::{Aabb, Cuboid, Cylinder, Intersection};

    use super::super::{DelaunayParams, Settings};
    use super::{Creases, Sampler};

    #[test]
    fn a_short_line_in_a_loops_gap_joins_it() {
        // A post's circular rim, traced most of the way round from one seed
        // and across the rest from another.
        let post = Intersection::new(
            Cuboid::new([0.0; 3], [1.0; 3]).unwrap(),
            Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.72, 1.35).unwrap(),
        );
        let sampler = Sampler::new(&post);
        let bounds = Aabb::new([-1.5; 3], [1.5; 3]).unwrap();
        let settings = Settings::new(&DelaunayParams::new(bounds, 3.0 / 32.0)).unwrap();
        let at = |angle: f64| {
            let (sin, cos) = exedra_math::Real::sin_cos(angle);
            [0.72 * cos, 0.72 * sin, 1.0]
        };
        let around: Vec<[f64; 3]> = (0..68).map(|i| at(0.09 * f64::from(i))).collect();
        let gap = alloc::vec![at(6.10), at(6.19)];
        let mut creases = Creases {
            lines: alloc::vec![around, gap],
            ..Creases::default()
        };
        let mut stops = alloc::vec![[None; 2]; 2];
        creases.join_continuations(&sampler, &mut stops, &[], &settings);
        assert_eq!(creases.lines.len(), 1);
        assert_eq!(creases.lines[0].len(), 70);
    }
}
