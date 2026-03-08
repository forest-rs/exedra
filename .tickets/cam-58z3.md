---
id: cam-58z3
status: closed
deps: [exe-uivs]
links: []
created: 2026-03-08T01:27:02Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Dissolve edges operator

Add a Cambium edit operator wrapper for Exedra dissolve_edges with deterministic canonicalization, typed output, and fluent MeshEdit support.

## Design

Expose edit.dissolve.edges over canonical edge selections. Compile/apply should canonicalize edge selections, call exedra::op::dissolve_edges, map kernel errors to OpError diagnostics, and return typed output containing the dissolved edge selection and affected face selection for chaining. Extend MeshEdit with a dissolve_edges step for edge selections.

## Acceptance Criteria

1) Delete-free dissolve operator is exported as edit.dissolve.edges. 2) Typed output includes canonical dissolved edges and resulting affected faces. 3) MeshEdit supports dissolve_edges on edge selections. 4) Tests cover success, stale/non-canonical input, and manual-vs-fluent equivalence.

