---
id: cam-mrwk
status: open
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

- Ticket tree established and linked for engine/session cleanup, adjacency infra, propagation core, EditPlan lifecycle, and selection bridge; - explicit do-now vs defer decisions documented in ticket notes; - subsequent implementation tickets reference this epic

## Notes

- Do now (v0.1): `exe-4zwi`, `exe-23ot`, `exe-cz8g`, `cam-gvmz`, `cam-inf3`
- Defer (v0.5): `exe-2zwc`, `cam-t6lk`, `cam-coak`
- Decision rationale: establish deterministic compile/apply and local-edit performance first; then layer ergonomics/composition once the engine boundary and propagation/indexing internals are stable.
- Preferred sequencing: land `exe-4zwi` before `exe-cz8g` when practical to avoid rename churn in newly extracted helpers (not a hard dependency).
- Perf contract:
- Tier A (developer loop): local microbench/focused perf tests required for perf-sensitive changes.
- Tier B (CI): correctness + determinism checks (for example adjacency cross-check and stable plan fingerprint tests) required.
- Tier C (wind tunnel): regression oracle for scheduled/release triage; not a per-change merge blocker by default.
