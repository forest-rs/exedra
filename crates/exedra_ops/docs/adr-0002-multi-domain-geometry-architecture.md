# ADR-0002: Multi-Domain Geometry Boundaries

- Status: Accepted
- Date: 2026-03-15
- Owners: Exedra Ops maintainers

## Context

The workspace has a real polygon kernel (`exedra`) and a real mesh-operations
layer (`exedra_ops`). The architectural pressure is coming from geometry
domains that do not fit a mesh-first source of truth:

- analytic/topological geometry,
- implicit fields,
- points/curves and reconstruction flows.

The wrong response would be to force these into the polygon kernel or invent a
lowest-common-denominator geometry abstraction that preserves little of each
domain's value.

## Decision

Adopt a multi-domain geometry boundary:

- `exedra` remains the polygon head.
- sibling crates own other canonical domains.
- `exedra_ops` owns deterministic mesh operations and typed adapters for
  explicit conversions or workflow expansion.
- each native domain keeps its own value and lifecycle; no current crate
  dispatches heterogeneous values.

The mesh is one canonical authoring domain, not a universal interchange value.
The current `EditOperator`/`OperatorRunner` contract remains mesh-specific.

## Domain Ownership

### Polygon head (`exedra`)

Owns:
- editable polygon topology,
- authored mesh attributes,
- deterministic render extraction,
- mesh-native edit/query invariants.

Does not own:
- exact analytic surface semantics,
- implicit field semantics,
- point-cloud semantics.

### Analytic head (`exedra_analytic`)

Owns:
- canonical analytic topology/geometry state,
- exact-ish authoring semantics for its supported scope,
- deterministic tessellation into polygon mesh.

Does not own:
- polygon edit semantics,
- implicit field semantics.

### Deferred heads

Potential future siblings:
- `exedra_implicit`
- `exedra_points`

These must each own real semantics if added. No placeholder crate should exist
without an earned first slice.

## Exedra Ops Role

Exedra Ops currently owns:

- deterministic mesh-operation planning, preview, application, and reporting;
- mesh selections, policies, diagnostics, and workflow composition;
- typed adapters that make a caller-requested domain crossing explicit and
  observable.

It does not own sibling-domain state or algorithms, define a universal mutable
geometry model, dispatch heterogeneous values, or schedule a graph. A future
Exedra procedural-network layer may own typed geometry ports, native artifact
storage, and compilation into the shared `execution_graph` runtime once it has
an end-to-end consumer, as described by ADR-0005. `execution_graph` retains
incremental scheduling, while `understory_node_graph` can present the authored
network without owning its execution or geometry meaning.

## Conversion Policy

Conversions are explicit and may be lossy.

Requirements:
1. Conversion steps are first-class operations.
2. Data-loss and approximation rules are documented.
3. Determinism is preserved for fixed input + params.
4. Provenance is preserved where practical.

Examples:
- analytic -> mesh tessellation
- implicit -> mesh extraction
- points -> mesh reconstruction

## Implications For Current Work

### What stays true

- current mesh-native Exedra Ops operators remain valid,
- `exedra` should continue getting stronger as an editable polygon kernel,
- mesh-native helpers/primitives in Exedra are still worthwhile.

### What changes

- new domains should not be designed as layers hidden inside `exedra`,
- crossings use typed adapters with their own input, output, and failure
  contracts,
- a future graph may schedule those typed crossings, but the mesh runner does
  not become domain-aware by metadata or by widening its trait.

## Consequences

Positive:
- preserves the value of the existing polygon kernel,
- makes room for analytic and implicit work without corrupting mesh APIs,
- aligns with a "multiple heads, explicit conversion" model,
- leaves a narrow, replaceable scheduling boundary for future graph work.

Tradeoffs:
- more crates and more explicit boundaries,
- some concepts will exist in parallel across domains,
- conversion seams become major architecture surfaces and must be documented.
