---
id: exe-uxff
title: split_face diagonal edge propagation policy completeness
status: open
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
