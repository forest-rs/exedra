---
id: cam-suy5
status: open
deps: [cam-g3hn]
links: []
created: 2026-03-05T08:04:54Z
type: task
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, api, docs]
---
# Operator naming and stable ID alignment pass

Align operator stable names, type names, and module placement with the curated taxonomy so the API surface is calm and consistent.

## Design

Audit all current Cambium operators for naming consistency across: Rust type names, module paths, and operator name() strings. Define allowed prefixes and naming grammar. Propose renames where needed, including documentation/test updates and compatibility notes for pre-release churn. Focus on consistency and discoverability, not behavior changes.

## Acceptance Criteria

- Naming convention documented with examples; - current operators audited against the convention; - required renames identified and ticketed/implemented; - rustdoc and naming-related tests updated to reflect final names.

