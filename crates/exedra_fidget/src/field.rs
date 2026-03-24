// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fidget-backed field wrapper implementations.

use core::fmt;
use std::sync::{Mutex, MutexGuard};

use exedra_isosurface::{ScalarField, SpecializableField};
use exedra_spatial::Aabb;
use fidget::{
    eval::{BulkEvaluator, Function, TracingEvaluator},
    shape::{EzShape, Shape, ShapeBulkEval, ShapeTape, ShapeTracingEval},
    types::{Grad, Interval},
    var::Var,
};

use crate::FidgetFieldError;

/// `ScalarField` adapter for any `fidget::shape::Shape`.
pub struct FidgetField<F: Function, T = ()> {
    shape: Shape<F, T>,
    interval: Mutex<IntervalCache<F>>,
    float: Mutex<BulkCache<F::FloatSliceEval>>,
    grad: Mutex<BulkCache<F::GradSliceEval>>,
}

/// Convenience alias for the VM-backed adapter.
pub type VmField = FidgetField<fidget::vm::VmFunction>;

/// Convenience alias for the JIT-backed adapter.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
pub type JitField = FidgetField<fidget::jit::JitFunction>;

struct IntervalCache<F: Function> {
    eval: ShapeTracingEval<F::IntervalEval>,
    tape: ShapeTape<<F::IntervalEval as TracingEvaluator>::Tape>,
}

struct BulkCache<E: BulkEvaluator> {
    eval: ShapeBulkEval<E>,
    tape: ShapeTape<E::Tape>,
}

impl<F: Function + Clone, T> FidgetField<F, T> {
    /// Wraps a `fidget` shape as an `exedra_isosurface::ScalarField`.
    ///
    /// The shape must depend only on the `x`, `y`, and `z` axes.
    pub fn new(shape: Shape<F, T>) -> Result<Self, FidgetFieldError> {
        reject_extra_vars(&shape)?;
        let interval = IntervalCache {
            eval: Shape::<F, T>::new_interval_eval(),
            tape: shape.ez_interval_tape(),
        };
        let float = BulkCache {
            eval: Shape::<F, T>::new_float_slice_eval(),
            tape: shape.ez_float_slice_tape(),
        };
        let grad = BulkCache {
            eval: Shape::<F, T>::new_grad_slice_eval(),
            tape: shape.ez_grad_slice_tape(),
        };

        Ok(Self {
            shape,
            interval: Mutex::new(interval),
            float: Mutex::new(float),
            grad: Mutex::new(grad),
        })
    }

    /// Returns the wrapped `fidget` shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape<F, T> {
        &self.shape
    }
}

impl<F: Function + Clone, T> fmt::Debug for FidgetField<F, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FidgetField")
            .field("size", &self.shape.size())
            .field("can_simplify", &self.shape.inner().can_simplify())
            .finish_non_exhaustive()
    }
}

impl<F: Function + Clone, T> TryFrom<Shape<F, T>> for FidgetField<F, T> {
    type Error = FidgetFieldError;

    fn try_from(shape: Shape<F, T>) -> Result<Self, Self::Error> {
        Self::new(shape)
    }
}

impl<F: Function + Clone, T> ScalarField for FidgetField<F, T> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let mut cache = lock_unpoison(&self.interval);
        let x = Interval::from([bounds.min[0], bounds.max[0]]);
        let y = Interval::from([bounds.min[1], bounds.max[1]]);
        let z = Interval::from([bounds.min[2], bounds.max[2]]);
        let tape = cache.tape.clone();
        match cache.eval.eval(&tape, x, y, z) {
            Ok((interval, _trace)) => Some([interval.lower(), interval.upper()]),
            Err(_error) => None,
        }
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        if points.is_empty() {
            return;
        }
        let (x, y, z) = split_points::<f32>(points);
        let mut cache = lock_unpoison(&self.float);
        let tape = cache.tape.clone();
        match cache.eval.eval(&tape, &x, &y, &z) {
            Ok(values) => out.copy_from_slice(values),
            Err(_error) => out.fill(f32::NAN),
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        if points.is_empty() {
            return;
        }
        let (x, y, z) = split_grad_points(points);
        let mut cache = lock_unpoison(&self.grad);
        let tape = cache.tape.clone();
        match cache.eval.eval(&tape, &x, &y, &z) {
            Ok(values) => {
                for (slot, grad) in out.iter_mut().zip(values.iter()) {
                    *slot = grad_to_array(*grad);
                }
            }
            Err(_error) => out.fill([f32::NAN; 4]),
        }
    }
}

