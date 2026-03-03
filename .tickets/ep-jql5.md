---
id: ep-jql5
status: open
deps: []
links: [ep-od6p, ep-wbxp]
created: 2026-03-03T17:19:52Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# no_std trig policy for primitive generation

Define and implement a clear numeric policy for trigonometric evaluation in no_std primitive generation (currently custom polynomial approximation).

## Design

Decide whether to keep the current approximation, switch to a feature-gated backend (e.g., libm under optional feature), or use a hybrid strategy. Document error tolerance expectations and determinism requirements. Add tests that quantify approximation error bounds relevant to cylinder/uv_sphere generation and verify that topology/selection determinism remains unchanged.

## Acceptance Criteria

A documented trig policy exists in crate docs; implementation matches policy; tests enforce chosen error bounds (or conformance assertions); no_std default remains dependency-light; deterministic outputs remain stable for fixed params.

