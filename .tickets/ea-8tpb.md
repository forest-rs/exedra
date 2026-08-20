---
id: ea-8tpb
status: open
deps: []
links: []
type: bug
priority: 1
---
# Preserve constructive diagnostics during assembly compilation

## Problem

Assembly compilation can evaluate a recipe with zero bodies and an
`eval.unimplemented` diagnostic, then silently omit the instance during
flattening.

## Fence

`exedra_assembly` owns compilation-result visibility and deterministic part
diagnostics; it does not own constructive evaluation semantics or viewer policy.

## Acceptance

- Zero-body error or unimplemented evaluations cannot compile as silent
  success.
- Callers can inspect deterministic reports or receive a typed compile error.
- Warning retention and cache-hit report behavior are defined.
- A regression covers the omitted instance; public result changes include a
  migration note.
