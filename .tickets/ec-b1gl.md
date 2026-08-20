---
id: ec-b1gl
status: open
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
