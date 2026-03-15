# ADR-0002: Multi-Domain Geometry Architecture

- Status: Accepted
- Date: 2026-03-15
- Owners: Cambium maintainers
- Ticket: `cam-m068`

## Context

The workspace already has a real polygon kernel (`exedra`) and a real workflow
layer (`cambium`). The architectural pressure is coming from future geometry
domains that do not fit a mesh-first source of truth:

- analytic/topological geometry,
- implicit fields,
- points/curves and reconstruction flows.

The wrong response would be to force these into the polygon kernel or invent a
lowest-common-denominator geometry abstraction that preserves little of each
domain's value.

## Decision

Adopt a multi-domain geometry architecture:

- `exedra` remains the polygon head.
- sibling crates own other canonical domains.
- `cambium` orchestrates domain-native operators and explicit conversions.

The mesh is one canonical authoring domain, not the canonical domain.

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

### Analytic head (`exedra_analytic`, new)

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

## Cambium Role

Cambium becomes the orchestration layer across canonical domains:

- runtime,
- workflow planning,
- diagnostics/reporting,
- explicit conversion steps,
- domain-aware operator discovery.

Cambium does not define a universal mutable geometry model shared by all heads.

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

- current mesh-native Cambium operators remain valid,
- `exedra` should continue getting stronger as an editable polygon kernel,
- mesh-native helpers/primitives in Exedra are still worthwhile.

### What changes

- new domains should not be designed as layers hidden inside `exedra`,
- operator taxonomy and runtime must become domain-aware,
- compile/apply and signature semantics should become honest enough to survive
  a multi-domain future.

## Consequences

Positive:
- preserves the value of the existing polygon kernel,
- makes room for analytic and implicit work without corrupting mesh APIs,
- aligns with a Houdini-like "multiple heads, explicit conversion" model,
- gives Cambium a stronger long-term role than "mesh operators only."

Tradeoffs:
- more crates and more explicit boundaries,
- some concepts will exist in parallel across domains,
- conversion seams become major architecture surfaces and must be documented.

## Follow-on Work

- mesh-head hardening in `exedra` (`exe-x154`, linked `exe-h2rh`)
- source-bound compile/apply semantics in Cambium (`cam-aou6`)
- explicit operator-domain model in Cambium (`cam-7qig`)
- analytic MVP with deterministic tessellation (`cam-6z7d`)
