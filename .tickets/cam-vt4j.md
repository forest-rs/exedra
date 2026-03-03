---
id: cam-vt4j
title: OpContext, Scratch, and Clock
status: closed
deps: [exe-mid7, exe-dc9l]
links: []
created: 2026-03-03T05:53:48Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# OpContext, Scratch, and Clock

Implement the operator execution context. OpContext carries policy, numeric config, scratch buffers, diagnostics sink, and timing clock. Scratch provides reusable typed buffers for hot-loop-friendly operation. Clock records timing into named buckets.

## Design

OpContext { policy, numeric, scratch, diagnostics, clock }

Scratch:
- Typed buffer pools: u32s, u64s, f32s, vec2, vec3
- ID lists: faces, half_edges, corners, vertices (using Exedra types)
- ScratchMaps: hashbrown maps for reuse
- clear() resets all buffers (retains capacity)
- Operators must not retain references into scratch

Clock:
- bucket(name) -> ClockBucket RAII guard
- Buckets are additive (multiple scopes with same name sum)
- Nested buckets allowed (time attributed to both)
- no_std: records zeros unless std feature enabled
- std: uses std::time::Instant

Module layout: context.rs

## Acceptance Criteria

- OpContext struct exists with all fields
- Scratch provides typed buffer pools, clear retains capacity
- Clock records timing into named buckets
- ClockBucket RAII guard works
- no_std compatible (Clock no-ops without std)
- Unit tests for scratch clear, clock bucket accumulation


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/cambium/docs/briefs/06_clock_and_timing_buckets.md

**2026-03-03T06:36:03Z**

Design brief: crates/exedra/docs/briefs/16_scratch_buffer_protocol.md
