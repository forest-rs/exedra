---
id: cam-fxvy
status: closed
deps: []
links: []
created: 2026-03-05T09:40:22Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, api]
---
# Fluent MeshEdit selection query integration

Wire new selection queries into MeshEdit so fluent flows can derive selections without manual handoff.

## Design

Add MeshEdit selection-step support for region and flood queries with deterministic compile/apply behavior. Keep face-domain fluent chain contract explicit; edge-loop selection entry should be available without breaking existing face-only steps.

## Acceptance Criteria

- MeshEdit can seed/replace face selection from region query and flood-by-region query
- Edge-loop query is available through fluent surface (domain-checked)
- Selection handoff remains deterministic with tests
- Rustdoc updated with examples

