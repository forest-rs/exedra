# Brief: Corner-domain attributes and render extraction splitting

## Decision
Store UVs, shading normals (and later tangents) in the **corner domain** (`CornerId == HalfEdgeId`). Keep topology (vertex sharing) independent from shading/parameterization. When producing a renderable TriMesh, **split render vertices** when corner-domain attributes differ.

## Why
Many discontinuities are *shading* discontinuities, not *topological* ones:

- UV seams: same topological vertex, different UVs per face corner
- hard edges: same topological vertex, different shading normals per corner (or per face group)
- tangent discontinuities: same story

Encoding these by splitting topology destroys stable modeling topology, increases churn, and complicates caching and booleans. Corner-domain attributes preserve a stable modeling mesh while still producing correct rendering results.

## Alternatives considered
- **Topological vertex splits** to represent seams/hard edges: simpler extraction, but breaks stable topology, increases element churn, and makes edit propagation and booleans nastier.
- **“Render mesh is canonical”**: makes modeling operations awkward (you’re constantly editing a representation that wants duplication).

## Implications
- Exedra extraction must define a deterministic **render-vertex key** (position source + enabled corner attributes).
- Edit primitives must define **attribute propagation rules** for corner-domain layers.
- Derived corner normals are recomputed from dirtiness; authored overrides are explicit and propagate via policy.
- Exedra Ops operators can create seams/hard edges by writing corner data, without changing topology.

## Non-goals / deferrals
- Full UV unwrapping (charting/packing) is not required; corner storage remains the right substrate when you add it later.
