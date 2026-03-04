---
id: exe-vzlq
title: Fidget adapter crate (exedra_fidget)
status: open
deps: [exe-2r7w, exe-gosk]
links: []
created: 2026-03-04T07:18:12Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# Fidget adapter crate (exedra_fidget)

Thin adapter crate implementing ScalarField (and extension traits) for fidget's Shape<F>. This is the bridge that lets fidget's JIT-compiled SDF evaluation drive the exedra DC pipeline.

## Design

Core implementation:

```rust
impl<F: Function> ScalarField for FidgetField<F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        // Use self.shape.interval_tape() + IntervalEval
        // Convert fidget::Interval → [f32; 2]
    }
    
    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        // Use self.shape.float_slice_tape() + FloatSliceEval
        // Bulk evaluation, SIMD-friendly
    }
    
    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        // Use self.shape.grad_slice_tape() + GradSliceEval
        // Convert fidget::Grad → [f32; 4]
    }
}
```

SpecializableField implementation:

```rust
impl<F: Function> SpecializableField for FidgetField<F> {
    type Specialized = FidgetField<F>;
    
    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized> {
        // Eval interval to get trace
        // Call Function::simplify() with trace
        // Return new FidgetField wrapping simplified function
        // Return None if simplification didn't reduce the tape
    }
}
```

ProvenanceField implementation:
- Requires access to fidget's CSG tree structure and the Choice (Left/Right/Both) info from interval evaluation.
- May require upstream fidget changes or use of internal APIs.
- Could be deferred to a later phase if fidget doesn't expose enough provenance info publicly.

Tape management:
- FidgetField owns tape storage for reuse across evaluations.
- Tape recycling via fidget's Storage types to minimize allocation.
- Thread-local evaluator caching if parallel octree traversal is used.

Dependencies:
- fidget-core (required): Function trait, types, VM evaluator
- fidget-jit (optional feature): JIT-compiled evaluation for performance
- exedra_isosurface: ScalarField trait

Wrapper type:

```rust
pub struct FidgetField<F: Function> {
    shape: Shape<F>,
    tape_storage: F::TapeStorage,
    // Evaluator caches
    interval_eval: F::IntervalEval,
    float_eval: F::FloatSliceEval,
    grad_eval: F::GradSliceEval,
}

impl<F: Function> FidgetField<F> {
    pub fn new(shape: Shape<F>) -> Self { ... }
}

// Convenience alias
pub type JitField = FidgetField<fidget_jit::JitFunction>;
pub type VmField = FidgetField<fidget_core::vm::VmFunction>;
```

## Acceptance Criteria

- impl ScalarField for FidgetField<F> where F: Function
- impl SpecializableField with tape simplification
- Tape storage reuse across evaluations
- Works with both VmFunction and JitFunction backends
- Integration test: fidget sphere SDF → DC → exedra Mesh → validate_deep
- Integration test: fidget CSG union → DC → mesh with correct sharp features
- Performance benchmark vs fidget's built-in mesher on equivalent inputs

