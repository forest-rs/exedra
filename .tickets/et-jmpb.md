---
id: et-jmpb
status: open
deps: []
links: []
type: feature
priority: 2
---
# Add an exact deterministic incircle predicate

## Problem

Constrained-Delaunay legalization needs an exact incircle sign predicate. Naive
degree-four expansions can overflow or underflow near the supported coordinate
envelope.

## Fence

`exedra_triangulate` owns deterministic planar predicates and strategy seams; it
does not add dependencies, unsafe code, or mesh-kernel ownership.

## Acceptance

- Filtered and exact paths correctly classify clear, cocircular, degenerate,
  one-ulp, permuted, oriented, scaled, and exponent-extreme inputs.
- Evaluated paths expose deterministic counters.
- The implementation remains `no_std` plus `alloc`, dependency-free, and safe.
- The predicate wind tunnel remains deterministic.
