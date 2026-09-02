// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scenario classes: seeded case families with distinct stress profiles.
//!
//! Each class deterministically expands a case seed into an expression
//! tree over operands. Classes differ in what they stress:
//!
//! - `convex_mixed`: the original family — boxes and low-resolution prisms
//!   under random rigid placements, left-fold trees.
//! - `curved_wall`: cylindrical prisms up to 96 segments — collinear cut
//!   runs and sliver cascades along curved walls by construction.
//! - `curved_surface`: pairs of faceted spheres at varied resolutions,
//!   overlap depths, and independent tessellation orientations.
//! - `nonconvex`: L/U-shaped prisms (single watertight meshes, referees as
//!   unions of convex boxes) against convex counterparts.
//! - `chained`: balanced trees `(a op b) op (c op d)` — intermediate
//!   pipeline *outputs*, with their cut-curve vertex structure, feed back
//!   in as operands.
//! - `adversarial`: axis-aligned boxes on an exact dyadic lattice placed
//!   in deliberate contact sub-modes: face-flush, shared-edge,
//!   shared-vertex, tiny overlap, near touch, near containment.
//! - `scale`: the convex family at coordinate scale `1e-3` or `1e4`
//!   (comparison bands scale linearly; stresses f32 narrowing).
//! - `empty_total`: disjoint, contained, and identical configurations
//!   whose results are empty, total, a component pair, or an internal
//!   cavity — the result-shape contract edges.

use exedra::boolean::BooleanOp;

use crate::operands::{
    Operand, Rigid, box_operand, random_curved_operand, random_nonconvex_operand, random_operand,
    random_operand_scaled, random_rigid, sphere_operand,
};
use crate::rng::SplitMix64;

/// One scenario family.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum ScenarioClass {
    /// Boxes + low-res prisms, random placements (the original family).
    ConvexMixed,
    /// Cylindrical prisms at 8..96 segments.
    CurvedWall,
    /// Intersections between independently oriented faceted spheres.
    CurvedSurface,
    /// L/U prisms with union-of-boxes referees.
    NonConvex,
    /// Balanced boolean-of-boolean trees.
    Chained,
    /// Deliberate exact/near contact placements on a dyadic lattice.
    Adversarial,
    /// Coordinate scales 1e-3 and 1e4.
    Scale,
    /// Empty/total/contained result-shape contract cases.
    EmptyTotal,
}

impl ScenarioClass {
    /// Every class, in reporting order.
    pub(crate) const ALL: [Self; 8] = [
        Self::ConvexMixed,
        Self::CurvedWall,
        Self::NonConvex,
        Self::Chained,
        Self::Adversarial,
        Self::Scale,
        Self::EmptyTotal,
        // Appended so adding this class does not re-key the deterministic
        // batch seeds of the established scenario families.
        Self::CurvedSurface,
    ];

    /// Stable report/CLI key.
    #[must_use]
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::ConvexMixed => "convex_mixed",
            Self::CurvedWall => "curved_wall",
            Self::CurvedSurface => "curved_surface",
            Self::NonConvex => "nonconvex",
            Self::Chained => "chained",
            Self::Adversarial => "adversarial",
            Self::Scale => "scale",
            Self::EmptyTotal => "empty_total",
        }
    }

    /// Parses a CLI key.
    #[must_use]
    pub(crate) fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.key() == key)
    }
}

/// A boolean expression tree over operand indices.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    /// One operand.
    Leaf(usize),
    /// A boolean combination of two subtrees.
    Op(BooleanOp, Box<Self>, Box<Self>),
}

/// One fully-expanded case.
pub(crate) struct Case {
    /// Operand solids (leaves).
    pub(crate) operands: Vec<Operand>,
    /// The expression tree.
    pub(crate) tree: Node,
    /// Linear scale of the comparison bands and sampling offsets
    /// (1.0 at unit scale).
    pub(crate) band_scale: f64,
    /// Placement sub-mode for adversarial/empty-total classes, "-"
    /// otherwise.
    pub(crate) submode: &'static str,
    /// Human-readable description for findings.
    pub(crate) describe: String,
}

fn random_op(rng: &mut SplitMix64) -> BooleanOp {
    [
        BooleanOp::Union,
        BooleanOp::Intersection,
        BooleanOp::Difference,
    ][rng.index(3)]
}

/// Left-fold tree over `count` leaves with random ops.
fn fold_tree(rng: &mut SplitMix64, count: usize) -> Node {
    let mut node = Node::Leaf(0);
    for leaf in 1..count {
        node = Node::Op(random_op(rng), Box::new(node), Box::new(Node::Leaf(leaf)));
    }
    node
}

