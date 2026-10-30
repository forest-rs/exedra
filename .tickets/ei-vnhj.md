---
id: ei-vnhj
status: closed
deps: []
links: []
type: feature
priority: 2
---
# Make manifold dual-contouring scope explicit

## Problem

The phase-1 adaptive dual contourer does not guarantee a two-manifold mesh for
every ambiguous cell configuration. Public documentation must not imply that it
does.

## Fence

`exedra_isosurface` owns field sampling, cell topology, QEF evidence, and
field-to-mesh extraction; it does not promise universal field topology.

## Acceptance

- The supported phase-1 envelope and bounded failures are documented.
- Manifold cell configurations use deterministic documented rules.
- Seeded CSG-field cases produce deep-valid manifold meshes.
- Golden changes are deliberate and oracle runs occur before and after.

## Notes

**2026-10-30T14:40:20Z**

Superseded: dual contouring was retired with the f32 fields. Implicit fields are meshed by restricted Delaunay refinement in exedra_isosurface, which enforces a disk around every vertex and reports any vertex left without one (DelaunayReport::disk_failures); a complete report means a closed manifold. The oracle's fixed suite checks seams on deep-valid meshes.
