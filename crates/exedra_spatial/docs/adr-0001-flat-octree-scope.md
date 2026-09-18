# ADR-0001: Flat Octree Scope

- Status: Accepted
- Date: 2026-03-24
- Owners: Exedra implicit-surface maintainers

## Context

The implicit-surface branch needs spatial indexing for octree-driven sampling,
but the spatial layer should stay reusable outside isosurface extraction.

## Decision

`exedra_spatial` owns:

- `Aabb`,
- a flat adaptive octree with deterministic child ordering,
- visitor-driven construction,
- deterministic traversal modes,
- spatial neighbor queries over octree cells.

It does not own:

- scalar-field evaluation,
- Hermite sampling,
- QEF solving,
- mesh extraction.

## Consequences

### Bounded, fallible construction

The octree owns stopping traversal on a visitor failure and enforcing a stored
cell limit before allocating the root or an eight-child batch. A visitor's
payload and work accounting remain caller-owned. The same checks apply to
initial construction and subsequent refinement; failure must stop traversal
immediately, without visiting remaining siblings.

Refinement failure restores the original leaf and truncates newly appended
cells. Visitor side effects are not rolled back. Errors carry the failing cell's
spatial bounds and the stored-cell count at failure; IDs created during a failed
refinement do not designate surviving cells. Callers must reset or discard
visitor state associated with that failed refinement.

Migration: `OctreeVisitor` gains an associated `Error` and returns `Result` from
both callbacks. Infallible visitors use `core::convert::Infallible` and `Ok(...)`.
`Octree::build` and `refine_leaf` take an optional stored-cell limit and return
typed results. `None` leaves the caller's cell count unrestricted, subject to
the arena's ID capacity. These are storage-slot and traversal guarantees, not
process-memory or wall-clock guarantees for arbitrary visitor implementations.

`leaf_ids()` now returns an allocation-free iterator. Replace `.len()` with
`.count()`, remove `.iter()` / `.into_iter()`, or explicitly collect when a
retained list is needed.

Extraction-specific limits and evidence remain in `exedra_isosurface`, as
described in its ADR-0002; the spatial crate does not acquire field semantics.

### Architectural consequences

Positive:

- keeps spatial indexing reusable for later ray, culling, and proximity work,
- keeps the implicit extraction stack layered,
- preserves `no_std` viability for the spatial root.

Tradeoffs:

- some octree conveniences needed by later extractors will arrive incrementally,
- neighbor queries are intentionally correctness-first before performance-first.
