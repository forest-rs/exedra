---
id: exe-1wdr
status: open
deps: []
links: []
created: 2026-03-03T05:51:38Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v1.0]
---
# Semver-stable API audit

Audit the entire Exedra public API surface for semver stability. Ensure types, traits, and methods are intentionally public and documented. Lock the API shape for 1.0.

## Acceptance Criteria

- All public types and methods are intentionally public
- All public items are documented
- No accidental pub(crate) leakage
- API surface documented in a summary

