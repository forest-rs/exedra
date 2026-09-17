# Authored surfaces through Booleans

Status: accepted

Constructive owns authored surface meaning and its propagation through geometric
operations. The mesh Boolean pipeline owns immediate input-face correspondence;
assembly identity and dependency scheduling remain with callers.

## Contract and invariants

- Keep immediate Boolean operand attribution unchanged. Add a separate original
  surface record (feature and optional authored source label) to SourceMap.
- Compose that record only through the Boolean pipeline's input-face mapping.
  Preserve it through placement, including reflection. Never infer ancestry by
  position, normal, region number, or nearest-face matching.
- Resolve start/end caps through this record and support source-qualified caps.
  Source labels identify generating nodes, not arbitrary wrapper nodes. Duplicate
  labels among reachable recipe nodes are ambiguous for qualified queries, including warm-cache
  and snapshot queries. Missing, disconnected, and nonplanar targets fail.
- Selectors serialize and fingerprint their owned labels. Schema 34 invalidates
  earlier evaluation caches. Snapshot queries reuse the same source maps.
- The semantic attachments example now attaches collars to the drilled panel
  through its original cap. No new production dependencies or unsafe code.

Surface records describe ancestry, not persistent face IDs or occurrence IDs.
Coincident Boolean attribution follows the backend's reported surviving input;
this slice does not infer multiple co-owners or promise general topology naming.

## Scope and migration

`SurfaceOrigin` stores the original feature and an optional shared source string.
`SourceMap::surface_origin` is separate from immediate face/vertex features;
Boolean operand-region queries keep their existing behavior. Unknown ancestry
stays unknown. Cap selection still requires one connected planar patch. An
inward or outward orientation comes from current face winding, not the original
cap role. A removed cap is missing, not replaced by a newly generated cut face.

Duplicate labels are computed when freezing the recipe over reachable nodes,
counting shared definitions once. The ambiguity annotation is applied outside the geometry cache and checked
before retained attachment resolution. The whole-recipe fingerprint includes the sorted ambiguity labels alongside
root content, so whole-result caches cannot hide changed naming context. Node
fingerprints remain content-based for subtree geometry reuse. Evaluation and
fingerprinting consume the same frozen ambiguity data. Unreachable duplicate
labels affect neither semantics nor identity. Snapshot-owned maps retain the annotation independently
of compiler eviction. Source-map byte estimates include the new records and
conservatively count shared strings per referencing face.

Unqualified cap selectors select all surviving faces with that original role;
use source-qualified caps when multiple operations contribute caps. This is an
operation reference, not an occurrence reference. Multiple disconnected surviving
patches remain ambiguous. Other topology-changing operations, such as edge
finishing and capped cuts, do not yet promise to propagate these additional
records. Standalone tessellation supplies roles but no recipe source label.

Callers must clone reused workplane selectors/attachments, which are no longer
`Copy`. Regenerate schema-stamped text and cache fingerprints. Existing source
labels, immediate Boolean features, and operand-region queries are unchanged.
Qualified labels in canonical text are UTF-8 hex tokens prefixed with `x` so they
can contain whitespace or be empty without changing the line-oriented parser.
