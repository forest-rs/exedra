---
id: exe-fc97
title: Validation (validate_fast and validate_deep)
status: closed
deps: [exe-cbv1]
links: [cam-0711]
created: 2026-03-03T05:35:11Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Validation (validate_fast and validate_deep)

Implement mesh validation routines. Validation is mandatory — debug-by-default posture. Provides both a cheap fast check and a thorough deep check with explanatory reporting.

## Design

validate_fast() — cheap checks:
- twin(twin(h)) == h for all half-edges
- face loop closedness (next^degree == start)
- vertex.out points to a valid half-edge
- No generation mismatches in topology references
- Attribute layer capacities match arena capacities (dense layers)

validate_deep() — graph walks (partial in v0.1):
- Full face loop enumeration matches cached degree
- Vertex star walk is well-defined and terminates
- All faces are reachable from their half-edges
- Boundary half-edges are attached to OUTSIDE
- Manifold checks (each edge has exactly 2 half-edges)
- "explain invalidity" reporting: return structured errors, not panics

Return type should be a Vec of structured validation errors, not bool. Each error identifies the offending element(s) and the violated invariant.

validate_deep may be incomplete in v0.1 (document what is and is not checked).

## Acceptance Criteria

- validate_fast() exists and checks core invariants cheaply
- validate_deep() exists with at least partial coverage
- Both return structured error lists (not bool, not panic)
- Errors identify offending elements and violated invariants
- Coverage of: twin symmetry, face loop closure, vertex.out validity, layer capacities
- Unit tests: valid mesh passes, intentionally broken mesh reports correct errors


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/03_determinism_contract.md, crates/exedra/docs/briefs/12_half_edge_vs_alternatives.md

**2026-03-03T06:27:28Z**

Design brief: crates/exedra/docs/briefs/15_validation_invariants_and_error_reporting.md

**2026-03-03T11:24:23Z**

Implemented structured mesh validation with ValidationError and two entrypoints: validate_fast() (cheap core checks) and validate_deep() (graph-walk checks, partial v0.1). Added fast checks for twin involution, face loop closedness by cached degree, topology reference validity, and dense/domain capacity mismatches. Added deep checks for face degree consistency, foreign half-edges in loops, vertex-star closure, OUTSIDE boundary invariants, and undirected edge multiplicity. Added Attributes::dense_capacity_mismatches helper. Added tests for valid mesh passes and intentionally corrupted meshes producing expected errors. Re-exported ValidationError at crate root. Validation run: cargo fmt --all; cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features; cargo doc --no-deps.
