// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Private integer-grid ownership for adaptive dual-contour transitions.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use exedra_spatial::{Aabb, CellId, CellRef, Octree, OctreeVisitor};
use hashbrown::HashMap;

use crate::ScalarField;

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct CornerKey {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) z: u32,
}

impl CornerKey {
    pub(super) const fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }

    pub(super) const fn as_tuple(self) -> (u32, u32, u32) {
        (self.x, self.y, self.z)
    }
}

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct CellKey {
    pub(super) origin: CornerKey,
    pub(super) span: u32,
    pub(super) depth: u8,
}

#[derive(Copy, Clone, Debug)]
struct LocatedCell {
    key: CellKey,
    bounds: Aabb,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
struct FacePatchKey {
    axis: u8,
    plane: u32,
    u: u32,
    v: u32,
    span: u32,
}

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct EdgeLineKey {
    axis: u8,
    fixed_u: u32,
    fixed_v: u32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct EdgeInterval {
    line: EdgeLineKey,
    start: u32,
    end: u32,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct EdgeSegmentKey {
    pub(super) axis: u8,
    pub(super) start: CornerKey,
    pub(super) length: u32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum ComponentRoute {
    LocalEdge(u8),
    OnlyComponent,
}

pub(super) struct LeafLocator {
    resolution: u32,
    leaves: HashMap<CellKey, CellId>,
    keys: HashMap<CellId, CellKey>,
}

pub(super) type LeafSet = Vec<(CellId, CellKey)>;

pub(super) struct AdaptiveGrid<'a, F> {
    field: &'a F,
    root_bounds: Aabb,
    resolution: u32,
    samples: HashMap<CornerKey, f32>,
    locations: Vec<Option<LocatedCell>>,
}

impl<'a, F: ScalarField> AdaptiveGrid<'a, F> {
    pub(super) fn new(field: &'a F, root_bounds: Aabb, resolution: u32) -> Self {
        Self {
            field,
            root_bounds,
            resolution,
            samples: HashMap::new(),
            locations: Vec::new(),
        }
    }

    pub(super) const fn resolution(&self) -> u32 {
        self.resolution
    }

    #[cfg(test)]
    pub(super) fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub(super) fn cell_key(&self, id: CellId) -> CellKey {
        self.locations
            .get(id.index() as usize)
            .and_then(|location| *location)
            .expect("visited octree cells must have integer locations")
            .key
    }

    pub(super) fn locate_cell(&mut self, cell: CellRef) -> CellKey {
        let index = cell.id.index() as usize;
        if let Some(key) = self
            .locations
            .get(index)
            .and_then(|location| location.as_ref())
            .map(|location| location.key)
        {
            return key;
        }

        let key = match cell.parent {
            None => CellKey {
                origin: CornerKey::new(
                    coordinate_axis(self.root_bounds.min[0], cell.bounds.min[0], self.step()[0]),
                    coordinate_axis(self.root_bounds.min[1], cell.bounds.min[1], self.step()[1]),
                    coordinate_axis(self.root_bounds.min[2], cell.bounds.min[2], self.step()[2]),
                ),
                span: self.resolution >> cell.depth,
                depth: cell.depth,
            },
            Some(parent_id) => {
                let parent = self
                    .locations
                    .get(parent_id.index() as usize)
                    .and_then(|location| *location)
                    .expect("octree parent must be located before its child");
                let half = parent.key.span / 2;
                let center = parent.bounds.center();
                CellKey {
                    origin: CornerKey::new(
                        parent.key.origin.x + u32::from(cell.bounds.min[0] == center[0]) * half,
                        parent.key.origin.y + u32::from(cell.bounds.min[1] == center[1]) * half,
                        parent.key.origin.z + u32::from(cell.bounds.min[2] == center[2]) * half,
                    ),
                    span: half,
                    depth: parent.key.depth + 1,
                }
            }
        };
        debug_assert_eq!(
            key.depth, cell.depth,
            "integer-grid depth must match octree depth"
        );
        if self.locations.len() <= index {
            self.locations.resize(index + 1, None);
        }
        self.locations[index] = Some(LocatedCell {
            key,
            bounds: cell.bounds,
        });
        key
    }

    pub(super) fn sample_cell_corners(&mut self, cell: CellKey) -> [f32; 8] {
        let keys = cell_corner_keys(cell);
        self.sample_keys(&keys);
        keys.map(|key| {
            *self
                .samples
                .get(&key)
                .expect("every requested corner must be cached")
        })
    }

    pub(super) fn sample_keys(&mut self, keys: &[CornerKey]) {
        let mut missing = keys
            .iter()
            .copied()
            .filter(|key| !self.samples.contains_key(key))
            .collect::<Vec<_>>();
        missing.sort_unstable();
        missing.dedup();
        if missing.is_empty() {
            return;
        }
        let points = missing
            .iter()
            .map(|key| self.point(*key))
            .collect::<Vec<_>>();
        let mut values = vec![0.0_f32; points.len()];
        self.field.eval_points(&points, &mut values);
        for (key, value) in missing.into_iter().zip(values) {
            self.samples.insert(key, value);
        }
    }

    pub(super) fn value(&self, key: CornerKey) -> Option<f32> {
        self.samples.get(&key).copied()
    }

    pub(super) fn point(&self, key: CornerKey) -> [f32; 3] {
        let step = self.step();
        [
            axis_point(
                self.root_bounds.min[0],
                self.root_bounds.max[0],
                step[0],
                key.x,
                self.resolution,
            ),
            axis_point(
                self.root_bounds.min[1],
                self.root_bounds.max[1],
                step[1],
                key.y,
                self.resolution,
            ),
            axis_point(
                self.root_bounds.min[2],
                self.root_bounds.max[2],
                step[2],
                key.z,
                self.resolution,
            ),
        ]
    }

    pub(super) fn cell_bounds(&self, cell: CellKey) -> Aabb {
        let min = self.point(cell.origin);
        let max = self.point(CornerKey::new(
            cell.origin.x + cell.span,
            cell.origin.y + cell.span,
            cell.origin.z + cell.span,
        ));
        Aabb::new(min, max).expect("integer-grid cells have ordered finite bounds")
    }

    fn step(&self) -> [f32; 3] {
        let extent = self.root_bounds.extent();
        [
            extent[0] / self.resolution as f32,
            extent[1] / self.resolution as f32,
            extent[2] / self.resolution as f32,
        ]
    }
}

impl LeafLocator {
    pub(super) fn new(leaves: &[(CellId, CellKey)], resolution: u32) -> Self {
        let mut by_key = HashMap::with_capacity(leaves.len());
        let mut keys = HashMap::with_capacity(leaves.len());
        for &(id, key) in leaves {
            by_key.insert(key, id);
            keys.insert(id, key);
        }
        Self {
            resolution,
            leaves: by_key,
            keys,
        }
    }

    pub(super) fn key(&self, id: CellId) -> CellKey {
        self.keys[&id]
    }

    pub(super) fn incident_leaves(
        &self,
        segment: EdgeSegmentKey,
    ) -> Result<Option<[CellId; 4]>, ()> {
        let (x, y, z) = segment.start.as_tuple();
        let coords = match segment.axis {
            0 if y > 0 && z > 0 && y < self.resolution && z < self.resolution => {
                [(x, y - 1, z - 1), (x, y, z - 1), (x, y, z), (x, y - 1, z)]
            }
            1 if x > 0 && z > 0 && x < self.resolution && z < self.resolution => {
                [(x - 1, y, z - 1), (x - 1, y, z), (x, y, z), (x, y, z - 1)]
            }
            2 if x > 0 && y > 0 && x < self.resolution && y < self.resolution => {
                [(x - 1, y - 1, z), (x, y - 1, z), (x, y, z), (x - 1, y, z)]
            }
            _ => return Ok(None),
        };
        let [a, b, c, d] = coords;
        Ok(Some([
            self.find_covering(CornerKey::new(a.0, a.1, a.2))
                .ok_or(())?,
            self.find_covering(CornerKey::new(b.0, b.1, b.2))
                .ok_or(())?,
            self.find_covering(CornerKey::new(c.0, c.1, c.2))
                .ok_or(())?,
            self.find_covering(CornerKey::new(d.0, d.1, d.2))
                .ok_or(())?,
        ]))
    }

    pub(super) fn component_route(&self, leaf: CellId, segment: EdgeSegmentKey) -> ComponentRoute {
        let cell = self.key(leaf);
        let fixed = segment.start;
        let route = match segment.axis {
            0 => local_edge_index(
                0,
                boundary_bit(fixed.y, cell.origin.y, cell.span),
                boundary_bit(fixed.z, cell.origin.z, cell.span),
            ),
            1 => local_edge_index(
                1,
                boundary_bit(fixed.x, cell.origin.x, cell.span),
                boundary_bit(fixed.z, cell.origin.z, cell.span),
            ),
            _ => local_edge_index(
                2,
                boundary_bit(fixed.x, cell.origin.x, cell.span),
                boundary_bit(fixed.y, cell.origin.y, cell.span),
            ),
        };
        route.map_or(ComponentRoute::OnlyComponent, ComponentRoute::LocalEdge)
    }

    fn find_covering(&self, coord: CornerKey) -> Option<CellId> {
        let mut span = 1_u32;
        loop {
            let depth =
                u8::try_from(self.resolution.ilog2() - span.ilog2()).expect("octree depth fits u8");
            let key = CellKey {
                origin: CornerKey::new(
                    align_down(coord.x, span),
                    align_down(coord.y, span),
                    align_down(coord.z, span),
                ),
                span,
                depth,
            };
            if let Some(id) = self.leaves.get(&key) {
                return Some(*id);
            }
            if span >= self.resolution {
                return None;
            }
            span *= 2;
        }
    }
}

pub(super) trait BalanceContext: OctreeVisitor {
    type Field: ScalarField;

    fn transition_grid(&self) -> &AdaptiveGrid<'_, Self::Field>;
    fn global_max_depth(&self) -> u8;
    fn failed(&self) -> bool;
}

pub(super) fn balance_tree<V>(tree: &mut Octree<V::Payload>, visitor: &mut V)
where
    V: BalanceContext,
{
    loop {
        let leaves = sorted_leaf_keys(tree, visitor.transition_grid());
        let mut faces = HashMap::<FacePatchKey, [Option<CellId>; 2]>::new();
        for &(id, key) in &leaves {
            for axis in 0_u8..3 {
                for side in 0_u8..2 {
                    let (patch, owner_side) = face_patch(key, axis, side);
                    let owners = faces.entry(patch).or_insert([None; 2]);
                    debug_assert!(
                        owners[owner_side].is_none(),
                        "one leaf must own each side of an exact face patch"
                    );
                    owners[owner_side] = Some(id);
                }
            }
        }

        let mut refine = Vec::new();
        for &(_, key) in &leaves {
            for axis in 0_u8..3 {
                for side in 0_u8..2 {
                    let (patch, owner_side) = face_patch(key, axis, side);
                    let opposite = 1 - owner_side;
                    let mut query_span = patch.span;
                    loop {
                        let query = FacePatchKey {
                            span: query_span,
                            u: align_down(patch.u, query_span),
                            v: align_down(patch.v, query_span),
                            ..patch
                        };
                        if let Some(neighbor) =
                            faces.get(&query).and_then(|owners| owners[opposite])
                        {
                            let neighbor_key = visitor.transition_grid().cell_key(neighbor);
                            if key.depth > neighbor_key.depth + 1 {
                                refine.push((neighbor_key, neighbor));
                            }
                            break;
                        }
                        if query_span >= visitor.transition_grid().resolution() {
                            break;
                        }
                        query_span *= 2;
                    }
                }
            }
        }

        refine.sort_unstable_by_key(|&(key, id)| (key, id));
        refine.dedup_by_key(|entry| entry.1);
        if refine.is_empty() {
            return;
        }
        let max_depth = visitor.global_max_depth();
        for (_, id) in refine {
            tree.refine_leaf(id, max_depth, visitor)
                .expect("balancing candidates are current octree leaves");
        }
        if visitor.failed() {
            return;
        }
    }
}

pub(super) fn leaf_keys<P, F: ScalarField>(
    tree: &Octree<P>,
    grid: &AdaptiveGrid<'_, F>,
) -> LeafSet {
    sorted_leaf_keys(tree, grid)
}

pub(super) fn enumerate_segments(leaves: &[(CellId, CellKey)]) -> Vec<EdgeSegmentKey> {
    let mut intervals = Vec::with_capacity(leaves.len() * 12);
    for &(_, cell) in leaves {
        for axis in 0_u8..3 {
            for first_high in [false, true] {
                for second_high in [false, true] {
                    intervals.push(edge_interval(cell, axis, first_high, second_high));
                }
            }
        }
    }
    intervals.sort_unstable_by_key(|interval| (interval.line, interval.start, interval.end));

    let mut segments = Vec::new();
    let mut first = 0_usize;
    while first < intervals.len() {
        let line = intervals[first].line;
        let mut end = first + 1;
        while end < intervals.len() && intervals[end].line == line {
            end += 1;
        }
        let group = &intervals[first..end];
        let mut breakpoints = Vec::with_capacity(group.len() * 2);
        for interval in group {
            breakpoints.push(interval.start);
            breakpoints.push(interval.end);
        }
        breakpoints.sort_unstable();
        breakpoints.dedup();
        for interval in group {
            for window in breakpoints.windows(2) {
                let start = window[0];
                let finish = window[1];
                if start >= interval.start && finish <= interval.end {
                    segments.push(segment_from_line(line, start, finish - start));
                }
            }
        }
        first = end;
    }
    segments.sort_unstable_by_key(|segment| {
        (
            segment.axis,
            segment.start.z,
            segment.start.y,
            segment.start.x,
            segment.length,
        )
    });
    segments.dedup();
    segments
}

pub(super) fn segment_end(segment: EdgeSegmentKey) -> CornerKey {
    match segment.axis {
        0 => CornerKey::new(
            segment.start.x + segment.length,
            segment.start.y,
            segment.start.z,
        ),
        1 => CornerKey::new(
            segment.start.x,
            segment.start.y + segment.length,
            segment.start.z,
        ),
        _ => CornerKey::new(
            segment.start.x,
            segment.start.y,
            segment.start.z + segment.length,
        ),
    }
}

fn sorted_leaf_keys<P, F: ScalarField>(tree: &Octree<P>, grid: &AdaptiveGrid<'_, F>) -> LeafSet {
    let mut leaves = tree
        .leaf_ids()
        .into_iter()
        .map(|id| (id, grid.cell_key(id)))
        .collect::<Vec<_>>();
    leaves.sort_unstable_by_key(|&(id, key)| (key, id));
    leaves
}

fn face_patch(cell: CellKey, axis: u8, side: u8) -> (FacePatchKey, usize) {
    let origin = cell.origin;
    let (plane, u, v) = match axis {
        0 => (origin.x + u32::from(side) * cell.span, origin.y, origin.z),
        1 => (origin.y + u32::from(side) * cell.span, origin.x, origin.z),
        _ => (origin.z + u32::from(side) * cell.span, origin.x, origin.y),
    };
    (
        FacePatchKey {
            axis,
            plane,
            u,
            v,
            span: cell.span,
        },
        usize::from(1 - side),
    )
}

fn edge_interval(cell: CellKey, axis: u8, first_high: bool, second_high: bool) -> EdgeInterval {
    let offset_first = u32::from(first_high) * cell.span;
    let offset_second = u32::from(second_high) * cell.span;
    match axis {
        0 => EdgeInterval {
            line: EdgeLineKey {
                axis,
                fixed_u: cell.origin.y + offset_first,
                fixed_v: cell.origin.z + offset_second,
            },
            start: cell.origin.x,
            end: cell.origin.x + cell.span,
        },
        1 => EdgeInterval {
            line: EdgeLineKey {
                axis,
                fixed_u: cell.origin.x + offset_first,
                fixed_v: cell.origin.z + offset_second,
            },
            start: cell.origin.y,
            end: cell.origin.y + cell.span,
        },
        _ => EdgeInterval {
            line: EdgeLineKey {
                axis,
                fixed_u: cell.origin.x + offset_first,
                fixed_v: cell.origin.y + offset_second,
            },
            start: cell.origin.z,
            end: cell.origin.z + cell.span,
        },
    }
}

fn segment_from_line(line: EdgeLineKey, start: u32, length: u32) -> EdgeSegmentKey {
    let start = match line.axis {
        0 => CornerKey::new(start, line.fixed_u, line.fixed_v),
        1 => CornerKey::new(line.fixed_u, start, line.fixed_v),
        _ => CornerKey::new(line.fixed_u, line.fixed_v, start),
    };
    EdgeSegmentKey {
        axis: line.axis,
        start,
        length,
    }
}

fn boundary_bit(value: u32, origin: u32, span: u32) -> Option<u8> {
    if value == origin {
        Some(0)
    } else if value == origin + span {
        Some(1)
    } else {
        None
    }
}

fn local_edge_index(axis: u8, first: Option<u8>, second: Option<u8>) -> Option<u8> {
    let (first, second) = (usize::from(first?), usize::from(second?));
    Some(match axis {
        0 => [[0, 4], [2, 6]][first][second],
        1 => [[3, 7], [1, 5]][first][second],
        _ => [[8, 11], [9, 10]][first][second],
    })
}

fn cell_corner_keys(cell: CellKey) -> [CornerKey; 8] {
    core::array::from_fn(|corner| CornerKey {
        x: cell.origin.x + bit(corner, 0) * cell.span,
        y: cell.origin.y + bit(corner, 1) * cell.span,
        z: cell.origin.z + bit(corner, 2) * cell.span,
    })
}

fn align_down(value: u32, span: u32) -> u32 {
    debug_assert!(span.is_power_of_two(), "octree spans must be dyadic");
    value & !(span - 1)
}

fn bit(value: usize, shift: u32) -> u32 {
    u32::from((value & (1_usize << shift)) != 0)
}

fn coordinate_axis(root_min: f32, value: f32, step: f32) -> u32 {
    let mut coord = 0_u32;
    let mut best_distance = abs(value - root_min);
    loop {
        let next = coord + 1;
        let next_value = root_min + step * next as f32;
        let next_distance = abs(value - next_value);
        if next_distance + 1.0e-6 < best_distance {
            coord = next;
            best_distance = next_distance;
        } else {
            return coord;
        }
    }
}

fn axis_point(min: f32, max: f32, step: f32, key: u32, resolution: u32) -> f32 {
    if key == 0 {
        min
    } else if key == resolution {
        max
    } else {
        min + step * key as f32
    }
}

fn abs(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.abs()
    }
    #[cfg(not(feature = "std"))]
    {
        libm::fabsf(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConstantField;

    impl ScalarField for ConstantField {
        fn eval_interval(&self, _bounds: &Aabb) -> Option<[f32; 2]> {
            Some([1.0, 1.0])
        }

        fn eval_points(&self, _points: &[[f32; 3]], out: &mut [f32]) {
            out.fill(1.0);
        }

        fn eval_gradients(&self, _points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            out.fill([1.0, 1.0, 0.0, 0.0]);
        }
    }

    struct SplitVisitor<'a> {
        grid: AdaptiveGrid<'a, ConstantField>,
        target_depth: u8,
        axis: usize,
        fine_high: bool,
    }

    impl OctreeVisitor for SplitVisitor<'_> {
        type Payload = ();

        fn should_subdivide(&mut self, cell: CellRef) -> bool {
            self.grid.locate_cell(cell);
            let in_fine_half = if self.fine_high {
                cell.bounds.min[self.axis] >= 0.0
            } else {
                cell.bounds.max[self.axis] <= 0.0
            };
            cell.depth == 0 || (cell.depth < self.target_depth && in_fine_half)
        }

        fn make_leaf_payload(&mut self, cell: CellRef) -> Self::Payload {
            self.grid.locate_cell(cell);
        }
    }

    impl BalanceContext for SplitVisitor<'_> {
        type Field = ConstantField;

        fn transition_grid(&self) -> &AdaptiveGrid<'_, Self::Field> {
            &self.grid
        }

        fn global_max_depth(&self) -> u8 {
            self.target_depth
        }

        fn failed(&self) -> bool {
            false
        }
    }

