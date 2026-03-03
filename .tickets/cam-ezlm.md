---
id: cam-ezlm
status: open
deps: []
links: [cam-ibof, cam-vt4j]
created: 2026-03-03T13:22:30Z
type: task
priority: P2
assignee: Bruce Mitchener
---
# Define threadability model for OpContext/Clock/Runner

Document and decide the long-term threading boundary for Cambium runtime types. Today OpContext contains non-Sync pieces (Clock via RefCell, scratch buffers, sinks) and is naturally per-run/thread-local. Before parallel runner work lands, define which components are intentionally thread-local vs shareable, and the API shape for both paths.

## Design

Adopt an explicit split: (1) thread-local execution context for hot mutable state (scratch, per-op diag/artifact assembly, local clock capture), and (2) optional shared runtime state for cross-thread orchestration/aggregation. Define trait or type boundaries so single-thread remains zero-overhead while parallel execution can opt into Sync components without forcing synchronization on all users. Specify Clock strategy (local fast clock vs shared aggregator), ownership/lifetimes, and Send/Sync contracts for public types. Capture migration notes from current OpContext. Include no_std/std/wasm posture in the decision.

## Acceptance Criteria

1) New/updated ADR or design doc states exact Send/Sync contract for OpContext, Clock, and runner-facing types. 2) Public docs call out which types are thread-local vs shareable. 3) If API changes are required, ticket lists migration steps and compatibility posture. 4) A follow-up implementation slice is identified (or linked) for introducing shared runtime pieces without regressing current single-thread performance. 5) Decision references cam-vt4j and cam-ibof so runtime work follows the chosen boundary.
