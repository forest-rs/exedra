---
id: cam-kxpi
status: open
deps: []
links: []
created: 2026-03-04T00:40:37Z
type: chore
priority: 3
assignee: Bruce Mitchener
tags: [v0.1, perf]
---
# Optimize canonicalize_face_set/edge_set changed-detection path

canonicalize_face_set and canonicalize_edge_set currently pre-scan with windows(2) to compute changed, then sort+dedup, resulting in an extra pass. Minor hot-path cleanup opportunity for very large selections.

## Design

Replace double-scan with a helper that computes changed with minimal overhead (e.g., clone+compare for selected thresholds or single pass after sort using before snapshot policy). Preserve deterministic order and exact changed semantics.

## Acceptance Criteria

1) canonicalize helpers keep identical observable behavior. 2) implementation avoids current pre-scan pattern. 3) tests cover unchanged vs changed cases. 4) clippy/test pass.

