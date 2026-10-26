// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Flat, immutable bounding hierarchy for geometry broad phases.

use alloc::vec::Vec;

#[derive(Copy, Clone, Debug)]
pub(crate) struct Bounds {
    pub(crate) min: [f64; 2],
    pub(crate) max: [f64; 2],
}

impl Bounds {
    pub(crate) fn segment(a: [f64; 2], b: [f64; 2]) -> Self {
        Self {
            min: [a[0].min(b[0]), a[1].min(b[1])],
            max: [a[0].max(b[0]), a[1].max(b[1])],
        }
    }

    pub(crate) fn overlaps(self, other: Self) -> bool {
        (0..2).all(|axis| self.min[axis] <= other.max[axis] && other.min[axis] <= self.max[axis])
    }
}

struct Node {
    bounds: Bounds,
    /// First node after this contiguous subtree.
    end: usize,
    item: Option<usize>,
}

/// Median splits bound construction recursion; preorder skip links make each
/// query an allocation-free walk over one contiguous array.
pub(crate) struct Index {
    nodes: Vec<Node>,
}

impl Index {
    pub(crate) fn new(bounds: &[Bounds]) -> Self {
        let mut nodes = Vec::with_capacity(bounds.len().saturating_mul(2).saturating_sub(1));
        let mut order: Vec<_> = (0..bounds.len()).collect();
        if !order.is_empty() {
            build(&mut order, bounds, &mut nodes);
        }
        Self { nodes }
    }

    /// Returns early when a leaf visitor succeeds. All decisions made from
    /// these boxes are conservative; consumers own exact geometry tests.
    pub(crate) fn any_matching(
        &self,
        accept: impl Fn(Bounds) -> bool,
        mut visit: impl FnMut(usize) -> bool,
    ) -> bool {
        let mut at = 0;
        while let Some(node) = self.nodes.get(at) {
            if !accept(node.bounds) {
                at = node.end;
                continue;
            }
            if let Some(item) = node.item
                && visit(item)
            {
                return true;
            }
            at += 1;
        }
        false
    }
}

fn build(order: &mut [usize], source: &[Bounds], nodes: &mut Vec<Node>) {
    let mut bounds = source[order[0]];
    for &index in &order[1..] {
        let other = source[index];
        for axis in 0..2 {
            bounds.min[axis] = bounds.min[axis].min(other.min[axis]);
            bounds.max[axis] = bounds.max[axis].max(other.max[axis]);
        }
    }
    let at = nodes.len();
    nodes.push(Node {
        bounds,
        end: at + 1,
        item: None,
    });
    if let [index] = order {
        nodes[at].item = Some(*index);
        return;
    }
    let axis = usize::from(bounds.max[1] - bounds.min[1] > bounds.max[0] - bounds.min[0]);
    let middle = order.len() / 2;
    order.select_nth_unstable_by(middle, |&a, &b| {
        (source[a].min[axis] + source[a].max[axis])
            .total_cmp(&(source[b].min[axis] + source[b].max[axis]))
            .then(a.cmp(&b))
    });
    let (left, right) = order.split_at_mut(middle);
    build(left, source, nodes);
    build(right, source, nodes);
    nodes[at].end = nodes.len();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_box_queries_match_direct_scans_and_keep_touches() {
        let boxes: Vec<_> = (0..64)
            .map(|i| {
                let x = f64::from(i % 8);
                let y = f64::from(i / 8);
                Bounds::segment([x, y], [x + 0.5, y + 0.75])
            })
            .collect();
        let index = Index::new(&boxes);
        for x in -1..18 {
            for y in -1..18 {
                let p = [f64::from(x) * 0.5, f64::from(y) * 0.5];
                let query = Bounds::segment(p, [p[0] + 1.0, p[1] + 0.25]);
                let expected: Vec<_> = boxes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, bounds)| bounds.overlaps(query).then_some(i))
                    .collect();
                let mut found = Vec::new();
                assert!(!index.any_matching(
                    |bounds| bounds.overlaps(query),
                    |i| {
                        found.push(i);
                        false
                    }
                ));
                found.sort_unstable();
                assert_eq!(found, expected);
            }
        }
        assert!(!Index::new(&[]).any_matching(|_| true, |_| true));
    }
}
