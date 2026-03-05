---
id: cam-inf3
status: open
deps: [cam-mrwk]
links: []
created: 2026-03-05T02:03:21Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, docs, api, architecture]
---
# Cambium-first public surface policy

Codify and implement Cambium as the recommended public SDK surface, with Exedra treated as engine-level API.

## Design

Define and document API tiers:
- Exedra (engine tier): topology/attribute storage, invariants, deterministic kernel edits, and performance-critical internals. No workflow/fluent/operator convenience surface.
- Cambium (SDK tier): operator catalog, selection/query ergonomics, compile/apply lifecycle, fluent MeshEdit surface, and workflow-first docs/examples.

Add Exedra admission checklist for new API surface:
- required for kernel correctness/invariants/performance
- deterministic and explicit (no hidden mutable global behavior)
- compatible with `no_std` and dependency policy
- covered by kernel-level tests/validation
- not a workflow/operator convenience API

Add exception process: API-tier exceptions require a ticket tagged `api-exception` documenting rationale, alternatives considered, and maintainer approval before merge.

Curate re-exports and docs so normal users import `cambium::*` for workflows. Keep Exedra available but explicitly documented as engine tier, with links to Cambium for operator/workflow entry points.

## Acceptance Criteria

- Cambium and Exedra crate docs include explicit tier guidance and cross-links (Exedra points workflow users to Cambium)
- Cambium docs include a `cambium::*` getting-started path for common workflows
- Re-export policy is documented: Cambium exposes the workflow-facing surface; Exedra avoids SDK/fluent/operator namespaces
- Operator catalog/discoverability docs include a clear "where to find X" map (selection, queries, operators, planning, inspection)
- Representative examples/doctests use Cambium imports for workflow scenarios; engine-level examples are explicitly labeled
- Exception path (`api-exception` ticket) is documented and referenced in contribution guidance
