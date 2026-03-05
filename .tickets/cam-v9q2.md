---
id: cam-v9q2
title: cambium_wind_tunnel crate
status: open
deps: [cam-gvmz]
links: [exe-m4re]
created: 2026-03-03T06:01:15Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.5, infra]
---
# cambium_wind_tunnel crate

Create cambium_wind_tunnel workspace crate for operator performance scenarios. CWT-1: UV planar 500k. CWT-2: UV box 500k. CWT-3: Subdivision L1 200k. CWT-4: Boolean preview vs commit.

## Notes

- Sequencing: crate scaffolding and non-EditPlan scenarios can land independently; EditPlan fingerprint scenarios follow `cam-gvmz` once plan lifecycle/types are available.
- Wind tunnel role: regression oracle for periodic/release profiling; day-to-day merge gates come from local/CI perf + determinism checks.

## Acceptance Criteria

- Crate exists in workspace
- At least CWT-1 scenario implemented
- Benchmark harness comparable across runs
- Includes determinism regression check (identical inputs produce identical plan fingerprint / stable output signature across runs)