    fn key(origin: [u32; 3], span: u32, depth: u8) -> CellKey {
        CellKey {
            origin: CornerKey::new(origin[0], origin[1], origin[2]),
            span,
            depth,
        }
    }

    fn oriented_corner(axis: u8, along: u32, first: u32, second: u32) -> CornerKey {
        match axis {
            0 => CornerKey::new(along, first, second),
            1 => CornerKey::new(first, along, second),
            _ => CornerKey::new(first, second, along),
        }
    }

    fn oriented_key(axis: u8, along: u32, first: u32, second: u32, span: u32) -> CellKey {
        let origin = oriented_corner(axis, along, first, second);
        CellKey {
            origin,
            span,
            depth: u8::try_from(3 - span.ilog2()).expect("fixture depth fits u8"),
        }
    }

    fn overlaps_strictly(a0: u32, a1: u32, b0: u32, b1: u32) -> bool {
        a0.max(b0) < a1.min(b1)
    }

    fn face_adjacent(a: CellKey, b: CellKey) -> bool {
        let a0 = a.origin.as_tuple();
        let b0 = b.origin.as_tuple();
        let a1 = (a0.0 + a.span, a0.1 + a.span, a0.2 + a.span);
        let b1 = (b0.0 + b.span, b0.1 + b.span, b0.2 + b.span);
        ((a1.0 == b0.0 || b1.0 == a0.0)
            && overlaps_strictly(a0.1, a1.1, b0.1, b1.1)
            && overlaps_strictly(a0.2, a1.2, b0.2, b1.2))
            || ((a1.1 == b0.1 || b1.1 == a0.1)
                && overlaps_strictly(a0.0, a1.0, b0.0, b1.0)
                && overlaps_strictly(a0.2, a1.2, b0.2, b1.2))
            || ((a1.2 == b0.2 || b1.2 == a0.2)
                && overlaps_strictly(a0.0, a1.0, b0.0, b1.0)
                && overlaps_strictly(a0.1, a1.1, b0.1, b1.1))
    }

