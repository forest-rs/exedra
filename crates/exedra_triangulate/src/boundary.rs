// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::spatial::{Bounds, Index};
use crate::{PolygonInput, TriError};
use alloc::vec::Vec;
use exedra_math::predicates::{Orientation, orient2d};

/// One original cyclic polygon edge, before pruning or hole bridging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundaryEdge {
    /// `None` for the outer boundary, or the original hole index.
    pub hole: Option<usize>,
    /// Edge from point `edge` to point `(edge + 1) % boundary.len()`.
    pub edge: usize,
}

impl core::fmt::Display for BoundaryEdge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.hole {
            None => write!(f, "outer edge {}", self.edge),
            Some(hole) => write!(f, "hole {hole} edge {}", self.edge),
        }
    }
}

/// Exact relationship between two offending input edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryContactKind {
    /// The open interiors cross with neither segment collinear with the other.
    Crossing,
    /// An endpoint touches the other segment, or collinear segments overlap.
    Touching,
}

struct InputEdge {
    ring: usize,
    original: usize,
    rank: usize,
    count: usize,
}

/// Validates original boundaries before any cover is accepted. Box queries
/// avoid testing disjoint edge pairs; exact predicates decide each survivor.
pub(crate) fn validate(input: &PolygonInput<'_>) -> Result<(), TriError> {
    let rings: Vec<_> = core::iter::once(input.outer)
        .chain(input.holes.iter().copied())
        .collect();
    let mut edges = Vec::with_capacity(input.vertex_count());
    let mut bounds = Vec::with_capacity(input.vertex_count());
    for (ring, points) in rings.iter().enumerate() {
        let start = edges.len();
        for original in 0..points.len() {
            let a = points[original];
            let b = points[(original + 1) % points.len()];
            if a == b {
                continue;
            }
            edges.push(InputEdge {
                ring,
                original,
                rank: 0,
                count: 0,
            });
            bounds.push(Bounds::segment(a, b));
        }
        let count = edges.len() - start;
        if count < 3 {
            return Err(TriError::NonSimple);
        }
        for (rank, edge) in edges[start..].iter_mut().enumerate() {
            edge.rank = rank;
            edge.count = count;
        }
    }
    let index = Index::new(&bounds);
    let mut first_contact = None;
    for (first, a) in edges.iter().enumerate() {
        let query = bounds[first];
        index.any_matching(
            |bounds| bounds.overlaps(query),
            |second| {
                if second <= first {
                    return false;
                }
                let b = &edges[second];
                let first_points = rings[a.ring];
                let second_points = rings[b.ring];
                let a0 = first_points[a.original];
                let a1 = first_points[(a.original + 1) % first_points.len()];
                let b0 = second_points[b.original];
                let b1 = second_points[(b.original + 1) % second_points.len()];
                let kind = if a.ring == b.ring && a.rank + 1 == b.rank {
                    adjacent_overlap(a0, a1, b1)
                } else if a.ring == b.ring && a.rank == 0 && b.rank + 1 == b.count {
                    adjacent_overlap(b0, b1, a1)
                } else {
                    contact(a0, a1, b0, b1)
                };
                if let Some(kind) = kind {
                    // Match the original loop-pair/edge-pair diagnosis order;
                    // tree shape and traversal order do not choose the witness.
                    let key = (a.ring, b.ring, a.original, b.original);
                    if first_contact.is_none_or(|(previous, _)| key < previous) {
                        first_contact = Some((key, kind));
                    }
                }
                false
            },
        );
    }
    if let Some(((first_ring, second_ring, a, b), kind)) = first_contact {
        return Err(TriError::BoundaryContact {
            first: BoundaryEdge {
                hole: first_ring.checked_sub(1),
                edge: a,
            },
            second: BoundaryEdge {
                hole: second_ring.checked_sub(1),
                edge: b,
            },
            kind,
        });
    }
    Ok(())
}

fn adjacent_overlap(a: [f64; 2], shared: [f64; 2], b: [f64; 2]) -> Option<BoundaryContactKind> {
    (orient2d(a, shared, b) == Orientation::Collinear && !between(a, b, shared))
        .then_some(BoundaryContactKind::Touching)
}

