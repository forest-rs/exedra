---
id: cam-mn4h
title: Face-edit winding/orientation policy
status: closed
deps: []
links: []
created: 2026-03-04T12:54:50Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Face-edit winding/orientation policy

Define and enforce a consistent winding/orientation contract for face-edit operators (extrude/inset and future edits) across open and closed manifold contexts.

## Design

Current operators now adaptively choose winding to avoid non-manifold failures, but the policy is implicit. Add an explicit contract and shared helper(s) that derive required loop orientation from boundary reuse direction, with deterministic behavior and diagnostics when ambiguous. Ensure cap/frame orientation is topology-derived rather than heuristic.

## Acceptance Criteria

- Shared internal winding helper for face-edit ops
- Extrude/inset use the helper (no duplicated ad-hoc logic)
- Regression tests cover closed-box face extrude and extrude->inset chain
- Rustdoc/manual section states winding contract and assumptions
