---
id: ec-b1gl
status: closed
deps: []
links: []
type: feature
priority: 1
---
# Compose mirror-safe constructive recipes

## Problem

Assembly patterns need mirrored recipe-backed parts without reflective instance
placements or evaluation-time mesh baking.

## Fence

`exedra_constructive` owns immutable recipe composition and winding-correct
mirror semantics; it does not own assembly naming, repetition, distribution, or
binding.

## Acceptance

- Mirroring uses immutable recipe composition with deterministic fingerprints
  and correct winding.
- Recipe tables, provenance, and source identity are preserved.
- The original recipe remains unchanged and invalid planes fail typed.
- No reflective instance placement or evaluated-mesh baking is introduced.

## Notes

**2026-08-29T19:47:27Z**

Implemented immutable Recipe::mirrored composition by cloning the frozen recipe tables and appending one unbound Mirror root. Existing node/table ids, child fingerprints, source/material/issue bindings, imported-mesh content, and evaluated source maps remain stable; the original recipe is untouched and repeated composition is deterministic. Plane validation now rejects finite-but-unnormalizable underflow, overflow, and distance-scaling cases before evaluation. The evaluator retains the prior arithmetic for valid planes and repairs winding through the existing constructive Mirror path, so no EVAL_SCHEMA_VERSION bump, reflective assembly/IR instance placement, or evaluated-mesh baking was introduced. Updated the existing constructive scope ADR, README, integration guide, rustdoc, and regression tests. Validation passed: typos; taplo fmt --check; cargo fmt --all -- --check; cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features; cargo doc --workspace --all-features --no-deps.
