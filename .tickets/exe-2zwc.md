---
id: exe-2zwc
status: open
deps: [cam-mrwk]
links: []
created: 2026-03-05T02:03:21Z
type: task
priority: P2
assignee: Bruce Mitchener
tags: [v0.5, architecture]
---
# Per-call propagation policy for edit kernels

Move propagation policy from mutable session-global state toward explicit per-call inputs for kernels/operators that need mixed policy behavior within one edit flow.

## Design

Refactor kernel signatures/session APIs so policy is passed explicitly per operation (or per helper call) with an optional session default for convenience. Preserve deterministic behavior and avoid hidden mutable policy state in compound operators.

## Acceptance Criteria

- Core kernels that consume propagation policy accept explicit policy input; - session-global mutation of policy is removed or clearly deprecated; - compound-operator tests cover mixed-policy flows

## Notes

- This ticket assumes `exe-cz8g` already moved shared propagation helpers to explicit-policy signatures.
- Primary scope here is call-site/session API migration and deprecation/removal of mutable session-global policy state.
