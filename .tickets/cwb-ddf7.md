---
id: cwb-ddf7
status: open
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

## Acceptance

- Dependency direction is documented before implementation.
- A named selection yields deterministic inspect bytes containing the full
  provenance and diagnostic chain.
- Group order, instance identity, and coordinate convention remain stable.
- Missing provenance is displayed as missing.
- Existing scenarios remain byte-deterministic.
