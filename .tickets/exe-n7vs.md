---
id: exe-n7vs
status: open
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
