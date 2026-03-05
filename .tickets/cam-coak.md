---
id: cam-coak
status: open
deps: [cam-mrwk, cam-gvmz, cam-inf3]
links: []
created: 2026-03-05T02:03:21Z
type: task
priority: P2
assignee: Bruce Mitchener
tags: [v0.5, api]
---
# Fluent MeshEdit SDK surface

Provide a calm fluent Cambium API layer over operators/runner for common modeling workflows.

## Design

Add MeshEdit-style builder chaining over operator compile/apply flow. Keep it as ergonomics layer over existing operator system and do not hide the lifecycle model.
Expected surface:
- `plan()` compiles accumulated intent into an EditPlan
- `preview()` applies the plan on a clone
- `apply()` applies in-place (and may compile internally if no explicit plan was requested)
Selections should support both generic bridge flows and explicit typed narrowing (for example `as_faces`/`require_faces`) so typed safety remains available where needed.

v0.1 subset (if partially landed early): minimal fluent wrapper for selection + single-op + apply.
v0.5 target: full chaining surface and richer composition ergonomics.

## Acceptance Criteria

- MeshEdit fluent API supports representative chains (select -> extrude -> inset -> delete/tag)
- fluent lifecycle exposes plan/preview/apply entry points (directly or via clear equivalent API names)
- deterministic behavior and plan fingerprint parity match equivalent direct operator flows
- docs include fluent usage examples and typed selection/output narrowing patterns
