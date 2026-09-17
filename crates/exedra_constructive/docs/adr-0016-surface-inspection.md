# Surface inspection and attachment diagnostics

Status: accepted

Constructive owns discovery of surviving selectable surfaces and geometric
failure evidence. Assembly owns the snapshot lifetime of that evidence. Neither
chooses the caller's attachment, anchor, roll, or repair action.

## Contract

- Return a selector-bearing failure from frame resolution, with structured
  ambiguity, planarity and budget evidence; retain the coarse error category
  as a classification method. Carry the same failure through retained nodes.
- Extract shared patch analysis so inventory and frame resolution agree about
  connectedness, planarity and winding. Inventory enumerates supported semantic
  selectors and their connected patches without inventing authored frames.
- Bound inventory work and output explicitly. Report unknown ancestry rather
  than infer lost labels. No transient patch identifier becomes persistent intent.
- Expose inventory from immutable snapshot bodies, test diagnosis and explicit
  recovery through public APIs, and preserve stale-selection checks.
- Schema 35 records the changed workplane budget semantics and shared geometric
  analysis. No dependencies or unsafe code.

## Public model and limits

`WorkplaneFailure` carries the requested selection, the existing coarse category,
and `WorkplaneEvidence`. Extra evidence is supplied for duplicate labels,
disconnected patches, mixed operands, nonplanarity and budgets. Other categories
explicitly carry no additional evidence. Retained attachments carry the same
failure without reducing it to a display string.

Inventory enumerates supported semantic cap and region selectors with surviving
faces, not every possible individual-face query. Entries can overlap. Origins and
unknown-ancestry counts describe what is retained, without inventing a cause for
missing provenance. Connected patch IDs are source face IDs scoped to this body.
`SurfacePlane` describes a geometric plane; it is not an authored workplane.

Global face/selector/corner limits bound inventory scan, output, and repeated
patch analysis. Corner counts include connectivity and geometry visits; these are
specific work units, not a CPU-time guarantee for robust triangulation. No partial
inventory is returned on budget failure. Per-entry geometry/selection failures do
not prevent inspection of other entries. Effective policy and measured work stay
with the inventory. Valid planar status does not certify closed solids, containment,
or validity of a caller's future anchor/projection/X choices.

Snapshots retain the queried body and check scope before evidence is reused.
Constructive standalone callers retain the existing same-logical-mesh obligation;
revision equality alone cannot distinguish unrelated bodies.

## Migration

Resolution functions now return `WorkplaneFailure`; match its `kind` for existing
category handling and `evidence` for detail. Mesh-revision checks still return the
coarse `WorkplaneError`. `TessellateError::Attachment` carries the detailed failure.
Workplane `max_corners` now counts two visits per selected corner on success.
Regenerate schema-stamped text/cache keys and adjust explicitly tuned budgets.