    #[test]
    fn integer_grid_preserves_exact_root_endpoints_and_shared_boundaries() {
        let field = ConstantField;
        let min = 0.021_897_81_f32;
        let max = 0.061_946_902_f32;
        let resolution = 64;
        let step = (max - min) / resolution as f32;
        assert_ne!(
            (min + step * resolution as f32).to_bits(),
            max.to_bits(),
            "fixture must expose endpoint recomposition drift"
        );
        let bounds = Aabb::new([min, -0.37, 1.13], [max, 0.91, 2.07]).expect("bounds");
        let grid = AdaptiveGrid::new(&field, bounds, resolution);

        assert_eq!(grid.cell_bounds(key([0, 0, 0], resolution, 0)), bounds);
        let low = grid.cell_bounds(key([31, 17, 9], 1, 6));
        let high_x = grid.cell_bounds(key([32, 17, 9], 1, 6));
        let high_y = grid.cell_bounds(key([31, 18, 9], 1, 6));
        let high_z = grid.cell_bounds(key([31, 17, 10], 1, 6));
        assert_eq!(low.max[0].to_bits(), high_x.min[0].to_bits());
        assert_eq!(low.max[1].to_bits(), high_y.min[1].to_bits());
        assert_eq!(low.max[2].to_bits(), high_z.min[2].to_bits());
    }

