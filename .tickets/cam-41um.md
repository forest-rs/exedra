---
id: cam-41um
status: closed
deps: []
links: []
created: 2026-03-05T12:54:51Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, demo, web, wasm]
---
# Wasm bridge for scenario-by-name execution

Add a wasm-facing bridge crate/API that runs named Cambium scenarios and returns mesh buffers plus deterministic metadata.

## Design

Expose a constrained API: run_scenario(name, options) -> step snapshots. Each snapshot includes positions/indices/optional region IDs, operator report summary, and plan fingerprint. Return full mesh snapshots per step for v0.1 simplicity.

## Acceptance Criteria

- wasm build target works for bridge crate\n- API executes named scenarios without free-form operator JSON\n- per-step payload includes mesh buffers + fingerprint + counters/diagnostics summary\n- deterministic replay test for at least one scenario

