// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fidget-backed field wrapper implementations.

use core::fmt;
use std::sync::{Mutex, MutexGuard};

use exedra_implicit::{Aabb, Dual, ScalarField};
use fidget::{
    eval::{BulkEvaluator, Function, TracingEvaluator},
    shape::{EzShape, Shape, ShapeBulkEval, ShapeTape, ShapeTracingEval},
    types::{Grad, Interval},
    var::Var,
};

use crate::FidgetFieldError;

/// `ScalarField` adapter for any `fidget::shape::Shape`.
pub struct FidgetField<F: Function> {
    shape: Shape<F>,
    interval: Mutex<IntervalCache<F>>,
    float: Mutex<FloatCache<F::FloatSliceEval>>,
    grad: Mutex<GradCache<F::GradSliceEval>>,
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

struct FloatCache<E: BulkEvaluator> {
    eval: ShapeBulkEval<E>,
    tape: ShapeTape<E::Tape>,
    x: Vec<f32>,
    y: Vec<f32>,
    z: Vec<f32>,
}

struct GradCache<E: BulkEvaluator> {
    eval: ShapeBulkEval<E>,
    tape: ShapeTape<E::Tape>,
    x: Vec<Grad>,
    y: Vec<Grad>,
    z: Vec<Grad>,
}

impl<F: Function + Clone> FidgetField<F> {
    /// Wraps a `fidget` shape as an `exedra_implicit::ScalarField`.
    ///
    /// The shape must depend only on the `x`, `y`, and `z` axes.
    pub fn new(shape: Shape<F>) -> Result<Self, FidgetFieldError> {
        reject_extra_vars(&shape)?;
        let interval = IntervalCache {
            eval: Shape::<F>::new_interval_eval(),
            tape: shape.ez_interval_tape(),
        };
        let float = FloatCache {
            eval: Shape::<F>::new_float_slice_eval(),
            tape: shape.ez_float_slice_tape(),
            x: Vec::new(),
            y: Vec::new(),
            z: Vec::new(),
        };
        let grad = GradCache {
            eval: Shape::<F>::new_grad_slice_eval(),
            tape: shape.ez_grad_slice_tape(),
            x: Vec::new(),
            y: Vec::new(),
            z: Vec::new(),
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
    pub const fn shape(&self) -> &Shape<F> {
        &self.shape
    }

    /// Returns a smaller field equal to this one throughout `region`, with
    /// the branches of `min` and `max` that never win there pruned, or
    /// `None` when nothing can be pruned.
    #[must_use]
    pub fn specialize(&self, region: &Aabb) -> Option<Self> {
        if !self.shape.inner().can_simplify() {
            return None;
        }

        let trace = {
            let mut cache = lock_unpoison(&self.interval);
            let [x, y, z] = intervals(region);
            let tape = cache.tape.clone();
            match cache.eval.eval(&tape, x, y, z) {
                Ok((_interval, Some(trace))) => trace.clone(),
                Ok((_, None)) | Err(_) => return None,
            }
        };

        let shape = self.shape.ez_simplify(&trace).ok()?;
        if shape.size() >= self.shape.size() {
            return None;
        }

        Self::new(shape).ok()
    }
}

impl<F: Function + Clone> fmt::Debug for FidgetField<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FidgetField")
            .field("size", &self.shape.size())
            .field("can_simplify", &self.shape.inner().can_simplify())
            .finish_non_exhaustive()
    }
}

impl<F: Function + Clone> TryFrom<Shape<F>> for FidgetField<F> {
    type Error = FidgetFieldError;

    fn try_from(shape: Shape<F>) -> Result<Self, Self::Error> {
        Self::new(shape)
    }
}

impl<F: Function + Clone> ScalarField for FidgetField<F> {
    /// Fidget's single-precision interval bounds over `region` rounded
    /// outward, one step of `f32` further on each side.
    fn eval_interval(&self, region: &Aabb) -> exedra_implicit::Interval {
        let mut cache = lock_unpoison(&self.interval);
        let [x, y, z] = intervals(region);
        let IntervalCache { eval, tape } = &mut *cache;
        match eval.eval(tape, x, y, z) {
            Ok((interval, _trace)) => exedra_implicit::Interval::new(
                f64::from(interval.lower().next_down()),
                f64::from(interval.upper().next_up()),
            ),
            Err(_error) => exedra_implicit::Interval::ENTIRE,
        }
    }

