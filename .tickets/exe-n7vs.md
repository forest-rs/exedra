---
id: exe-n7vs
status: closed
deps: []
links: []
type: bug
priority: 1
---
# Return typed refusal for non-manifold Boolean contacts

## Problem

Union of solids sharing exactly one edge can surface an internal
`Build(NonManifoldEdge { count: 4 })` failure. Refusal is geometrically valid,
but the public result should classify the unsupported contact.

## Fence

The Boolean pipeline owns geometric classification and typed errors; it does
not reinterpret a non-manifold result as a valid mesh.

## Acceptance

- Shared-edge and vertex-contact outcomes return a typed geometric refusal, or
  a documented regularized multi-shell result.
- They never surface as an internal build failure.
- The oracle reports a named refusal category.
- Existing valid unions remain deterministic and unchanged.

## Notes

**2026-08-24T02:16:13Z**

Implemented a typed NonManifoldContact refusal for Union when otherwise-manifold operands share an edge whose selected boundary has four incident faces. The stitcher distinguishes this from an internal rebuild defect using the failing graph edge, original input-edge provenance, per-operand selected-face counts, and absence of an adjacent positive-area coplanar contact; unproven failures remain BooleanError::Build. Isolated vertex contacts now avoid contact-point ray samples and retain shell-local vertex identities, producing two valid closed shells. Public API migration: BooleanError and BooleanFailureKind gain additive variants on already non-exhaustive enums; wildcard handlers remain source-compatible, while callers may explicitly report the new condition. The oracle now reports non_manifold_contact and the former 15 build_failure skips in a 240-case adversarial sweep all move to that category with zero mesh or field disagreements. Focused validation passed: all-feature exedra tests, boolean_oracle tests, exedra libm no_std check, and clippy for exedra plus boolean_oracle with warnings denied.
