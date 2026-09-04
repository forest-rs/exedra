# Brief: Stable IDs (index + generation) and explicit compaction

## Decision
Expose stable public handles as `(index, generation)`. Deletion creates tombstones; memory reclamation happens only through an explicit **compaction** operation that returns an ID remap.

## Why
Stable IDs enable:

- safe caching keyed by IDs
- safe external references (avoid use-after-free)
- deterministic ordering and traversal

Tombstones keep editing cheap and local; explicit compaction preserves “explicit over implicit” and prevents surprise pauses.

## Alternatives considered
- **Raw indices**: fast but unsafe for long-lived references and caches.
- **Pointer-based nodes**: ergonomic but worse cache locality and harder `no_std` posture.
- **Implicit GC/compaction**: unpredictable performance.

## Implications
- Any API returning sets of IDs must define deterministic ordering.
- Compaction returns remap and invalidates old IDs for the new mesh.
- Generation checks become part of validation and debug tooling.

## Implementation
`Mesh::compact` copies live vertices, faces, half-edges, and attributes into a
fresh mesh with contiguous arena slots. It returns a `Remap` with per-domain
lookups for source IDs. Deleted or stale source IDs return `None`, and
`FaceId::OUTSIDE` maps to itself because it is a sentinel rather than an arena
entry.

## Non-goals / deferrals
- Perfect minimal remap formats; start with dense per-domain maps if needed and optimize later.
