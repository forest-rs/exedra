---
id: exe-8kli
status: open
deps: []
links: []
type: bug
priority: 1
---
# Make seam cleanup and edge rounding panic-free

## Problem

Boolean seam cleanup followed by sharp-edge rounding can panic in the outside
loop stitcher on drilled-block fixtures. Without cleanup, rounding returns a
typed clearance failure.

## Fence

Exedra owns topology-safe failure and rollback; it does not choose application
rounding policy or viewer behavior.

## Acceptance

- Rotated and unrotated fixtures succeed or return typed errors, never panic.
- Errors leave the mesh byte-identical.
- A regression covers cleanup followed by rounding.
- The error taxonomy and root cause are documented.
