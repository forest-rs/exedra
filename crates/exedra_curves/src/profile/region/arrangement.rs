// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::profile::arc::{Piece, bounds_may_contact, cross, length, sub, turns, winding_angle};
use crate::profile::{Arc, Curve, Loop, LoopRef, ProfilePolicy, SegTag, Segment};
use alloc::{collections::BTreeMap, vec, vec::Vec};

#[derive(Copy, Clone)]
struct Input {
    piece: Piece,
    center_defined: bool,
    source: InputSegment,
    tag: Option<SegTag>,
    bounds: [[f64; 2]; 2],
}

#[derive(Clone)]
struct CutEvent {
    input: usize,
    parameter: f64,
    node: usize,
}

#[derive(Clone)]
struct Fragment {
    from: usize,
    to: usize,
    curve: Curve,
    tag: Option<SegTag>,
    sources: [Option<SegmentSource>; 2],
}

#[derive(Default)]
struct Nodes {
    points: Vec<[f64; 2]>,
    ids: BTreeMap<[u64; 2], usize>,
}
impl Nodes {
    fn insert(&mut self, p: [f64; 2]) -> usize {
        let p = p.map(|x| x + 0.0);
        *self.ids.entry(p.map(f64::to_bits)).or_insert_with(|| {
            let id = self.points.len();
            self.points.push(p);
            id
        })
    }
}

impl Profile {
    /// Combines filled regions while retaining analytic line and arc boundaries.
    ///
    /// Intersections split source segments, and a boundary is retained only
    /// where the operation's material changes sides. Results may be empty or
    /// disconnected. Tags stay with retained segments; for a coincident run,
    /// the left tag wins when present, while correspondence retains both inputs.
    /// No mesh approximation is used. Tangent point contacts that would branch
    /// a boundary are refused, as are arrangements too close to validate.
    ///
    /// # Errors
    ///
    /// Returns typed input, budget, numerical or output-arrangement refusals.
    pub fn combine(
        &self,
        other: &Self,
        op: RegionOp,
        policy: &RegionPolicy,
    ) -> Result<RegionResult, RegionError> {
        for (resource, maximum) in [
            (RegionBudget::PairTests, policy.max_pair_tests),
            (
                RegionBudget::ClassificationTests,
                policy.max_classification_tests,
            ),
            (RegionBudget::Events, u64::from(policy.max_events)),
            (RegionBudget::Fragments, u64::from(policy.max_fragments)),
        ] {
            if maximum == 0 {
                return Err(RegionError::InvalidBudget { resource, maximum });
            }
        }
        for (operand, profile) in [(Operand::Left, self), (Operand::Right, other)] {
            profile
                .validate(&policy.profile)
                .map_err(|error| RegionError::Input { operand, error })?;
        }
        let left = inputs(self, Operand::Left);
        let split = left.len();
        let mut input = left;
        input.extend(inputs(other, Operand::Right));
        let mut nodes = Nodes::default();
        let mut cuts = Vec::with_capacity(input.len() * 2);
        for (index, item) in input.iter().enumerate() {
            cuts.push(CutEvent {
                input: index,
                parameter: 0.0,
                node: nodes.insert(item.piece.from()),
            });
            cuts.push(CutEvent {
                input: index,
                parameter: 1.0,
                node: nodes.insert(item.piece.to()),
            });
        }
        let mut report = RegionReport::default();
        for a in 0..split {
            for b in split..input.len() {
                charge(
                    RegionBudget::PairTests,
                    report.pair_tests + 1,
                    policy.max_pair_tests,
                )?;
                report.pair_tests += 1;
                intersect(a, b, &input, &mut cuts, &mut nodes, policy, &mut report)?;
            }
        }
        let mut fragments = Vec::new();
        cuts.sort_unstable_by(|a, b| {
            a.input
                .cmp(&b.input)
                .then_with(|| a.parameter.total_cmp(&b.parameter))
                .then_with(|| a.node.cmp(&b.node))
        });
        cuts.dedup_by(|a, b| a.input == b.input && a.node == b.node);
        let bounds = [self.bounds(), other.bounds()];
        for pair in cuts.windows(2) {
            if pair[0].input != pair[1].input || pair[0].node == pair[1].node {
                continue;
            }
            charge(
                RegionBudget::Fragments,
                u64::from(report.fragments) + 1,
                u64::from(policy.max_fragments),
            )?;
            report.fragments += 1;
            if let Some(fragment) = classify(
                pair[0].input,
                pair,
                &input,
                &nodes,
                bounds,
                op,
                policy,
                &mut report,
            )? {
                fragments.push(fragment);
            }
        }
        let fragments = deduplicate(fragments, &nodes)?;
        report.retained_segments = fragments.len();
        assemble(fragments, &nodes, &policy.profile, report)
    }
}

