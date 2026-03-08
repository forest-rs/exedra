---
id: cam-3ekt
status: closed
deps: [exe-nite]
links: []
created: 2026-03-08T02:08:17Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Add dissolve vertices operator and fluent step

Wrap Exedra dissolve_vertices in Cambium with typed output and MeshEdit support.

## Design

Expose edit.dissolve.vertices over canonical vertex selections. Compile/apply should canonicalize vertex selections, call exedra::op::dissolve_vertices, map kernel errors to OpError diagnostics, and return typed output containing the dissolved vertex selection and affected face selection for chaining. Extend MeshEdit with a dissolve_vertices step for vertex selections, with compile-time validation rather than apply-time surprises.

## Acceptance Criteria

1) edit.dissolve.vertices is exported. 2) Typed output includes canonical dissolved vertices and resulting affected faces. 3) MeshEdit supports dissolve_vertices on vertex selections. 4) Tests cover success, stale/non-canonical input, and manual-vs-fluent equivalence.