/// Expands a class + case seed into a case. Deterministic.
#[must_use]
pub(crate) fn build_case(class: ScenarioClass, case_seed: u64) -> Case {
    let mut rng = SplitMix64::new(case_seed);
    match class {
        ScenarioClass::ConvexMixed => {
            let count = 2 + rng.index(3);
            let operands: Vec<Operand> = (0..count).map(|_| random_operand(&mut rng)).collect();
            let tree = fold_tree(&mut rng, count);
            finish(class, operands, tree, 1.0, "-")
        }
        ScenarioClass::CurvedWall => {
            let count = 2 + rng.index(2);
            let operands: Vec<Operand> = (0..count)
                .map(|index| {
                    if index == 0 || rng.index(3) != 0 {
                        random_curved_operand(&mut rng)
                    } else {
                        random_operand(&mut rng)
                    }
                })
                .collect();
            let tree = fold_tree(&mut rng, count);
            finish(class, operands, tree, 1.0, "-")
        }
        ScenarioClass::CurvedSurface => curved_surface_case(&mut rng, class),
        ScenarioClass::NonConvex => {
            let count = 2 + rng.index(2);
            let operands: Vec<Operand> = (0..count)
                .map(|index| {
                    if index == 0 {
                        random_nonconvex_operand(&mut rng)
                    } else {
                        random_operand(&mut rng)
                    }
                })
                .collect();
            let tree = fold_tree(&mut rng, count);
            finish(class, operands, tree, 1.0, "-")
        }
        ScenarioClass::Chained => {
            let operands: Vec<Operand> = (0..4).map(|_| random_operand(&mut rng)).collect();
            let left = Node::Op(
                random_op(&mut rng),
                Box::new(Node::Leaf(0)),
                Box::new(Node::Leaf(1)),
            );
            let right = Node::Op(
                random_op(&mut rng),
                Box::new(Node::Leaf(2)),
                Box::new(Node::Leaf(3)),
            );
            let tree = Node::Op(random_op(&mut rng), Box::new(left), Box::new(right));
            finish(class, operands, tree, 1.0, "-")
        }
        ScenarioClass::Adversarial => adversarial_case(&mut rng, class),
        ScenarioClass::Scale => {
            let scale = if rng.index(2) == 0 { 1.0e-3 } else { 1.0e4 };
            let count = 2 + rng.index(3);
            let operands: Vec<Operand> = (0..count)
                .map(|_| random_operand_scaled(&mut rng, scale))
                .collect();
            let tree = fold_tree(&mut rng, count);
            let submode = if scale < 1.0 { "milli" } else { "kilo" };
            finish(class, operands, tree, scale, submode)
        }
        ScenarioClass::EmptyTotal => empty_total_case(&mut rng, class),
    }
}

/// Overlapping faceted spheres exercise closed, curved-on-curved seam loops.
/// Independent rotations change the triangulation relationship without
/// changing the analytic field, while fixed overlap bands keep every case a
/// non-empty lens rather than spending the sweep on containment or misses.
fn curved_surface_case(rng: &mut SplitMix64, class: ScenarioClass) -> Case {
    let (lat_segments, lon_segments, submode) = [
        (4, 8, "coarse"),
        (6, 12, "medium"),
        (8, 16, "standard"),
        (12, 24, "dense"),
    ][rng.index(4)];
    let offset = [0.6, 0.7, 0.8, 0.9][rng.index(4)];
    let mut left = random_rigid(rng);
    let mut right = random_rigid(rng);
    left.t = [-offset, 0.0, 0.0];
    right.t = [offset, 0.0, 0.0];
    let mut operands = vec![
        sphere_operand(1.0, lat_segments, lon_segments, &left),
        sphere_operand(1.0, lat_segments, lon_segments, &right),
    ];
    if rng.index(2) == 1 {
        operands.reverse();
    }
    let tree = Node::Op(
        BooleanOp::Intersection,
        Box::new(Node::Leaf(0)),
        Box::new(Node::Leaf(1)),
    );
    finish(class, operands, tree, 1.0, submode)
}

/// Dyadic lattice value: `k / 64` with `k` in `[lo64, hi64)` — exactly
/// representable in f32, so contact arithmetic stays exact.
fn dyadic(rng: &mut SplitMix64, lo64: i64, hi64: i64) -> f64 {
    let span = usize::try_from(hi64 - lo64).expect("positive span");
    #[expect(clippy::cast_precision_loss, reason = "small dyadic integers")]
    let k = (lo64 + i64::try_from(rng.index(span)).expect("small index")) as f64;
    k / 64.0
}

fn axis_aligned(t: [f64; 3]) -> Rigid {
    Rigid {
        cols: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t,
    }
}

