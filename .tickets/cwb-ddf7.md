---
id: cwb-ddf7
status: in_progress
deps: []
links: []
type: feature
priority: 2
---
# Complete deterministic provenance inspection

## Problem

Existing payloads expose provenance tables, but inspection does not yet follow
one named assembly element through its instance path, part, recipe node,
feature, source reference, fidelity, region, material, and diagnostics.

## Fence

The web bridge and viewer own deterministic inspection payloads and
presentation; they do not own example geometry, kernel semantics, or a generic
plugin system.

## Addressable session extension

Retain the `panel_trio` assembly behind a Wasm `InspectionSession`, then let a
picked face identify its exact instance address and constructive material slot.
The bridge owns this session and its JSON presentation. Exedra continues to own
assembly state, material policy, and typed addressed operations; this work does
not extract a generic tooling schema.

## Acceptance

- Dependency direction is documented before implementation.
- A named selection yields deterministic inspect bytes containing the full
  provenance and diagnostic chain.
- Group order, instance identity, and coordinate convention remain stable.
- Missing provenance is displayed as missing.
- Existing scenarios remain byte-deterministic.
- Repeated session snapshots observe one explicit runtime space and revision.
- A picked material-bearing face yields an instance-address/material-slot pair.