impl<F: Function + Clone, T> SpecializableField for FidgetField<F, T> {
    type Specialized = Self;

    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized> {
        if !self.shape.inner().can_simplify() {
            return None;
        }

        let trace = {
            let mut cache = lock_unpoison(&self.interval);
            let x = Interval::from([bounds.min[0], bounds.max[0]]);
            let y = Interval::from([bounds.min[1], bounds.max[1]]);
            let z = Interval::from([bounds.min[2], bounds.max[2]]);
            let tape = cache.tape.clone();
            match cache.eval.eval(&tape, x, y, z) {
                Ok((_interval, Some(trace))) => trace.clone(),
                Ok((_interval, None)) => return None,
                Err(_) => return None,
            }
        };

        let shape = self.shape.ez_simplify(&trace).ok()?;
        if shape.size() >= self.shape.size() {
            return None;
        }

        Self::new(shape).ok()
    }
}

fn reject_extra_vars<F: Function, T>(shape: &Shape<F, T>) -> Result<(), FidgetFieldError> {
    let vars = shape.inner().vars();
    let axis_count = vars.get(&Var::X).is_some() as usize
        + vars.get(&Var::Y).is_some() as usize
        + vars.get(&Var::Z).is_some() as usize;
    let extra = vars.len().saturating_sub(axis_count);
    if extra == 0 {
        Ok(())
    } else {
        Err(FidgetFieldError::ExtraVariables { count: extra })
    }
}

fn split_points<T: From<f32>>(points: &[[f32; 3]]) -> (Vec<T>, Vec<T>, Vec<T>) {
    let mut x = Vec::with_capacity(points.len());
    let mut y = Vec::with_capacity(points.len());
    let mut z = Vec::with_capacity(points.len());
    for point in points {
        x.push(T::from(point[0]));
        y.push(T::from(point[1]));
        z.push(T::from(point[2]));
    }
    (x, y, z)
}

fn split_grad_points(points: &[[f32; 3]]) -> (Vec<Grad>, Vec<Grad>, Vec<Grad>) {
    let mut x = Vec::with_capacity(points.len());
    let mut y = Vec::with_capacity(points.len());
    let mut z = Vec::with_capacity(points.len());
    for point in points {
        x.push(Grad::new(point[0], 1.0, 0.0, 0.0));
        y.push(Grad::new(point[1], 0.0, 1.0, 0.0));
        z.push(Grad::new(point[2], 0.0, 0.0, 1.0));
    }
    (x, y, z)
}

fn grad_to_array(grad: Grad) -> [f32; 4] {
    [grad.v, grad.dx, grad.dy, grad.dz]
}

fn lock_unpoison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn assert_same_len(expected: usize, found: usize, label: &str) {
    assert_eq!(expected, found, "{label} slice lengths must match");
}

#[cfg(test)]
mod tests {
    use exedra_isosurface::{
        DualContourParams, EdgeSearchParams, ScalarField, SpecializableField, dual_contour,
    };
    use exedra_qef::QefParams;
    use exedra_spatial::Aabb;
    use fidget::{context::Tree, var::Var, vm::VmShape};

    use super::{FidgetField, VmField};

    fn params(bounds: Aabb, max_depth: u8) -> DualContourParams {
        DualContourParams {
            root_bounds: bounds,
            max_depth,
            cell_budget: None,
            edge_search: EdgeSearchParams {
                bisection_steps: 10,
            },
            qef: QefParams::default(),
        }
    }

    fn box_tree(center_x: f32, half_extents: [f32; 3]) -> Tree {
        let x = Tree::x();
        let y = Tree::y();
        let z = Tree::z();
        let qx = (x - center_x).abs() - half_extents[0];
        let qy = y.abs() - half_extents[1];
        let qz = z.abs() - half_extents[2];
        let ox = qx.max(0.0);
        let oy = qy.max(0.0);
        let oz = qz.max(0.0);
        let outside = (ox.square() + oy.square() + oz.square()).sqrt();
        let inside = qx.max(qy.max(qz)).min(0.0);
        outside + inside
    }

    fn nearest_center(value: f32, root_min: f32, step: f32) -> f32 {
        let center_index = ((value - root_min) / step - 0.5).round();
        root_min + (center_index + 0.5) * step
    }

