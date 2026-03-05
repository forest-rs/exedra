---
id: cam-mrwk
status: closed
deps: []
links: []
created: 2026-03-05T02:03:21Z
type: epic
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, architecture]
---
# Operator architecture spine (SDK boundary + plan/apply)

Establish the core architecture direction: Cambium as public SDK surface, Exedra as engine kernel, and a deterministic compile/apply flow via EditPlan. This epic coordinates required refactors so future operators and selection/graph work land on stable foundations.

## Design

Scope: 1) clarify Cambium-first API surface and docs, 2) introduce minimal EditPlan + compile/apply lifecycle in Cambium, 3) evolve Exedra session/kernel boundaries to support deterministic planning and scalable local edits. Keep no_std/determinism guarantees. Defer full command-object/undo model.

## Acceptance Criteria

- Ticket tree established and linked for engine/session cleanup, adjacency infra, propagation core, EditPlan lifecycle, and selection bridge.
- Explicit do-now vs defer decisions documented in ticket notes.
- Subsequent implementation tickets reference this epic.

## Notes

- Landed under this epic:
  - Engine/session and perf: `exe-4zwi`, `exe-23ot`, `exe-0sqg`, `exe-08f4`
  - Propagation core and policy: `exe-cz8g`, `exe-2zwc`
  - SDK boundary + lifecycle: `cam-inf3`, `cam-gvmz`
  - Selection/composition + fluent layer: `cam-t6lk`, `cam-coak`
- Originally deferred (`exe-2zwc`, `cam-t6lk`, `cam-coak`) were pulled forward and completed to stabilize the SDK surface early.
- Remaining intentionally deferred architecture work is command-object/undo style modeling, plus larger graph/runtime concerns.
- Perf contract for follow-on work:
  - Tier A (developer loop): local microbench/focused perf tests for perf-sensitive changes.
  - Tier B (CI): correctness + determinism checks (for example adjacency cross-check and stable plan fingerprint checks).
  - Tier C (wind tunnel): regression oracle for scheduled/release triage; not a per-change merge blocker by default.
