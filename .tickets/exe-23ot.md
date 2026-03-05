---
id: exe-23ot
status: open
deps: [cam-mrwk, exe-0sqg]
links: []
created: 2026-03-05T02:03:21Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, perf, architecture]
---
# Maintained vertex adjacency index for local edits

Introduce maintained adjacency/index structures so local edit kernels avoid repeated full-arena scans.

## Design

Add internal adjacency/index support (at minimum vertex -> incident/outgoing half-edges) maintained across kernel mutations. Provide deterministic iteration/tie-break behavior. Migrate scan hotspots (has_undirected_edge, find_boundary_half_edge, vertex_has_incident_half_edge, stitch_outside_loops helpers where applicable) to indexed paths.

## Acceptance Criteria

- Internal adjacency index maintained correctly through existing edit kernels; - scan-heavy helpers migrated or wrapped by indexed equivalents; - deterministic behavior documented; - tests cover index consistency after representative edits; - local benchmark/test evidence is recorded for targeted scan replacements (wind tunnel integration may land as follow-up regression harness)
