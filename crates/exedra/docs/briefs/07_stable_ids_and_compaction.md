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

## Non-goals / deferrals
- Perfect minimal remap formats; start with dense per-domain maps if needed and optimize later.
