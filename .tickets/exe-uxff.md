---
id: exe-uxff
title: split_face diagonal edge propagation policy completeness
status: closed
deps: [exe-0a9w]
links: []
created: 2026-03-04T06:44:21Z
type: task
priority: P2
assignee: Bruce Mitchener
tags: [v0.5]
---
# split_face diagonal edge propagation policy completeness

Clarify/extend edge propagation for split_face so callers can explicitly choose diagonal sharpness outcomes (for example force smooth, force inherit/source-driven, or explicit value). Current v0.1 behavior uses Inherit=>smooth and DecayOnSplit=>derived decay, which is safe but limited.

## Design

Introduce an explicit split-face edge policy mode (or per-kernel override) distinct from split-edge semantics. Keep deterministic behavior and preserve no_std constraints. Ensure rustdoc explains per-kernel policy semantics and defaults.

## Acceptance Criteria

1) API exposes explicit split_face diagonal edge propagation mode(s). 2) Default remains backward-compatible for v0.1 callers. 3) Tests cover each mode on authored sharp input. 4) Docs explain modeling vs subdivision use cases.

## Notes

**2026-05-04T16:53:08Z**

Added SplitFaceDiagonalEdgePropagation and PropagatePolicy::split_face_diagonal_edge_attr so split_face diagonal sharpness has explicit Smooth, Inherit, and DecayOnSplit modes. Default is FromEdgePolicy, which preserves v0.1 behavior for existing edge_attr-based callers: Inherit/Clear produce smooth diagonals and DecayOnSplit keeps the previous derived-decay behavior. Migration note: code constructing PropagatePolicy with exhaustive struct literals must add split_face_diagonal_edge_attr, usually SplitFaceDiagonalEdgePropagation::FromEdgePolicy or an explicit split-face mode; callers using PropagatePolicy::default() or ..Default remain source-compatible. Updated the edit propagation brief, manual, and API surface notes. No new ADR: this refines the existing edit propagation model captured in brief 13 rather than changing crate ownership or edit invariants. Validation: cargo fmt --all; cargo test -p exedra --all-features; cargo check -p cambium --all-features; cargo clippy -p exedra --all-targets --all-features -- -D warnings; cargo doc -p exedra --no-deps; typos crates/exedra/src crates/exedra/docs .tickets/exe-uxff.md; cargo fmt --all --check; git diff --check.
