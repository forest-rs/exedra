---
id: exe-imti
status: open
deps: [exe-ikdf]
links: []
created: 2026-03-03T05:46:51Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# Mesh splitting along intersection graph

Split mesh faces and edges along intersection polylines. After this stage, intersection curves lie exactly on mesh edges, enabling clean patch classification.

## Design

For each mesh (A and B):
- Walk intersection polylines
- Split faces crossed by polyline segments
- Insert new vertices at intersection points
- Insert new edges along polyline paths
- Result: both meshes now share the intersection curve as explicit edges

Attribute propagation during splits uses PropagatePolicy (or boolean-specific defaults).
Must maintain mesh validity throughout.

## Acceptance Criteria

- Faces split along intersection curves
- New vertices and edges align with intersection geometry
- Mesh validity maintained (validate_fast passes)
- Attribute propagation handled
- Deterministic result

