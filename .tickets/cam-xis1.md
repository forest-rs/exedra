---
id: cam-xis1
status: open
deps: [cam-4x8o, cam-f0wg]
links: []
created: 2026-03-03T05:54:17Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# OpReport, Stats, and Timings

Implement operator reporting infrastructure. Every operator produces an OpReport with deterministic stats/counters and best-effort timing. Stats are part of the determinism contract; timings are not.

## Design

OpReport { name: &static str, stats: Stats, timings: Timings, artifacts: Artifacts }

Stats { elements_touched, elements_created, elements_deleted, counters: SmallCounters }
- SmallCounters: fixed struct with common fields (faces_processed, corners_written, corners_skipped_existing, selections_canonicalized)
- Add new counters as fields (semver-managed)

ElementsTouched/Created/Deleted: per-domain u64 counts (vertices, half_edges, faces, corners)

Timings:
- Vec<TimeBucket> in deterministic order
- max_buckets limit (e.g. 32) prevents unbounded growth
- add(name, nanos) accumulates into existing bucket or creates new
- Deterministic: if max exceeded, new bucket is dropped

Module layout: report.rs

## Acceptance Criteria

- OpReport struct exists with all fields
- Stats counters are deterministic
- Timings accumulate correctly, respect max_buckets
- SmallCounters has documented fields
- Unit tests for timing accumulation, max_buckets overflow