fn charge(resource: RegionBudget, required: u64, maximum: u64) -> Result<(), RegionError> {
    if required > maximum {
        Err(RegionError::Budget {
            resource,
            maximum,
            required,
        })
    } else {
        Ok(())
    }
}

fn inputs(profile: &Profile, operand: Operand) -> Vec<Input> {
    core::iter::once((LoopRef::Outer, profile.outer()))
        .chain(
            profile
                .holes()
                .iter()
                .enumerate()
                .map(|(i, l)| (LoopRef::Hole(i), l)),
        )
        .flat_map(|(loop_ref, l)| {
            l.pieces()
                .zip(l.segments())
                .enumerate()
                .map(move |(segment, (piece, s))| Input {
                    piece,
                    center_defined: matches!(s.curve, Curve::ArcCenter { .. }),
                    source: InputSegment {
                        operand,
                        segment: SegmentRef { loop_ref, segment },
                    },
                    tag: s.tag,
                    bounds: piece.bounds(),
                })
        })
        .collect()
}

fn tolerance(a: Piece, b: Piece) -> f64 {
    let scale = a
        .from()
        .into_iter()
        .chain(a.to())
        .chain(b.from())
        .chain(b.to())
        .map(f64::abs)
        .fold(f64::MIN_POSITIVE, f64::max);
    64.0 * f64::EPSILON * scale
}

fn parameter(piece: Piece, p: [f64; 2]) -> f64 {
    if p == piece.from() {
        return 0.0;
    }
    if p == piece.to() {
        return 1.0;
    }
    match piece {
        Piece::Line(a, b) => {
            let d = sub(b, a);
            let axis = usize::from(d[1].abs() > d[0].abs());
            (p[axis] - a[axis]) / d[axis]
        }
        Piece::Arc(arc) => {
            let a = sub(arc.from, arc.center);
            let b = sub(p, arc.center);
            let mut angle = libm::atan2(cross(a, b), a[0] * b[0] + a[1] * b[1]);
            if arc.sweep > 0.0 && angle < 0.0 {
                angle += core::f64::consts::TAU;
            }
            if arc.sweep < 0.0 && angle > 0.0 {
                angle -= core::f64::consts::TAU;
            }
            angle / arc.sweep
        }
    }
}

pub(super) fn point_at(piece: Piece, t: f64) -> [f64; 2] {
    match piece {
        Piece::Line(a, b) => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
        Piece::Arc(a) => a.point_at(t),
    }
}

