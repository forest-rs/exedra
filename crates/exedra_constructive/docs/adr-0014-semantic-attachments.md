# Semantic workplane attachments and planar clearance

Status: accepted

## Ownership

Constructive owns semantic surface selection, authored workplane attachment,
retained attachment evaluation, and planar boundary/footprint measurements.
Mesh owns topology. Assembly owns snapshot/body identity and occurrence placement.
The physical measurement-value crate remains independent of geometric queries.
No new dependencies or unsafe code are introduced.

## Persistent intent and resolution

`SurfaceSelector` contains no mesh IDs. Start/end caps refer to source features;
regions and Boolean-operand-qualified regions express explicit authored intent.
Resolution requires one connected planar patch, or returns a typed failure.
Extrusion-to-plane terminal faces now retain `CapEnd` meaning instead of exposing
the internal splitter's generic `PlaneCutCap` feature. Standalone cut caps keep
their separate meaning even when they reuse the same region number.

`WorkplaneAttachment` specifies a surface, anchor, projection direction and
preferred X. Its origin is the intersection of the selected plane and that
infinite line; positive and negative distances are allowed. Normal winding and
projected X determine a right-handed orthonormal frame. Invalid, near-parallel,
nonplanar, missing or ambiguous inputs are refused; no axis/face is guessed.
The origin need not lie inside the selected material domain.

`OnWorkplane` evaluates one complete support body locally, resolves the frame,
and evaluates its child in that frame. It emits the child, not the support.
Ancestor placement applies afterward. Existing child caches key on the resolved
placement and policy; support/body cache hits do not suppress resolution checks.
Reports retain local frame, selected face count and planarity evidence. Reused
instance definitions share recorded local resolutions, not invented per-instance
world frames. Serialization and fingerprints retain all authored intent.

## Verification

`PlanarPatch` extracts the selected patch's evaluated polygonal boundary and
preserves source edges, face features, adjacent features and measured work.
Validation rejects degenerate/backtracking edges, nonadjacent crossings or
near-contact, and invalid outer/hole nesting under explicit finite budgets.
Orientation predicates come from the existing triangulation crate.

Circular footprints measure signed center-to-boundary distance minus radius.
Witnesses identify the nearest segment and point in workplane coordinates.
Queries allocate no new mesh and do not repeat tessellation. `classify` uses an
explicit decision tolerance and distinguishes satisfied, violated and within
tolerance. The tolerance is not a rigorous numerical or analytic error bound.
Measurements concern the evaluated mounting face, not global solid validity,
three-dimensional bore clearance, minimum wall thickness, or structural safety.

Assembly snapshots add unique producing-source lookup and attachment/patch
queries. Missing and duplicate source labels are explicit results. Snapshot
workplanes own their body and retain scope checks; persistent attachment intent
is re-resolved after edits. No transient face IDs are serialized as attachments.

## Limits and migration

[ADR 0015](adr-0015-boolean-surface-provenance.md) extends cap selection through
Booleans with separate original-surface records and source-qualified selectors.
Immediate operand attribution remains available. General topology matching and
curved-surface attachments remain outside this contract.

Schema 33 invalidates evaluation fingerprints and canonical text. Queries for
extrusion-to-plane terminal faces should use `CapEnd`. Explicit `WorkplanePolicy`
literals gain `min_projection_cos`, and explicit `EvalPolicy` literals gain
`workplane`; defaults retain strict-origin behavior for
existing `face_workplane` calls. Render-only assembly compilation is unchanged.

The `material_gallery` `semantic_attachments` example authors actual retained
Boolean drills, re-resolves resized/tilted supports, and highlights measured
clearance violations. It publishes canonical recipes, measurement reports, and
geometry extracted from the query snapshot.
