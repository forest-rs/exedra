# Brief: Why half-edge (vs winged-edge, DCEL, adjacency lists) for Exedra

## Decision
Use a **pointerless polygonal half-edge** representation as Exedra’s canonical topology model (with stable IDs and an OUTSIDE face boundary model).

## Why
Half-edge is a good fit for Exedra’s goals: editing, robust traversal, deterministic extraction, and booleans.

- **Local, explicit adjacency.** `next/twin/face/to` makes the common traversals cheap and predictable.
- **Face-loop iteration is first-class.** Many operations (triangulation, attribute propagation, boolean splitting) are naturally expressed as face-loop walks.
- **Boundary modeling is clean.** With an OUTSIDE face, boundaries become ordinary face adjacency rather than special cases.
- **Debuggability.** The topology graph is explicit and inspectable; validation can assert concrete invariants.
- **Cache locality.** A pointerless arena layout keeps topology in contiguous arrays.

## Alternatives considered
- **Winged-edge.** Provides direct access to incident faces/edges per edge, but is heavier to maintain under edits and less natural for face-loop iteration.
- **DCEL (doubly connected edge list).** Similar family; often oriented toward planar subdivisions. In practice it converges toward half-edge; differences are mostly naming and typical invariants.
- **Adjacency lists / indexed meshes.** Great for static rendering, but poor for topology edits, seam-aware attributes, and boolean pipelines (you end up rebuilding connectivity constantly).

## Implications
- Exedra invests in strong **validation** and clear **edit primitives** to preserve invariants.
- Stable deterministic face-loop ordering is straightforward (arena order + `Face.edge` walk).
- Attribute domains (especially corner domain) map cleanly: `CornerId == HalfEdgeId`.

## Non-goals / deferrals
- Supporting multiple canonical topology backends in v1.0. If needed later, it should be layered behind stable public APIs.