fn intersect(
    a: usize,
    b: usize,
    input: &[Input],
    cuts: &mut Vec<CutEvent>,
    nodes: &mut Nodes,
    policy: &RegionPolicy,
    report: &mut RegionReport,
) -> Result<(), RegionError> {
    if !bounds_may_contact(input[a].bounds, input[b].bounds, 0.0) {
        return Ok(());
    }
    let (p, q) = (input[a].piece, input[b].piece);
    let tol = tolerance(p, q);
    // Two supporting-curve crossings plus four authored endpoints is the
    // fixed maximum. Contacts need no per-pair heap allocation.
    let mut events = [None; 6];
    let mut count = 0;
    for point in p.crossings(&q).iter() {
        events[count] = Some(point);
        count += 1;
    }
    for endpoint in [p.from(), p.to(), q.from(), q.to()] {
        if p.distance_to(endpoint) <= tol
            && q.distance_to(endpoint) <= tol
            && !events
                .iter()
                .flatten()
                .any(|&v| length(sub(v, endpoint)) <= tol)
        {
            events[count] = Some(endpoint);
            count += 1;
        }
    }
    for mut point in events.into_iter().flatten() {
        // Authored endpoints remain authoritative when the intersection solver
        // reaches them within its scale-dependent floating-point residual.
        for endpoint in [p.from(), p.to(), q.from(), q.to()] {
            if length(sub(point, endpoint)) <= tol {
                point = endpoint;
                break;
            }
        }
        let residual = p.distance_to(point).max(q.distance_to(point));
        if residual > policy.profile.min_separation * 0.125 {
            return Err(RegionError::Ambiguous {
                first: input[a].source,
                second: input[b].source,
            });
        }
        let (u, v) = (parameter(p, point), parameter(q, point));
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            continue;
        }
        charge(
            RegionBudget::Events,
            u64::from(report.events) + 1,
            u64::from(policy.max_events),
        )?;
        report.events += 1;
        report.max_intersection_residual = report.max_intersection_residual.max(residual);
        let node = nodes.insert(point);
        cuts.push(CutEvent {
            input: a,
            parameter: u,
            node,
        });
        cuts.push(CutEvent {
            input: b,
            parameter: v,
            node,
        });
    }
    Ok(())
}

fn same_support(a: Piece, b: Piece, tol: f64) -> bool {
    match (a, b) {
        (Piece::Line(p, q), Piece::Line(r, s)) => {
            let d = sub(q, p);
            let e = sub(s, r);
            cross(d, e).abs() <= 64.0 * f64::EPSILON * length(d) * length(e)
                && cross(d, sub(r, p)).abs() <= tol * length(d)
        }
        (Piece::Arc(a), Piece::Arc(b)) => {
            length(sub(a.center, b.center)) <= tol && (a.radius - b.radius).abs() <= tol
        }
        _ => false,
    }
}

