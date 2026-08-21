---
id: exe-8kli
status: closed
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

## Notes

**2026-08-21T05:50:28Z**

Implemented an atomic sharp-edge rounding apply path. Rounding now rewrites a cloned mesh, preflights each eager face addition for one-in/one-out OUTSIDE boundary continuation, and commits only after the complete rewrite succeeds. Cleaned drilled rims that create a temporary pinched boundary return RoundError::UnsupportedTopology instead of panicking; all errors preserve topology, attributes, arena state, and revision. Regressions cover unrotated and 45-degree transformed Boolean drill outputs after default seam cleanup. Updated ADR 0011 with the root cause, taxonomy, atomicity invariant, and mesh-clone cost. Validation: cargo nextest run --workspace --all-features (977 passed); cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo doc --workspace --no-deps --all-features; cargo check -p exedra --no-default-features --features libm; cargo fmt --all -- --check; taplo fmt --check; typos; git diff --check.
