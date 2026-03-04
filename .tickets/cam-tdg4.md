---
id: cam-tdg4
title: Typed operator outputs for authoritative chaining
status: open
deps: []
links: []
created: 2026-03-04T16:17:03Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Typed operator outputs for authoritative chaining

Add typed operator outputs so workflow-critical data does not depend on bounded artifacts.

## Design

Extend EditOperator with an Output associated type (default unit) and update OperatorRunner result types to include typed output for commit/preview. Keep OpReport::artifacts bounded/debug-only. Migrate initial operators (InsetFaces/ExtrudeFaces at minimum) to return typed generated-face sets.

## Acceptance Criteria

- EditOperator supports typed output
- run_commit/run_preview return typed output
- InsetFaces/ExtrudeFaces migrated to typed outputs
- Docs clarify artifacts vs outputs contract
- Tests cover chaining without artifact lookup
