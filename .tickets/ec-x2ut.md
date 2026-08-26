---
id: ec-x2ut
status: open
deps: []
links: []
created: 2026-08-26T10:14:48Z
type: bug
priority: 1
assignee: Bruce Mitchener
external-ref: cxc-819.14
tags: [constructive, primitives, semantic-gap]
---
# Evaluate declared constructive box and cylinder primitives

NodeKind::Primitive validates and round-trips Box/Cylinder recipes, but EvalCx::walk falls through to eval.unimplemented and emits no body. cx-catalog cxc-819.14 exposed this while building panel carcases and currently uses rectangular profile extrusions as a correct workaround.

## Design

Give primitive nodes an explicit evaluation path with transformed meshes, stable FACE_REGION/source-map attribution, exact fidelity, cache participation, and report counters. Reuse exedra_primitives only if the dependency direction and no_std feature propagation remain clean; otherwise keep a narrow local tessellation seam.

## Acceptance Criteria

Box and cylinder PrimitiveSpec nodes emit deterministic placed bodies; non-identity placements and cache hits are tested; diagnostics contain no eval.unimplemented; source-map/region semantics are documented; strict workspace clippy, tests, and rustdoc pass.

