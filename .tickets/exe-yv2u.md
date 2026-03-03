---
id: exe-yv2u
status: open
deps: []
links: []
created: 2026-03-03T05:51:38Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v1.0]
---
# Fuzz targets for topology edits and booleans

Create fuzz targets that exercise topology edit primitives and the boolean pipeline with random/generated inputs. Goal: find invariant violations and panics.

## Acceptance Criteria

- Fuzz targets exist for: split_edge, split_face, collapse_edge, flip_edge
- Fuzz target for boolean pipeline with random mesh pairs
- validate_deep() called after each fuzzed operation
- No panics or invariant violations found after reasonable fuzzing time

