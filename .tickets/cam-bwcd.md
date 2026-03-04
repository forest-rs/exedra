---
id: cam-bwcd
status: open
deps: []
links: []
created: 2026-03-04T04:27:31Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, docs, api]
---
# Operator namespace and module discoverability pass

Curate Cambium operator discoverability at crate-root/module namespace level as operator count grows for 0.1.

## Design

Define and enforce namespace conventions for operator names and module grouping (e.g., uv.*, tag.*, mark.*, inspect.*, select.*). Ensure root re-exports are grouped/documented, avoid flat undifferentiated surface, and provide stable naming guidance for future operators. Add rustdoc section in crate docs that points to operator families and primary entry points.

## Acceptance Criteria

- Namespace conventions documented and reflected in current operator names/docs
- Crate root/operator docs provide grouped discoverability by operator family
- New operators added in 0.1 follow naming/module conventions
- Tests/docs updated where naming expectations are asserted
