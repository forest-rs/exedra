# ADR 0001: Structural graph and proof levels

## Status

Accepted for the isolated experiment; adoption into `basilica_ruin` remains a
separate gate. The consequence that the graph "stays example-private until a
later ticket demonstrates a reusable boundary" is superseded by
[ADR 0002](adr-0002-joiner-construction-layer.md), which names that boundary
and the `joiner` crate that will own it.

## Context

The accepted basilica authors roof slabs and visible trusses independently.
Their formulas agree closely enough to render, but the principal rafters are
deliberately separated from the roof underside. Bounds tests can detect a
particular collision or reveal without establishing a complete support chain.

## Decision

`basilica_structure_lab` owns one deterministic structural graph containing
stable keys for nodes, geometry-bearing elements, joints, bearings, supports,
and directed load-transfer edges. The graph is the only authoring source for
validation, assembly geometry, OBJ groups, semantic layers, and explanations.

Assembly hierarchy continues to mean transform composition. Structural
connectivity remains in the lab graph and is not encoded through parent-child
placement.

Validation is deliberately layered:

1. Schema and coherence: finite values, unique keys, valid references,
   positive geometry, member endpoints inside their emitted solids, and joint
   incidence with every named member.
2. Contact: local-bounds-checked carried/carrier anchors that coincide along
   the normal and both tangents, an orthonormal frame, signed gap, finite
   nonnegative overlap minima, and measured overlap. The contact tolerance is
   `1e-9 m`.
3. Load path: unique required direct-support multiplicity, acyclic transfers,
   and a named route from every carrying element to ground. A transfer counts
   only when its kind has a matching bearing, joint, or ground-support witness.
4. Kinematic or stiffness analysis is not implemented.
5. Statics, FEA, capacity, and certification are explicitly outside the claim.

Evidence-source keys and compatible classifications travel with the model so a
modern joint hypothesis cannot be mistaken for observed Early Byzantine
fabric. Joint kinds in this slice describe uncut connectivity hypotheses;
subtractive mating faces are deferred to the joint specimen ticket.

## Consequences

- Roof geometry cannot move independently of its declared bearings without a
  validation failure.
- Every emitted geometry group is addressable by the same stable graph key
  used in diagnostics.
- The graph stays example-private until a later ticket demonstrates a reusable
  boundary.
- The first slice uses only existing workspace crates and simple oriented-box
  geometry. Detailed subtractive joinery and solver integration remain
  extension points, not hidden claims.