// Full-scan reference: inspect original edges, never synthesized bridges.
// At most N*(N-1)/2 edge pairs, with an allocation-free bounding-box rejection.
// Adjacent same-loop edges are omitted because their shared endpoint is normal.
#[cfg(test)]
pub(crate) fn diagnose(input: &PolygonInput<'_>) -> Option<TriError> {
    let rings = || core::iter::once(input.outer).chain(input.holes.iter().copied());
    for (first_loop, first) in rings().enumerate() {
        for (second_loop, second) in rings().enumerate().skip(first_loop) {
            for a in 0..first.len() {
                let begin = if first_loop == second_loop { a + 1 } else { 0 };
                for b in begin..second.len() {
                    if first_loop == second_loop && (b == a + 1 || (a == 0 && b + 1 == first.len()))
                    {
                        continue;
                    }
                    if let Some(kind) = contact(
                        first[a],
                        first[(a + 1) % first.len()],
                        second[b],
                        second[(b + 1) % second.len()],
                    ) {
                        return Some(TriError::BoundaryContact {
                            first: BoundaryEdge {
                                hole: first_loop.checked_sub(1),
                                edge: a,
                            },
                            second: BoundaryEdge {
                                hole: second_loop.checked_sub(1),
                                edge: b,
                            },
                            kind,
                        });
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn contact(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
) -> Option<BoundaryContactKind> {
    for axis in 0..2 {
        if a[axis].max(b[axis]) < c[axis].min(d[axis])
            || c[axis].max(d[axis]) < a[axis].min(b[axis])
        {
            return None;
        }
    }
    let ab_c = orient2d(a, b, c);
    let ab_d = orient2d(a, b, d);
    let cd_a = orient2d(c, d, a);
    let cd_b = orient2d(c, d, b);
    if ab_c != ab_d
        && cd_a != cd_b
        && [ab_c, ab_d, cd_a, cd_b]
            .iter()
            .all(|o| *o != Orientation::Collinear)
    {
        return Some(BoundaryContactKind::Crossing);
    }
    ((ab_c == Orientation::Collinear && between(a, b, c))
        || (ab_d == Orientation::Collinear && between(a, b, d))
        || (cd_a == Orientation::Collinear && between(c, d, a))
        || (cd_b == Orientation::Collinear && between(c, d, b)))
    .then_some(BoundaryContactKind::Touching)
}

pub(crate) fn between(a: [f64; 2], b: [f64; 2], q: [f64; 2]) -> bool {
    (a[0].min(b[0]) <= q[0] && q[0] <= a[0].max(b[0]))
        && (a[1].min(b[1]) <= q[1] && q[1] <= a[1].max(b[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TriParams, triangulate};

    #[test]
    fn crossed_input_is_refused_even_when_ear_clipping_can_build_a_cover() {
        let outer = [
            [0.0, 6.0],
            [-1.0, 2.0],
            [8.0, 6.0],
            [2.0, 8.0],
            [0.0, 0.0],
            [6.0, 0.0],
            [8.0, 2.0],
            [6.0, 8.0],
        ];
        let hole = [[3.75, 3.75], [3.75, 4.25], [4.25, 4.25], [4.25, 3.75]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[&hole],
        };
        let expected = TriError::BoundaryContact {
            first: BoundaryEdge {
                hole: None,
                edge: 1,
            },
            second: BoundaryEdge {
                hole: None,
                edge: 3,
            },
            kind: BoundaryContactKind::Crossing,
        };
        for params in [TriParams::ear_clip(), TriParams::constrained_delaunay()] {
            assert_eq!(triangulate(&input, &params), Err(expected));
            assert_eq!(
                crate::triangulate_preserving_boundary(&input, &params),
                Err(expected)
            );
        }
        assert_eq!(
            crate::refine(&input, &crate::RefineParams::default()),
            Err(expected)
        );
    }

    #[test]
    fn indexed_contact_witness_matches_the_full_original_edge_oracle() {
        let original = [
            [0.0, 0.0],
            [6.0, 0.0],
            [8.0, 2.0],
            [8.0, 6.0],
            [6.0, 8.0],
            [2.0, 8.0],
            [0.0, 6.0],
            [-1.0, 2.0],
        ];
        let hole = [[3.75, 3.75], [3.75, 4.25], [4.25, 4.25], [4.25, 3.75]];
        let mut random = 0x51c2_81db_145e_6aa7_u64;
        let mut contacts = 0;
        for _ in 0..512 {
            let mut outer = original;
            for i in 0..outer.len() {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                outer.swap(i, usize::try_from(random % 8).unwrap());
            }
            let input = PolygonInput {
                outer: &outer,
                holes: &[&hole],
            };
            let expected = diagnose(&input);
            assert_eq!(validate(&input).err(), expected, "{outer:?}");
            if expected.is_some() {
                contacts += 1;
                for params in [TriParams::ear_clip(), TriParams::constrained_delaunay()] {
                    assert!(triangulate(&input, &params).is_err());
                }
            }
        }
        assert!(contacts > 400);
    }

    #[test]
    fn coincident_runs_keep_effective_adjacency_but_backtracking_is_refused() {
        let valid = [
            [-0.0, 0.0],
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 0.0],
            [2.0, 2.0],
            [0.0, 2.0],
            [-0.0, 0.0],
        ];
        let input = PolygonInput {
            outer: &valid,
            holes: &[],
        };
        assert_eq!(validate(&input), Ok(()));
        for params in [TriParams::ear_clip(), TriParams::constrained_delaunay()] {
            assert!(triangulate(&input, &params).is_ok());
        }
        let backtracking = [
            [0.0, 0.0],
            [3.0, 0.0],
            [2.0, 0.0],
            [3.0, 1.0],
            [3.0, 3.0],
            [0.0, 3.0],
        ];
        assert_eq!(
            validate(&PolygonInput {
                outer: &backtracking,
                holes: &[]
            }),
            Err(TriError::BoundaryContact {
                first: BoundaryEdge {
                    hole: None,
                    edge: 0
                },
                second: BoundaryEdge {
                    hole: None,
                    edge: 1
                },
                kind: BoundaryContactKind::Touching,
            })
        );
    }

    #[test]
    fn ground_reaching_hole_names_original_boundary_edges() {
        // First outer edge contains a redundant sample: diagnosis must not
        // use the pruned/bridged numbering. Hole edge zero touches outer edge one.
        let outer = [[0.0, 0.0], [1.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let hole = [[2.0, 0.0], [2.0, 2.0], [3.0, 2.0], [3.0, 0.0]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[&hole],
        };
        let expected = TriError::BoundaryContact {
            first: BoundaryEdge {
                hole: None,
                edge: 1,
            },
            second: BoundaryEdge {
                hole: Some(0),
                edge: 0,
            },
            kind: BoundaryContactKind::Touching,
        };
        assert_eq!(triangulate(&input, &TriParams::default()), Err(expected));
    }

    #[test]
    fn crossing_holes_name_both_loops_and_disjoint_escape_keeps_its_error() {
        let outer = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let a = [[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]];
        let b = [[2.0, 0.5], [2.0, 2.0], [3.5, 2.0], [3.5, 0.5]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[&a, &b],
        };
        let error = triangulate(&input, &TriParams::default()).unwrap_err();
        assert!(matches!(
            error,
            TriError::BoundaryContact {
                first: BoundaryEdge { hole: Some(0), .. },
                second: BoundaryEdge { hole: Some(1), .. },
                kind: BoundaryContactKind::Crossing,
            }
        ));
        let outside = a.map(|p| [p[0] + 5.0, p[1]]);
        let input = PolygonInput {
            outer: &outer,
            holes: &[&outside],
        };
        assert_eq!(
            triangulate(&input, &TriParams::default()),
            Err(TriError::HoleOutsideOuter { hole: 0 })
        );
    }

    #[test]
    fn exact_contact_distinguishes_crossing_touching_and_disjoint() {
        assert_eq!(
            contact([0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]),
            Some(BoundaryContactKind::Crossing)
        );
        assert_eq!(
            contact([0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [3.0, 0.0]),
            Some(BoundaryContactKind::Touching)
        );
        assert_eq!(
            contact([0.0, 0.0], [2.0, 0.0], [1.0, 1e-300], [3.0, 1e-300]),
            None
        );
        assert_eq!(
            contact([0.0, 0.0], [2.0, 0.0], [3.0, 0.0], [4.0, 0.0]),
            None
        );
    }
}