/// Deliberate-contact case: two axis-aligned boxes on the 1/64 lattice.
fn adversarial_case(rng: &mut SplitMix64, class: ScenarioClass) -> Case {
    let size_a = [
        dyadic(rng, 32, 116),
        dyadic(rng, 32, 116),
        dyadic(rng, 32, 116),
    ];
    let size_b = [
        dyadic(rng, 32, 116),
        dyadic(rng, 32, 116),
        dyadic(rng, 32, 116),
    ];
    let t_a = [
        dyadic(rng, -32, 33),
        dyadic(rng, -32, 33),
        dyadic(rng, -32, 33),
    ];
    // Exact half-sums: sizes are k/64, so (sa + sb) / 2 is k'/128 — still
    // exact in f32 at these magnitudes.
    let flush = |axis: usize| t_a[axis] + (size_a[axis] + size_b[axis]) * 0.5;
    const NUDGE: f64 = 1.0 / 1_048_576.0;
    let submodes: [&'static str; 6] = [
        "face_flush",
        "shared_edge",
        "shared_vertex",
        "tiny_overlap",
        "near_touch",
        "containment_near",
    ];
    let submode = submodes[rng.index(6)];
    let (size_b, t_b) = match submode {
        "face_flush" => (size_b, [flush(0), t_a[1], t_a[2]]),
        "shared_edge" => (size_b, [flush(0), flush(1), t_a[2]]),
        "shared_vertex" => (size_b, [flush(0), flush(1), flush(2)]),
        "tiny_overlap" => (size_b, [flush(0) - NUDGE, t_a[1], t_a[2]]),
        "near_touch" => (size_b, [flush(0) + NUDGE, t_a[1], t_a[2]]),
        _ => (
            // Strictly contained copy: 63/64 of the host, same center.
            [
                size_a[0] * (63.0 / 64.0),
                size_a[1] * (63.0 / 64.0),
                size_a[2] * (63.0 / 64.0),
            ],
            t_a,
        ),
    };
    let operands = vec![
        box_operand(size_a, &axis_aligned(t_a)),
        box_operand(size_b, &axis_aligned(t_b)),
    ];
    let tree = Node::Op(
        random_op(rng),
        Box::new(Node::Leaf(0)),
        Box::new(Node::Leaf(1)),
    );
    finish(class, operands, tree, 1.0, submode)
}

/// Result-shape contract case: disjoint / contained / identical pairs.
fn empty_total_case(rng: &mut SplitMix64, class: ScenarioClass) -> Case {
    let size_a = [
        dyadic(rng, 48, 128),
        dyadic(rng, 48, 128),
        dyadic(rng, 48, 128),
    ];
    let t_a = [
        dyadic(rng, -16, 17),
        dyadic(rng, -16, 17),
        dyadic(rng, -16, 17),
    ];
    let submodes: [(&'static str, BooleanOp); 8] = [
        ("disjoint_difference", BooleanOp::Difference),
        ("disjoint_intersection", BooleanOp::Intersection),
        ("disjoint_union", BooleanOp::Union),
        ("contained_intersection", BooleanOp::Intersection),
        ("contained_difference", BooleanOp::Difference),
        ("contained_union", BooleanOp::Union),
        ("identical_difference", BooleanOp::Difference),
        ("identical_intersection", BooleanOp::Intersection),
    ];
    let (submode, op) = submodes[rng.index(8)];
    let (size_b, t_b) = if submode.starts_with("disjoint") {
        // Separated by at least 0.5 along x.
        let size_b = [
            dyadic(rng, 48, 128),
            dyadic(rng, 48, 128),
            dyadic(rng, 48, 128),
        ];
        let gap = dyadic(rng, 32, 96);
        (
            size_b,
            [t_a[0] + (size_a[0] + size_b[0]) * 0.5 + gap, t_a[1], t_a[2]],
        )
    } else if submode.starts_with("contained") {
        // Strictly inside with generous margin.
        ([size_a[0] * 0.5, size_a[1] * 0.5, size_a[2] * 0.5], t_a)
    } else {
        (size_a, t_a)
    };
    let operands = vec![
        box_operand(size_a, &axis_aligned(t_a)),
        box_operand(size_b, &axis_aligned(t_b)),
    ];
    let tree = Node::Op(op, Box::new(Node::Leaf(0)), Box::new(Node::Leaf(1)));
    finish(class, operands, tree, 1.0, submode)
}

fn finish(
    class: ScenarioClass,
    operands: Vec<Operand>,
    tree: Node,
    band_scale: f64,
    submode: &'static str,
) -> Case {
    let describe = describe_tree(class, submode, &operands, &tree);
    Case {
        operands,
        tree,
        band_scale,
        submode,
        describe,
    }
}

fn describe_tree(
    class: ScenarioClass,
    submode: &'static str,
    operands: &[Operand],
    tree: &Node,
) -> String {
    fn render(operands: &[Operand], node: &Node, out: &mut String) {
        match node {
            Node::Leaf(index) => {
                out.push('[');
                out.push_str(&operands[*index].describe);
                out.push(']');
            }
            Node::Op(op, left, right) => {
                let symbol = match op {
                    BooleanOp::Union => "+",
                    BooleanOp::Intersection => "&",
                    BooleanOp::Difference => "-",
                };
                out.push('(');
                render(operands, left, out);
                out.push_str(&format!(" {symbol} "));
                render(operands, right, out);
                out.push(')');
            }
        }
    }
    let mut text = format!("{}/{submode}: ", class.key());
    render(operands, tree, &mut text);
    text
}
