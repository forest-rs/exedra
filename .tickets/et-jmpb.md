---
id: et-jmpb
status: closed
deps: []
links: [et-o66p, et-c9eu, exe-ccxf]
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
- Evaluated paths expose deterministic per-call diagnostics without global
  counters.
- The implementation remains `no_std` plus `alloc`, dependency-free, and safe.
- The predicate wind tunnel remains deterministic.

## Notes

- Added `incircle`/`incircle_evaluated`. Clear queries use the
  standard error-bound filter; inconclusive finite queries use a fixed
  132-limb exact dyadic sum of the 48 homogeneous determinant monomials.
- The filter deliberately defers when a nonzero intermediate product is not
  normal. An independent broad-exponent oracle exposed a case where an
  underflowed cross term was later amplified by a huge lift and reversed the
  apparent sign; that exact bit pattern is now a regression test.
- No dependency, allocation, unsafe code, or default-strategy change was
  introduced. The crate ADR records the degree-four bit bound.
- Validation: 53 crate tests with all/default-free features, 9 wind-tunnel
  tests, clippy with warnings denied, rustdoc, fmt, Taplo, typos, diff check,
  10,000 direct exact-path integer-oracle cases, and 100,000 external Python
  `Fraction` cases all passed. The final release quick run measured roughly
  11 ns for the filter and 325 ns average for the exact path on this machine.
