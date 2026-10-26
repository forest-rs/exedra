// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Immutable source-boundary geometry plus the bridges added while splicing.

use alloc::vec::Vec;
use core::cmp::Ordering;
use core::ops::Range;

use super::blocks_bridge;
use crate::predicates::{Orientation, orient2d};

use crate::spatial::{Bounds, Index};

/// Original edges never disappear during hole splicing. The only new edges
/// are the two copies of each bridge, which have identical blocking geometry.
pub(super) struct BoundaryIndex<'a> {
    points: &'a [[f64; 2]],
    edges: Vec<[u32; 2]>,
    boundaries: Vec<Range<usize>>,
    spatial: Index,
    /// Sorted by endpoint coordinate; each original edge contributes twice.
    incidents: Vec<[u32; 2]>,
    bridges: Vec<[u32; 2]>,
}

fn compare_points(a: [f64; 2], b: [f64; 2]) -> Ordering {
    // Numerical comparison keeps -0 and +0 in the same coordinate group.
    a[0].partial_cmp(&b[0])
        .expect("finite input")
        .then_with(|| a[1].partial_cmp(&b[1]).expect("finite input"))
}

impl<'a> BoundaryIndex<'a> {
    pub(super) fn new(points: &'a [[f64; 2]], outer: &[u32], holes: &[Vec<u32>]) -> Self {
        let count = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
        let mut edges = Vec::with_capacity(count);
        let mut incidents = Vec::with_capacity(count * 2);
        let mut boundaries = Vec::with_capacity(holes.len() + 1);
        let mut source_bounds = Vec::with_capacity(holes.len() + 1);
        for ring in core::iter::once(outer).chain(holes.iter().map(Vec::as_slice)) {
            let start = edges.len();
            let mut min = [f64::INFINITY; 2];
            let mut max = [f64::NEG_INFINITY; 2];
            for (i, &a) in ring.iter().enumerate() {
                let b = ring[(i + 1) % ring.len()];
                edges.push([a, b]);
                incidents.extend([[a, b], [b, a]]);
                let p = points[a as usize];
                for axis in 0..2 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
            boundaries.push(start..edges.len());
            source_bounds.push(Bounds { min, max });
        }
        incidents.sort_unstable_by(|a, b| {
            compare_points(points[a[0] as usize], points[b[0] as usize]).then_with(|| a.cmp(b))
        });
        let spatial = Index::new(&source_bounds);
        Self {
            points,
            edges,
            boundaries,
            spatial,
            incidents,
            bridges: Vec::with_capacity(holes.len()),
        }
    }

    pub(super) fn add_bridge(&mut self, a: u32, b: u32) {
        self.bridges.push([a, b]);
    }

    pub(super) fn collinear_incident(&self, from: [f64; 2], to: [f64; 2]) -> bool {
        let start = self.incidents.partition_point(|edge| {
            compare_points(self.points[edge[0] as usize], to) == Ordering::Less
        });
        for &[point, neighbor] in &self.incidents[start..] {
            if self.points[point as usize] != to {
                break;
            }
            if orient2d(from, to, self.points[neighbor as usize]) == Orientation::Collinear {
                return true;
            }
        }
        self.bridges.iter().any(|&[a, b]| {
            let a = self.points[a as usize];
            let b = self.points[b as usize];
            (a == to && orient2d(from, to, b) == Orientation::Collinear)
                || (b == to && orient2d(from, to, a) == Orientation::Collinear)
        })
    }

    pub(super) fn blocks(&self, from: [f64; 2], to: [f64; 2]) -> bool {
        if self.spatial.any_matching(
            |bounds| bounds.may_intersect_segment(from, to),
            |ring| {
                self.edges[self.boundaries[ring].clone()]
                    .iter()
                    .any(|&[a, b]| {
                        blocks_bridge(from, to, self.points[a as usize], self.points[b as usize])
                    })
            },
        ) {
            return true;
        }
        self.bridges.iter().any(|&[a, b]| {
            blocks_bridge(from, to, self.points[a as usize], self.points[b as usize])
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn indexed_queries_match_full_edge_scans_including_bridges_and_coordinate_aliases() {
        let original = [
            [0.0, 0.0],
            [8.0, 0.0],
            [8.0, 8.0],
            [0.0, 8.0],
            [2.0, 2.0],
            [2.0, 3.0],
            [3.0, 3.0],
            [3.0, 2.0],
            [5.0, 5.0],
            [5.0, 6.0],
            [6.0, 6.0],
            [6.0, 5.0],
            [-0.0, 0.0],
            [1.0, 1.0],
            [1.0, 2.0],
        ];
        // The last ring intentionally touches the first through a different
        // point label and signed zero. Query equivalence must hold even on
        // an arrangement the caller will subsequently diagnose as invalid.
        let outer = [0, 1, 2, 3];
        let holes = vec![vec![4, 5, 6, 7], vec![8, 9, 10, 11], vec![12, 13, 14]];
        for scale in [1e-300, 1e-90, 1.0, 1e90] {
            let points: Vec<_> = original
                .iter()
                .map(|p| [p[0] * scale, p[1] * scale])
                .collect();
            let mut index = BoundaryIndex::new(&points, &outer, &holes);
            let mut all_edges = Vec::new();
            for ring in core::iter::once(outer.as_slice()).chain(holes.iter().map(Vec::as_slice)) {
                for (i, &a) in ring.iter().enumerate() {
                    all_edges.push([a, ring[(i + 1) % ring.len()]]);
                }
            }
            for bridge in [None, Some([1, 7]), Some([7, 11])] {
                if let Some([a, b]) = bridge {
                    index.add_bridge(a, b);
                    all_edges.push([a, b]);
                }
                let coordinates = [-1.0, 0.0, 2.0, 4.0, 6.0, 8.0, 9.0];
                let queries: Vec<_> = coordinates
                    .iter()
                    .flat_map(|&x| coordinates.iter().map(move |&y| [x * scale, y * scale]))
                    .collect();
                for &from in &queries {
                    for &to in &queries {
                        let expected_block = all_edges.iter().any(|&[a, b]| {
                            blocks_bridge(from, to, points[a as usize], points[b as usize])
                        });
                        assert_eq!(index.blocks(from, to), expected_block, "{from:?} -> {to:?}");
                        let expected_collinear = all_edges.iter().any(|&[a, b]| {
                            let a = points[a as usize];
                            let b = points[b as usize];
                            (a == to && orient2d(from, to, b) == Orientation::Collinear)
                                || (b == to && orient2d(from, to, a) == Orientation::Collinear)
                        });
                        assert_eq!(index.collinear_incident(from, to), expected_collinear);
                    }
                }
            }
        }
    }
}