fn classify(
    index: usize,
    pair: &[CutEvent],
    input: &[Input],
    nodes: &Nodes,
    bounds: [[[f64; 2]; 2]; 2],
    op: RegionOp,
    policy: &RegionPolicy,
    report: &mut RegionReport,
) -> Result<Option<Fragment>, RegionError> {
    let item = input[index];
    let side = usize::from(item.source.operand == Operand::Right);
    let mid = point_at(item.piece, 0.5 * (pair[0].parameter + pair[1].parameter));
    let mut other_sides = None;
    let mut winding = 0.0;
    let candidates = if bounds_may_contact(bounds[1 - side], [mid, mid], 0.0) {
        input
    } else {
        &[]
    };
    for other in candidates
        .iter()
        .filter(|i| i.source.operand != item.source.operand)
    {
        charge(
            RegionBudget::ClassificationTests,
            report.classification_tests + 1,
            policy.max_classification_tests,
        )?;
        report.classification_tests += 1;
        winding += winding_angle(&other.piece, mid);
        let tol = tolerance(item.piece, other.piece);
        if other.piece.distance_to(mid) <= tol {
            if !same_support(item.piece, other.piece, tol) {
                return Err(RegionError::Ambiguous {
                    first: item.source,
                    second: other.source,
                });
            }
            let a = item.piece.tangent(false);
            let b = other.piece.tangent(false);
            let aligned = match (item.piece, other.piece) {
                (Piece::Arc(a), Piece::Arc(b)) => a.sweep.signum() == b.sweep.signum(),
                _ => a[0] * b[0] + a[1] * b[1] > 0.0,
            };
            let value = [aligned, !aligned];
            if other_sides.is_some_and(|old| old != value) {
                return Err(RegionError::Ambiguous {
                    first: item.source,
                    second: other.source,
                });
            }
            other_sides = Some(value);
        }
    }
    let other_sides = other_sides.unwrap_or_else(|| {
        let value = turns(winding) != 0;
        [value, value]
    });
    let (left, right) = if side == 0 {
        (
            op.contains(true, other_sides[0]),
            op.contains(false, other_sides[1]),
        )
    } else {
        (
            op.contains(other_sides[0], true),
            op.contains(other_sides[1], false),
        )
    };
    if left == right {
        return Ok(None);
    }
    let reversed = right;
    let (from, to, parameters) = if reversed {
        (
            pair[1].node,
            pair[0].node,
            [pair[1].parameter, pair[0].parameter],
        )
    } else {
        (
            pair[0].node,
            pair[1].node,
            [pair[0].parameter, pair[1].parameter],
        )
    };
    let curve = match item.piece {
        Piece::Line(..) => Curve::Line,
        Piece::Arc(arc) if item.center_defined => Curve::ArcCenter {
            center: arc.center,
            clockwise: arc.sweep * (parameters[1] - parameters[0]) < 0.0,
        },
        Piece::Arc(arc) => Curve::Arc {
            bulge: libm::tan(arc.sweep * (parameters[1] - parameters[0]) * 0.25),
        },
    };
    if nodes.points[from] == nodes.points[to] {
        return Err(RegionError::Loop(LoopError::ZeroLength {
            segment: item.source.segment.segment,
        }));
    }
    let mut sources = [None, None];
    sources[side] = Some(SegmentSource {
        segment: item.source.segment,
        parameters,
    });
    Ok(Some(Fragment {
        from,
        to,
        curve,
        tag: item.tag,
        sources,
    }))
}

fn fragment_piece(f: &Fragment, nodes: &Nodes) -> Result<Piece, RegionError> {
    match f.curve {
        Curve::Line => Ok(Piece::Line(nodes.points[f.from], nodes.points[f.to])),
        curve => Arc::resolve(nodes.points[f.from], nodes.points[f.to], curve)
            .ok()
            .flatten()
            .map(Piece::Arc)
            .ok_or(RegionError::Loop(LoopError::InvalidArc { segment: 0 })),
    }
}

fn deduplicate(fragments: Vec<Fragment>, nodes: &Nodes) -> Result<Vec<Fragment>, RegionError> {
    let mut retained: Vec<Fragment> = Vec::new();
    let mut by_ends: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for fragment in fragments {
        if let Some(&index) = by_ends.get(&(fragment.from, fragment.to)) {
            let a = fragment_piece(&retained[index], nodes)?;
            let b = fragment_piece(&fragment, nodes)?;
            if !same_support(a, b, tolerance(a, b))
                || length(sub(a.midpoint(), b.midpoint())) > tolerance(a, b)
            {
                return Err(RegionError::NonManifold {
                    point: nodes.points[fragment.from],
                    incoming: 1,
                    outgoing: 2,
                });
            }
            let existing = &mut retained[index];
            for side in 0..2 {
                if existing.sources[side].is_none() {
                    existing.sources[side] = fragment.sources[side];
                }
            }
            if existing.tag.is_none() {
                existing.tag = fragment.tag;
            }
        } else {
            by_ends.insert((fragment.from, fragment.to), retained.len());
            retained.push(fragment);
        }
    }
    Ok(retained)
}

struct Ring {
    loop_: Loop,
    fragments: Vec<usize>,
}

