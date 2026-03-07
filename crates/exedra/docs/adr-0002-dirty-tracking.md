# ADR-0002: Dirty Tracking via understory_dirty

**Status:** Accepted
**Date:** 2026-03-03

## Context

Exedra's kernel boundary contract requires explicit dirty tracking: recorded
edit scopes produce a `ChangeSet` containing a `DirtySet` that tells higher layers which
faces, vertices, and corners need derived data recomputed. Cambium consumes
these dirty sets for incremental workflows.

Dirty tracking is a shared concern across forest-rs projects, not specific to
mesh topology.

## Decision

Both Exedra and Cambium use **`understory_dirty`** from the understory
workspace for dirty tracking primitives.

This is added as a workspace dependency:

```toml
understory_dirty = { git = "https://github.com/endoli/understory.git", rev = "83ccf57799fe9aef99b78bfd5d541b9fad45200a" }
```

Exedra's `DirtySet` and `ChangeSet` types will be built on top of
`understory_dirty` rather than rolling custom tracking.

## Consequences

- Shared dirty-tracking semantics across the forest-rs ecosystem.
- Exedra avoids reinventing set-tracking primitives.
- Pinned to a specific git rev for reproducibility; update deliberately.
