---
id: cwb-ddf7
status: closed
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

## Notes

**2026-08-27T06:04:39Z**

Implemented as two stacked changes. The first retains `panel_trio` behind a Wasm
`InspectionSession` with an explicit runtime space and revision while preserving
one-shot recipe scenarios. The second joins each picked assembly face through
its instance and producing node into the canonical instance-address/material-slot
pair required by a later typed material read. The bridge keeps JSON presentation
local and does not extract a generic tooling schema. Verified `cargo fmt`, Taplo,
typos, `cambium_web_bridge` tests and strict Clippy, warning-denied rustdoc,
`wasm32` check, `wasm-pack`/Vite production build, and dist smoke check. Direct
`npx tsc` remains unusable because the existing viewer lacks Three.js declaration
packages; the production Vite build passes.