    fn eval_points(&self, points: &[[f64; 3]], out: &mut [f64]) {
        assert_same_len(points.len(), out.len(), "point/value");
        if points.is_empty() {
            return;
        }
        let mut cache = lock_unpoison(&self.float);
        {
            let FloatCache { x, y, z, .. } = &mut *cache;
            split_points_into(points, x, y, z);
        }
        let FloatCache {
            eval,
            tape,
            x,
            y,
            z,
        } = &mut *cache;
        match eval.eval(tape, x, y, z) {
            Ok(values) => {
                for (slot, value) in out.iter_mut().zip(values) {
                    *slot = f64::from(*value);
                }
            }
            Err(_error) => out.fill(f64::NAN),
        }
    }

    fn eval_gradients(&self, points: &[[f64; 3]], out: &mut [Dual]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        if points.is_empty() {
            return;
        }
        let mut cache = lock_unpoison(&self.grad);
        {
            let GradCache { x, y, z, .. } = &mut *cache;
            split_grad_points_into(points, x, y, z);
        }
        let GradCache {
            eval,
            tape,
            x,
            y,
            z,
        } = &mut *cache;
        match eval.eval(tape, x, y, z) {
            Ok(values) => {
                for (slot, grad) in out.iter_mut().zip(values.iter()) {
                    *slot = Dual {
                        value: f64::from(grad.v),
                        gradient: [grad.dx, grad.dy, grad.dz].map(f64::from),
                    };
                }
            }
            Err(_error) => {
                out.fill(Dual {
                    value: f64::NAN,
                    gradient: [f64::NAN; 3],
                });
            }
        }
    }
}

/// The single-precision intervals covering `region`, rounded outward.
fn intervals(region: &Aabb) -> [Interval; 3] {
    core::array::from_fn(|axis| Interval::from([down(region.min[axis]), up(region.max[axis])]))
}

/// The greatest `f32` at most `value`.
#[expect(clippy::cast_possible_truncation, reason = "rounded to the side asked")]
fn down(value: f64) -> f32 {
    let near = value as f32;
    if f64::from(near) > value {
        near.next_down()
    } else {
        near
    }
}

/// The least `f32` at least `value`.
#[expect(clippy::cast_possible_truncation, reason = "rounded to the side asked")]
fn up(value: f64) -> f32 {
    let near = value as f32;
    if f64::from(near) < value {
        near.next_up()
    } else {
        near
    }
}

fn reject_extra_vars<F: Function>(shape: &Shape<F>) -> Result<(), FidgetFieldError> {
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

/// Fidget evaluates points in single precision.
#[expect(clippy::cast_possible_truncation, reason = "fidget evaluates in f32")]
fn split_points_into(points: &[[f64; 3]], x: &mut Vec<f32>, y: &mut Vec<f32>, z: &mut Vec<f32>) {
    x.clear();
    y.clear();
    z.clear();
    x.reserve(points.len());
    y.reserve(points.len());
    z.reserve(points.len());
    for point in points {
        x.push(point[0] as f32);
        y.push(point[1] as f32);
        z.push(point[2] as f32);
    }
}

#[expect(clippy::cast_possible_truncation, reason = "fidget evaluates in f32")]
fn split_grad_points_into(
    points: &[[f64; 3]],
    x: &mut Vec<Grad>,
    y: &mut Vec<Grad>,
    z: &mut Vec<Grad>,
) {
    x.clear();
    y.clear();
    z.clear();
    x.reserve(points.len());
    y.reserve(points.len());
    z.reserve(points.len());
    for point in points {
        x.push(Grad::new(point[0] as f32, 1.0, 0.0, 0.0));
        y.push(Grad::new(point[1] as f32, 0.0, 1.0, 0.0));
        z.push(Grad::new(point[2] as f32, 0.0, 0.0, 1.0));
    }
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
    use exedra_implicit::{Aabb, Dual, ScalarField};
    use exedra_isosurface::{DelaunayParams, DelaunayResult, delaunay_mesh};
    use fidget::{context::Tree, var::Var, vm::VmShape};

    use super::{FidgetField, VmField};

    fn sphere() -> VmField {
        let (x, y, z) = (Tree::x(), Tree::y(), Tree::z());
        let sphere = (x.square() + y.square() + z.square()).sqrt() - 1.0;
        VmField::new(VmShape::from(sphere)).expect("axis-only shape")
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

    fn mesh(field: &VmField, bounds: Aabb, cell_size: f64) -> DelaunayResult {
        let result = delaunay_mesh(field, &DelaunayParams::new(bounds, cell_size))
            .expect("valid parameters");
        assert!(result.report.is_complete(), "{:?}", result.report);
        let mesh = result.mesh().expect("a valid mesh");
        assert!(mesh.validate_deep().is_empty());
        assert!(
            mesh.boundary_loops()
                .expect("boundary traversal")
                .is_empty()
        );
        result
    }

    #[test]
    fn vm_field_meshes_a_sphere_on_its_surface() {
        let field = sphere();
        let bounds = Aabb::new([-1.5; 3], [1.5; 3]).expect("bounds");
        let result = mesh(&field, bounds, 0.2);

        let mut values = vec![0.0; result.positions.len()];
        field.eval_points(&result.positions, &mut values);
        // Surface points are searched to within a small part of a cell.
        assert!(values.iter().all(|value| value.abs() < 0.2 / 64.0));
    }

    #[test]
    fn vm_field_gradients_seed_basis_derivatives() {
        let field = VmField::new(VmShape::from(Tree::x())).expect("axis-only shape");
        let mut gradients = [Dual::default()];
        field.eval_gradients(&[[2.5, -3.0, 4.0]], &mut gradients);

        assert_eq!(
            gradients[0],
            Dual {
                value: 2.5,
                gradient: [1.0, 0.0, 0.0]
            }
        );
    }

    #[test]
    fn vm_field_intervals_contain_values_at_f64_corners() {
        let field = sphere();
        // Corners no `f32` represents exactly.
        let region = Aabb::new([0.1, 0.2, 0.3], [0.7, 0.9, 1.1]).expect("bounds");
        let bounds = field.eval_interval(&region);
        let mut values = [0.0; 8];
        field.eval_points(&region.corners(), &mut values);
        assert!(values.iter().all(|&value| bounds.contains(value)));
    }

    #[test]
    fn vm_field_reuses_scratch_across_varying_batch_sizes() {
        let field = sphere();

        let small = [[0.125, 0.0, 0.0], [0.5, 0.25, -0.75]];
        let large = [
            [-1.0, 0.0, 0.0],
            [-0.5, 0.5, 0.0],
            [0.125, -0.25, 0.375],
            [0.5, -0.25, 0.75],
            [1.0, 0.0, 0.0],
        ];
        let finite = |rows: &[Dual]| {
            rows.iter()
                .all(|row| row.value.is_finite() && row.gradient.iter().all(|g| g.is_finite()))
        };

        let mut point_values = [0.0; 5];
        let mut gradients = [Dual::default(); 5];
        field.eval_points(&large, &mut point_values);
        field.eval_gradients(&large, &mut gradients);
        assert!(point_values.iter().all(|value| value.is_finite()));
        assert!(finite(&gradients));

        let mut small_values = [0.0; 2];
        let mut small_gradients = [Dual::default(); 2];
        field.eval_points(&small, &mut small_values);
        field.eval_gradients(&small, &mut small_gradients);
        assert!(small_values.iter().all(|value| value.is_finite()));
        assert!(finite(&small_gradients));

        field.eval_points(&large, &mut point_values);
        field.eval_gradients(&large, &mut gradients);
        assert!(point_values.iter().all(|value| value.is_finite()));
        assert!(finite(&gradients));
    }

    #[test]
    fn vm_field_union_meshes_closed() {
        let left = box_tree(-0.9, [0.35, 0.35, 0.35]);
        let right = box_tree(0.9, [0.35, 0.45, 0.35]);
        let field = VmField::new(VmShape::from(left.min(right))).expect("axis-only shape");
        let bounds = Aabb::new([-1.6, -1.0, -1.0], [1.6, 1.0, 1.0]).expect("bounds");

        let result = mesh(&field, bounds, 0.1);
        assert!(!result.triangles.is_empty());
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
