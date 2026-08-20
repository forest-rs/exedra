// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Separated field-work counters and dyadic leaf-histogram reconstruction.

use std::cell::RefCell;
use std::collections::BTreeSet;

use exedra_isosurface::{
    AnalyticPrimitive, ScalarField, SemiAnalyticField, SemiAnalyticProjection,
    SemiAnalyticProjectionOutcome,
};
use exedra_spatial::Aabb;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkMeasurements {
    pub(crate) interval_calls: usize,
    pub(crate) interval_elements: usize,
    pub(crate) interval_cells: Vec<AabbBits>,
    pub(crate) point_calls: usize,
    pub(crate) point_elements: usize,
    pub(crate) gradient_calls: usize,
    pub(crate) gradient_elements: usize,
    pub(crate) projection_attempts: usize,
    pub(crate) projection_cells: Vec<AabbBits>,
}

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct AabbBits {
    min: [u32; 3],
    max: [u32; 3],
}

impl From<Aabb> for AabbBits {
    fn from(value: Aabb) -> Self {
        Self {
            min: value.min.map(f32::to_bits),
            max: value.max.map(f32::to_bits),
        }
    }
}

impl AabbBits {
    fn aabb(self) -> Aabb {
        Aabb {
            min: self.min.map(f32::from_bits),
            max: self.max.map(f32::from_bits),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Measured<F> {
    field: F,
    work: RefCell<WorkMeasurements>,
}

impl<F> Measured<F> {
    pub(crate) fn new(field: F) -> Self {
        Self {
            field,
            work: RefCell::new(WorkMeasurements::default()),
        }
    }

    pub(crate) fn snapshot(&self) -> WorkMeasurements {
        self.work.borrow().clone()
    }
}

impl<F: ScalarField> ScalarField for Measured<F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let mut work = self.work.borrow_mut();
        work.interval_calls += 1;
        work.interval_elements += 1;
        work.interval_cells.push((*bounds).into());
        drop(work);
        self.field.eval_interval(bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        let mut work = self.work.borrow_mut();
        work.point_calls += 1;
        work.point_elements += points.len();
        drop(work);
        self.field.eval_points(points, out);
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        let mut work = self.work.borrow_mut();
        work.gradient_calls += 1;
        work.gradient_elements += points.len();
        drop(work);
        self.field.eval_gradients(points, out);
    }
}

impl<F: SemiAnalyticField> SemiAnalyticField for Measured<F> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        self.field.project_cell_vertex(point, cell)
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        let mut work = self.work.borrow_mut();
        work.projection_attempts += 1;
        work.projection_cells.push((*cell).into());
        drop(work);
        self.field.project_cell_vertex_detailed(point, cell)
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        self.field.primitive_at(point)
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        self.field.leaf_primitive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DyadicMeasurements {
    pub(crate) unique_interval_cells: usize,
    pub(crate) final_leaf_depths: Vec<usize>,
    pub(crate) contributing_depths: Vec<usize>,
}

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CellKey {
    origin: [u32; 3],
    span: u32,
    depth: u8,
}

pub(crate) fn reconstruct_dyadic(
    work: &WorkMeasurements,
    root: Aabb,
    max_depth: u8,
) -> DyadicMeasurements {
    let unique = work
        .interval_cells
        .iter()
        .copied()
        .map(|bounds| cell_key(bounds.aabb(), root, max_depth))
        .collect::<BTreeSet<_>>();
    let mut final_leaf_depths = vec![0; usize::from(max_depth) + 1];
    for key in &unique {
        if key.depth == max_depth || !children(*key).iter().any(|child| unique.contains(child)) {
            final_leaf_depths[usize::from(key.depth)] += 1;
        }
    }
    let mut contributing_depths = vec![0; usize::from(max_depth) + 1];
    for bounds in &work.projection_cells {
        let key = cell_key(bounds.aabb(), root, max_depth);
        contributing_depths[usize::from(key.depth)] += 1;
    }
    DyadicMeasurements {
        unique_interval_cells: unique.len(),
        final_leaf_depths,
        contributing_depths,
    }
}

fn children(key: CellKey) -> [CellKey; 8] {
    let child_span = key.span / 2;
    core::array::from_fn(|corner| CellKey {
        origin: core::array::from_fn(|axis| {
            key.origin[axis] + child_span * u32::from(((corner >> axis) & 1) != 0)
        }),
        span: child_span,
        depth: key.depth + 1,
    })
}

fn cell_key(bounds: Aabb, root: Aabb, max_depth: u8) -> CellKey {
    let resolution = 1_u32 << max_depth;
    let root_extent = root.extent();
    let step = root_extent.map(|extent| extent / resolution as f32);
    let origin: [u32; 3] = core::array::from_fn(|axis| {
        let approximate = ((bounds.min[axis] - root.min[axis]) / step[axis]).round();
        assert!(approximate >= 0.0 && approximate <= resolution as f32);
        let key = integer_key(approximate, resolution);
        assert_eq!(
            axis_point(root, step, resolution, axis, key).to_bits(),
            bounds.min[axis].to_bits()
        );
        key
    });
    let maximum: [u32; 3] = core::array::from_fn(|axis| {
        let approximate = ((bounds.max[axis] - root.min[axis]) / step[axis]).round();
        assert!(approximate >= 0.0 && approximate <= resolution as f32);
        let key = integer_key(approximate, resolution);
        assert_eq!(
            axis_point(root, step, resolution, axis, key).to_bits(),
            bounds.max[axis].to_bits()
        );
        key
    });
    let spans = core::array::from_fn::<_, 3, _>(|axis| maximum[axis] - origin[axis]);
    assert!(
        spans[0].is_power_of_two(),
        "non-dyadic interval bounds: {bounds:?}"
    );
    assert_eq!(spans, [spans[0]; 3], "non-cubic interval key: {bounds:?}");
    let span = spans[0];
    let depth = max_depth - u8::try_from(span.ilog2()).expect("span depth fits u8");
    CellKey {
        origin,
        span,
        depth,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the caller proves this rounded dyadic coordinate lies in the u32 resolution"
)]
fn integer_key(value: f32, resolution: u32) -> u32 {
    assert!(value >= 0.0 && value <= resolution as f32);
    value as u32
}

fn axis_point(root: Aabb, step: [f32; 3], resolution: u32, axis: usize, key: u32) -> f32 {
    if key == 0 {
        root.min[axis]
    } else if key == resolution {
        root.max[axis]
    } else {
        root.min[axis] + step[axis] * key as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{AabbBits, WorkMeasurements, reconstruct_dyadic};
    use exedra_spatial::Aabb;

    #[test]
    fn dyadic_reconstruction_finds_final_leaves() {
        let root = Aabb::new([-1.3, -2.1, -3.7], [2.9, 3.5, 4.1]).expect("root");
        let center = core::array::from_fn(|axis| {
            let step = (root.max[axis] - root.min[axis]) / 16.0;
            root.min[axis] + step * 8.0
        });
        let low = Aabb::new(root.min, center).expect("low child");
        let work = WorkMeasurements {
            interval_calls: 2,
            interval_cells: vec![root.into(), low.into()],
            projection_attempts: 1,
            projection_cells: vec![AabbBits::from(low)],
            ..WorkMeasurements::default()
        };
        let measured = reconstruct_dyadic(&work, root, 4);
        assert_eq!(measured.unique_interval_cells, 2);
        assert_eq!(measured.final_leaf_depths, [0, 1, 0, 0, 0]);
        assert_eq!(measured.contributing_depths, [0, 1, 0, 0, 0]);
    }
}
