---
id: exe-jgq6
status: closed
deps: []
links: []
created: 2026-03-07T14:26:47Z
type: epic
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, architecture, api]
---
# Kernel op catalog for Exedra

Introduce exedra::op as the public kernel-operation catalog so EditSession stops being both transaction host and operation catalog.

## Design

Fence: session/* owns transaction hosting, bookkeeping, dirty/change tracking, cache invalidation, and low-level mutation helpers; op/* owns public kernel operation definitions and typed apply boundaries over &mut EditSession. Start with concrete op types and inherent apply methods rather than a trait hierarchy.

## Acceptance Criteria

1) exedra::op exists and is documented as the kernel operation catalog. 2) Main topology edits have first-class op entry points. 3) EditSession docs are updated to emphasize transaction-host responsibilities. 4) Tests cover op entry points and existing behavior remains stable.

