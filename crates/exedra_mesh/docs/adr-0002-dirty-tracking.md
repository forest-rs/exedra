# ADR-0002: Dirty Tracking via invalidation

**Status:** Accepted
**Date:** 2026-03-03
**Amended:** 2026-08-19 (understory_dirty replaced by its published successor, `invalidation`)

## Context

Exedra's kernel boundary contract requires explicit dirty tracking: recorded
edit scopes produce a `ChangeSet` containing a `DirtySet` that tells higher layers which
faces, vertices, and corners need derived data recomputed. Exedra Ops consumes
these dirty sets for incremental workflows.

Dirty tracking is a shared concern across forest-rs projects, not specific to
mesh topology.

## Decision

Both Exedra and Exedra Ops use **`invalidation`** (crates.io, the published
evolution of the earlier `understory_dirty` git crate) for invalidation
primitives.

This is added as a workspace dependency:

```toml
invalidation = "0.2.0"
```

Exedra's `DirtySet` and `ChangeSet` types are built on top of
`invalidation::InvalidationSet` rather than rolling custom tracking. Exedra Ops'
`CacheDirtySet` wraps the same primitive with its own channel vocabulary.

Beyond the channelized set used today, `invalidation` also provides
dependency-graph propagation, channel cascades, and topological drains —
the substrate intended for future dependency-aware regeneration work
(constructive recipe caches, incremental extraction).

## Consequences

- Shared invalidation semantics across the forest-rs ecosystem.
- Exedra avoids reinventing set-tracking primitives.
- Published crates.io dependency with semver; no git pin remains in the
  workspace. Upgrade deliberately.

## History

- 2026-03-03: Accepted against `understory_dirty` (git-pinned to
  endoli/understory).
- 2026-08-19: `understory_dirty` evolved into the published `invalidation`
  crate (github.com/forest-rs/invalidation); workspace migrated (ticket
  exe-q2m6). API mapping was mechanical: `DirtySet` → `InvalidationSet`,
  `has_dirty` → `has_invalidated`; `Channel`, `mark`, `drain`, `clear`,
  `generation`, `is_empty` unchanged.
