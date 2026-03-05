---
id: exe-4zwi
status: open
deps: [cam-mrwk]
links: []
created: 2026-03-05T02:03:21Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, architecture]
---
# Rename Txn to EditSession and split session modules

Rename Exedra Txn to EditSession to match eager non-rollback semantics, and split the monolithic session implementation into concern-focused modules without behavior changes.

## Design

Mechanical refactor: rename public type + docs, keep compatibility aliases only if needed briefly, and split implementation into files (session core/bookkeeping, topology kernels, attribute writes, query delegates). No semantic changes to mutation behavior or ChangeSet output.

## Acceptance Criteria

- Txn renamed to EditSession across Exedra and downstream call sites; - session implementation split into modular files; - no behavior regressions (existing tests pass); - rustdoc updated to remove rollback implication

