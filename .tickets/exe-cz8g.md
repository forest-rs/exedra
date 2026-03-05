---
id: exe-cz8g
status: closed
deps: [cam-mrwk]
links: []
created: 2026-03-05T02:03:21Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, architecture]
---
# Shared attribute propagation kernel helpers

Centralize seam/sharpness/corner-UV propagation behavior for topology-creating kernels to eliminate per-kernel drift.

## Design

Add internal propagation helpers for edge/corner attributes (and policy interpretation) used by split_edge, split_face, add_face/face-edit-related kernels. Define explicit missing-data behavior and deterministic source selection rules.
Helper APIs must take explicit `&PropagatePolicy` inputs now (even if current call sites source that from session state) so the later per-call policy migration is a call-site/API cleanup, not a helper redesign.

## Acceptance Criteria

- Shared propagation helpers exist and are used by split_edge and split_face at minimum; - propagation semantics documented in one place; - helper signatures accept explicit policy inputs; - tests verify consistency across kernels and policy variants

## Notes

- Do now: centralize logic and force explicit-policy helper signatures.
- Preferred sequencing: land `exe-4zwi` first when practical to avoid `Txn` -> `EditSession` rename churn in newly extracted helper code (soft preference, not a hard dependency).
- Scope for this ticket: shared helper primitives and migration of `split_edge` + `split_face` to those helpers.
- Downstream extension: `cam-h7yk`/`cam-7u7l` carry adoption into add-face/face-edit compound flows that currently have partial or deferred propagation behavior.
- Defer to `exe-2zwc`: remove session-global policy mutation and propagate explicit policy through all public/session kernel entry points.
