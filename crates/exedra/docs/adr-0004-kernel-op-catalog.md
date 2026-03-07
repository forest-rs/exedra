# ADR-0004: Exedra Kernel Operations Live in `exedra::op`

**Status:** Accepted  
**Date:** 2026-03-08

## Context

Exedra's topology-editing surface currently lives mostly on
[`EditSession`](crate::EditSession). That makes one type act as:

1. edit host,
2. bookkeeping/change-set owner,
3. low-level topology helper namespace,
4. public catalog of kernel edits.

This has become hard to navigate and obscures the architectural boundary
between Exedra kernel edits and Cambium workflow operators.

## Decision

Add a first-class `exedra::op` module that owns the public kernel operation
catalog.

The boundary is:

- `session/*` owns eager edit hosting, bookkeeping, dirty/change tracking,
  cache invalidation, and low-level mutation helpers.
- `op/*` owns public kernel mutation functions over `&mut EditSession`.
- `cambium` owns compile/apply plans, diagnostics, reports, artifacts, and
  user-facing workflow operators.

Exedra ops are namespaced free functions such as `op::split_edge(...)` and
`op::delete_faces(...)`. We intentionally do **not** introduce a trait
hierarchy, runner layer, or command-object representation in Exedra.

## Rationale

- Makes kernel edits explicit and discoverable without turning `EditSession`
  into the public operation catalog.
- Keeps bookkeeping and mutation plumbing where it belongs: on the session.
- Gives Cambium a cleaner composition seam for future modeling operators.
- Avoids the allocation/verbosity cost of struct-wrapped command values for
  borrowed slice inputs and simple authored writes.

## Consequences

- Public mutation entry points live in `exedra::op::*`.
- `MeshBuilder` remains the construction API; `Mesh` is no longer a mutation
  convenience surface for topology edits.
- `EditSession` is the eager edit host and internal plumbing seam, not a
  second public mutation catalog.
- Topology operation bodies live in `op/*`; `session/*` retains bookkeeping,
  cache invalidation, propagation helpers, and other mutation plumbing.
