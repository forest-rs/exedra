---
id: cam-vllm
status: closed
deps: []
links: []
created: 2026-03-08T04:01:25Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Add face poke operator and fluent step

Add edit.face.poke to split selected faces into triangle fans from a new center vertex, with typed output and MeshEdit support.

## Design

Implement poke as a Cambium face-edit operator over Exedra kernel ops. Compile/apply should canonicalize face selections, compute per-face centers, add one vertex per selected face, delete the source face, add triangle fan faces in deterministic loop order, propagate face region and authored attrs where sensible, and return typed output for created center vertices and created fan faces. Extend MeshEdit with a poke step for face selections.

## Acceptance Criteria

1) edit.face.poke is exported. 2) Typed output includes canonical source faces, created center vertices, and created fan faces. 3) MeshEdit supports poke on face selections. 4) Tests cover quad/ngon success, stale/non-canonical input, and manual-vs-fluent equivalence.