    #[test]
    fn balancing_closes_depth_two_and_three_face_gaps() {
        let field = ConstantField;
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("bounds");
        for target_depth in [3_u8, 4] {
            for axis in 0..3 {
                for fine_high in [false, true] {
                    let resolution = 1_u32 << target_depth;
                    let mut visitor = SplitVisitor {
                        grid: AdaptiveGrid::new(&field, bounds, resolution),
                        target_depth,
                        axis,
                        fine_high,
                    };
                    let mut tree = Octree::build(bounds, target_depth, &mut visitor);
                    let before = leaf_keys(&tree, &visitor.grid);
                    assert!(before.iter().any(|(_, a)| {
                        before.iter().any(|(_, b)| {
                            face_adjacent(*a, *b) && a.depth.abs_diff(b.depth) == target_depth - 1
                        })
                    }));

                    balance_tree(&mut tree, &mut visitor);
                    let after = leaf_keys(&tree, &visitor.grid);
                    for (index, &(_, a)) in after.iter().enumerate() {
                        for &(_, b) in &after[index + 1..] {
                            if face_adjacent(a, b) {
                                assert!(
                                    a.depth.abs_diff(b.depth) <= 1,
                                    "axis={axis}, fine_high={fine_high}: {a:?} / {b:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn transition_order_and_component_routes_cover_all_axes_and_face_sides() {
        let low_incident = [[0, 0, 2, 1], [0, 1, 2, 0], [0, 0, 2, 1]];
        let high_incident = [[1, 2, 0, 0], [1, 0, 0, 2], [1, 2, 0, 0]];
        let low_edges = [[2, 0], [1, 3], [9, 8]];
        let high_edges = [[6, 4], [5, 7], [10, 11]];

        for axis in 0_u8..3 {
            for coarse_high in [false, true] {
                let coarse_second = if coarse_high { 4 } else { 0 };
                let fine_second = if coarse_high { 2 } else { 4 };
                let leaves = [
                    (
                        CellId::from_index(0),
                        oriented_key(axis, 0, 0, coarse_second, 4),
                    ),
                    (
                        CellId::from_index(1),
                        oriented_key(axis, 2, 0, fine_second, 2),
                    ),
                    (
                        CellId::from_index(2),
                        oriented_key(axis, 2, 2, fine_second, 2),
                    ),
                ];
                let locator = LeafLocator::new(&leaves, 8);
                let segment = EdgeSegmentKey {
                    axis,
                    start: oriented_corner(axis, 2, 2, 4),
                    length: 2,
                };
                let incident = locator
                    .incident_leaves(segment)
                    .expect("fixture partition is complete")
                    .expect("fixture segment is interior");
                let indices = incident.map(CellId::index);
                let expected = if coarse_high {
                    high_incident[usize::from(axis)]
                } else {
                    low_incident[usize::from(axis)]
                };
                assert_eq!(indices, expected, "axis={axis}, coarse_high={coarse_high}");
                assert_eq!(
                    locator.component_route(CellId::from_index(0), segment),
                    ComponentRoute::OnlyComponent
                );
                let expected_edges = if coarse_high {
                    high_edges[usize::from(axis)]
                } else {
                    low_edges[usize::from(axis)]
                };
                assert_eq!(
                    locator.component_route(CellId::from_index(1), segment),
                    ComponentRoute::LocalEdge(expected_edges[0])
                );
                assert_eq!(
                    locator.component_route(CellId::from_index(2), segment),
                    ComponentRoute::LocalEdge(expected_edges[1])
                );
                assert!(enumerate_segments(&leaves).contains(&segment));
            }
        }
    }

    #[test]
    fn incident_lookup_distinguishes_domain_boundary_from_missing_partition() {
        let interior = EdgeSegmentKey {
            axis: 0,
            start: CornerKey::new(2, 2, 2),
            length: 1,
        };
        let incomplete = LeafLocator::new(&[(CellId::from_index(0), key([0, 0, 0], 2, 2))], 4);
        assert_eq!(incomplete.incident_leaves(interior), Err(()));

        let boundary = EdgeSegmentKey {
            axis: 0,
            start: CornerKey::new(0, 0, 0),
            length: 1,
        };
        assert_eq!(incomplete.incident_leaves(boundary), Ok(None));
    }
}
