# Brief: Face-edit semantics matrix (extrude/inset/solidify)

## Decision
Cambium face-edit semantics are defined explicitly by operation family and mode,
with deterministic topology outcomes across open-surface and closed-volume
contexts.

This brief is the contract for `cam-7u7l` and `cam-1xjc`.

## Terminology
- **Selection patch**: canonical set of selected interior faces.
- **Outer boundary**: boundary edges of the patch (edges incident to exactly
  one selected face).
- **Internal boundary**: shared edges between selected faces (not part of outer
  boundary).
- **Source face**: a selected input face before edit.
- **Generated cap**: offset/rebuilt face set derived from source faces.

## Mode naming (forward-compatible)

### Extrude
- `ShellOpen` (current default behavior): remove source faces, create side
  walls on outer boundary, create offset cap.
- `KeepSource`: keep source faces, create side walls on outer boundary, create
  offset cap.

### Inset
- `FaceInset` (current behavior): remove source faces, create frame ring +
  inset inner faces.
- Future extension (deferred): patch-inset modes for shared inner patch
  behavior over adjacent selections.

### Solidify (future family)
- `OffsetPair`: duplicate and offset selected surface, bridge boundary loops.
  (Deferred beyond v0.1; listed for naming/semantic alignment only.)

## Semantics matrix

| Op | Mode | Context | Source Faces | Walls Created On | Cap/Inner Creation | Internal Shared Edges |
| --- | --- | --- | --- | --- | --- | --- |
| Extrude | `ShellOpen` | Open surface | Removed | Outer boundary only | Offset cap faces | Collapsed (no internal walls) |
| Extrude | `ShellOpen` | Closed volume | Removed | Outer boundary only | Offset cap faces | Collapsed (no internal walls) |
| Extrude | `KeepSource` | Open surface | Preserved | Outer boundary only | Offset cap faces | Collapsed (no internal walls) |
| Extrude | `KeepSource` | Closed volume | Preserved | Outer boundary only | Offset cap faces | Collapsed (no internal walls) |
| Inset | `FaceInset` | Open/closed | Removed | Outer boundary-derived frame | Inner inset faces | Collapsed (no duplicate internal frame walls) |

## Attribute/region propagation contract
- Face region:
  - generated walls and cap/inner faces inherit region from owning source face
    (or deterministic patch-owner rule when adjacency is merged).
- Corner UV:
  - generated corners copy source per-face/per-vertex UV where available.
- Edge seam/sharpness:
  - boundary-parallel generated edges follow `policy.propagate.edge_attr`.
  - support/bridge edges default clear unless explicitly specified by mode.

## Determinism and topology invariants
- Selection input must be canonical (sorted, deduplicated).
- Patch boundary construction is deterministic in stable ID order.
- Winding/orientation follows `cam-mn4h` contract (single resolved orientation
  per operator invocation/component, then reused).
- Internal shared borders in adjacent selections are never emitted as duplicate
  walls.
- Result must pass `validate_fast` and `validate_deep` in tests.

## Implementation mapping
- `cam-1xjc`: implements adjacency-aware boundary extraction and internal-edge
  collapse for extrude/inset.
- `cam-7u7l`: introduces explicit extrude mode enum and behavior according to
  this matrix.

## Non-goals / deferrals
- Full patch-level inset remeshing across complex non-planar adjacent
  selections.
- Solidify operator implementation in v0.1.
