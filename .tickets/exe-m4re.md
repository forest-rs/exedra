---
id: exe-m4re
title: exedra_wind_tunnel crate
status: open
deps: []
links: [cam-v9q2]
created: 2026-03-03T05:52:56Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.5, infra]
---
# exedra_wind_tunnel crate

Create the exedra_wind_tunnel workspace crate for benchmarks and performance regression scenarios. Contains the formal wind tunnel scenarios from the spec.

## Design

Wind tunnel scenarios:
- WT-1: Triangulation stress (500k faces, UVs, sharp edges)
- WT-2: Normal generation (1M faces)
- WT-3: Incremental edit (500k faces, 100 split_edges)
- WT-4: Boolean medium (2x200k faces)
- WT-5: Boolean heavy (2M faces, offline)

Each scenario measures: time, allocations, memory, element counts.
Uses criterion or similar for benchmarking.
Reports must be comparable across runs for regression detection.
Wind tunnel acts as a regression oracle (scheduled/release/perf-triage), not a mandatory per-change merge gate.

## Acceptance Criteria

- Wind tunnel crate exists in workspace
- At least WT-1 scenario implemented
- Benchmark harness produces comparable results
- Includes determinism/stability check path suitable for Exedra scenarios (stable output signature across repeated runs for identical inputs)
- No std dependency leaks into exedra core