    #[test]
    fn vm_field_extracts_sphere() {
        let x = Tree::x();
        let y = Tree::y();
        let z = Tree::z();
        let sphere = (x.square() + y.square() + z.square()).sqrt() - 1.0;
        let field = VmField::new(VmShape::from(sphere)).expect("axis-only shape");
        let bounds = Aabb::new([-1.5, -1.5, -1.5], [1.5, 1.5, 1.5]).expect("bounds");

        let result = dual_contour(&field, &params(bounds, 4))
            .expect("fidget sphere extraction should succeed");

        assert!(result.mesh.validate_deep().is_empty());
        assert!(result.stats.faces > 0);
    }

    #[test]
    fn vm_field_gradients_seed_basis_derivatives() {
        let field = VmField::new(VmShape::from(Tree::x())).expect("axis-only shape");
        let mut gradients = [[0.0_f32; 4]; 1];
        field.eval_gradients(&[[2.5, -3.0, 4.0]], &mut gradients);

        assert_eq!(gradients[0], [2.5, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn vm_field_sphere_vertices_move_off_center_lattice() {
        let x = Tree::x();
        let y = Tree::y();
        let z = Tree::z();
        let sphere = (x.square() + y.square() + z.square()).sqrt() - 1.0;
        let field = VmField::new(VmShape::from(sphere)).expect("axis-only shape");
        let bounds = Aabb::new([-1.5, -1.5, -1.5], [1.5, 1.5, 1.5]).expect("bounds");
        let params = params(bounds, 4);

        let result =
            dual_contour(&field, &params).expect("fidget sphere extraction should succeed");
        let resolution = 1_u32 << params.max_depth;
        let step = [
            (params.root_bounds.max[0] - params.root_bounds.min[0]) / resolution as f32,
            (params.root_bounds.max[1] - params.root_bounds.min[1]) / resolution as f32,
            (params.root_bounds.max[2] - params.root_bounds.min[2]) / resolution as f32,
        ];

        let moved = result.mesh.vertices().any(|vertex| {
            let position = result.mesh.vertex_position(vertex).expect("live vertex");
            let center = [
                nearest_center(position[0], params.root_bounds.min[0], step[0]),
                nearest_center(position[1], params.root_bounds.min[1], step[1]),
                nearest_center(position[2], params.root_bounds.min[2], step[2]),
            ];
            let dx = position[0] - center[0];
            let dy = position[1] - center[1];
            let dz = position[2] - center[2];
            dx * dx + dy * dy + dz * dz > 1.0e-8
        });

        assert!(moved);
    }

    #[test]
    fn vm_field_union_builds_valid_mesh() {
        let left = box_tree(-0.9, [0.35, 0.35, 0.35]);
        let right = box_tree(0.9, [0.35, 0.45, 0.35]);
        let union = left.min(right);
        let field = VmField::new(VmShape::from(union)).expect("axis-only shape");
        let bounds = Aabb::new([-1.6, -1.0, -1.0], [1.6, 1.0, 1.0]).expect("bounds");

        let result = dual_contour(&field, &params(bounds, 4))
            .expect("fidget union extraction should succeed");

        assert!(result.mesh.validate_deep().is_empty());
        assert!(result.stats.faces > 0);
    }

    #[test]
    fn vm_field_box_builds_valid_mesh() {
        let field =
            VmField::new(VmShape::from(box_tree(0.0, [0.7, 0.7, 0.7]))).expect("axis-only shape");
        let bounds = Aabb::new([-1.2, -1.2, -1.2], [1.2, 1.2, 1.2]).expect("bounds");
        let result =
            dual_contour(&field, &params(bounds, 4)).expect("fidget box extraction should succeed");

        assert!(result.mesh.validate_deep().is_empty());
        assert!(result.stats.faces > 0);
    }

    #[test]
    fn vm_field_specialize_reduces_dominated_branch() {
        let shape = VmShape::from(Tree::x().min(Tree::y()));
        let field = VmField::new(shape).expect("axis-only shape");
        let bounds = Aabb::new([-2.0, 1.0, 0.0], [-1.0, 2.0, 0.0]).expect("bounds");

        let specialized = field.specialize(&bounds).expect("shape should simplify");
        assert!(specialized.shape().size() < field.shape().size());
    }

    #[test]
    fn rejects_extra_variables() {
        let shape = VmShape::from(Tree::from(Var::new()) + Tree::x());

        assert!(FidgetField::new(shape).is_err());
    }
}