fn assemble(
    fragments: Vec<Fragment>,
    nodes: &Nodes,
    policy: &ProfilePolicy,
    mut report: RegionReport,
) -> Result<RegionResult, RegionError> {
    let mut outgoing = vec![None; nodes.points.len()];
    let mut incoming = vec![0_usize; nodes.points.len()];
    for (index, fragment) in fragments.iter().enumerate() {
        incoming[fragment.to] += 1;
        if outgoing[fragment.from].replace(index).is_some() {
            return Err(RegionError::NonManifold {
                point: nodes.points[fragment.from],
                incoming: incoming[fragment.from],
                outgoing: 2,
            });
        }
    }
    for (id, &count) in incoming.iter().enumerate() {
        if count != usize::from(outgoing[id].is_some()) {
            return Err(RegionError::NonManifold {
                point: nodes.points[id],
                incoming: count,
                outgoing: usize::from(outgoing[id].is_some()),
            });
        }
    }
    let mut visited = vec![false; fragments.len()];
    let mut rings = Vec::new();
    for root in 0..fragments.len() {
        if visited[root] {
            continue;
        }
        let mut chain = Vec::new();
        let mut segments = Vec::new();
        let mut next = root;
        loop {
            if visited[next] {
                break;
            }
            visited[next] = true;
            chain.push(next);
            let f = &fragments[next];
            segments.push(Segment {
                to: nodes.points[f.to],
                curve: f.curve,
                tag: f.tag,
            });
            next = outgoing[f.to].expect("balanced retained nodes");
        }
        rings.push(Ring {
            loop_: Loop::new(segments).map_err(RegionError::Loop)?,
            fragments: chain,
        });
    }
    report.loops = rings.len();
    let outer: Vec<usize> = rings
        .iter()
        .enumerate()
        .filter_map(|(i, r)| (r.loop_.signed_area() > 0.0).then_some(i))
        .collect();
    let mut holes = vec![Vec::new(); outer.len()];
    for (index, ring) in rings
        .iter()
        .enumerate()
        .filter(|(_, r)| r.loop_.signed_area() <= 0.0)
    {
        let point = ring.loop_.segments()[0].to;
        let owner = outer
            .iter()
            .enumerate()
            .filter(|(_, i)| rings[**i].loop_.winding(point) != 0)
            .min_by(|(_, a), (_, b)| {
                rings[**a]
                    .loop_
                    .signed_area()
                    .total_cmp(&rings[**b].loop_.signed_area())
            })
            .map(|(owner, _)| owner);
        let Some(owner) = owner else {
            return Err(RegionError::Output(ProfileError::WrongWinding {
                loop_ref: LoopRef::Outer,
            }));
        };
        holes[owner].push(index);
    }
    let mut rings: Vec<Option<Ring>> = rings.into_iter().map(Some).collect();
    let mut profiles = Vec::with_capacity(outer.len());
    let mut correspondence = Vec::with_capacity(fragments.len());
    for (profile, &o) in outer.iter().enumerate() {
        let loops = core::iter::once((LoopRef::Outer, o)).chain(
            holes[profile]
                .iter()
                .enumerate()
                .map(|(i, &h)| (LoopRef::Hole(i), h)),
        );
        for (loop_ref, index) in loops {
            for (segment, &fragment) in rings[index]
                .as_ref()
                .expect("unconsumed ring")
                .fragments
                .iter()
                .enumerate()
            {
                correspondence.push(RegionSegment {
                    profile,
                    segment: SegmentRef { loop_ref, segment },
                    sources: fragments[fragment].sources,
                });
            }
        }
        let holes = holes[profile]
            .iter()
            .map(|&i| rings[i].take().expect("unconsumed hole").loop_)
            .collect();
        profiles.push(
            Profile::new(
                rings[o].take().expect("unconsumed outer").loop_,
                holes,
                policy,
            )
            .map_err(RegionError::Output)?,
        );
    }
    Ok(RegionResult {
        profiles,
        segments: correspondence,
        report,
    })
}
