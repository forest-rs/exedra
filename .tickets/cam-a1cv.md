---
id: cam-a1cv
status: closed
deps: []
links: []
created: 2026-03-08T06:23:23Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Smooth selected faces via derived normals

Add a Cambium operator that clears existing corner normal overrides on selected faces and writes newly derived corner normals using explicit NormalParams. This is a calmer smoothing tool than requiring users to combine clear + average manually.

## Design

Input is a canonical face selection plus NormalParams. Compile canonicalizes and rejects stale faces. Apply derives corner normals with Mesh::derive_corner_normals, then writes the resulting normals into attr::CORNER_NORMAL_OVERRIDE for each selected face corner via exedra::op::set_corner_normal_override. Output returns the affected face set.

## Acceptance Criteria

- edit.normal.smooth exists
- Accepts explicit NormalParams
- Compile canonicalizes and rejects stale faces
- Writes current derived corner normals as authored overrides for selected faces
- MeshEdit fluent API supports the operator
- Tests cover smooth-on-cylinder style behavior and fluent equivalence

