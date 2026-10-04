# Exedra Compile

Content-addressed geometry compilation and retained queries, independently of
assembly or scene composition. Supply borrowed `PartInput` records containing
caller-owned diagnostic keys and recipes or baked meshes. The service owns no
hierarchy, occurrence placements, appearance binding or persistent scene identity.

`PartCompiler::compile_parts` produces triangle buffers and complete recipe
reports. `compile_snapshot` additionally retains topology and source maps for
sections, semantic surface inspection, attachments and clearance queries.
`GeometrySnapshot` contains geometry only; callers retain placement separately.

Inputs use snapshot-local `PartId` handles. `CompiledParts::validate_for` checks
counts and source content without evaluating geometry. Content and policy hashes
retain their existing encodings; moving ownership does not invalidate compiled
content identities. Diagnostic keys are excluded from sharing.

Cloned geometry snapshots share their tables in constant time. Fresh snapshots
create fresh selection scopes even when geometry is reused. Snapshot bodies,
workplanes and inspected surface evidence survive compiler eviction and remain
`Send + Sync`. Re-resolve authored attachment intent after geometry edits.

`GeometrySnapshot::part` retains one input without retaining unrelated input
buffers. Body handles and per-part views share the parent snapshot's query scope.

The compiler defaults to explicit caller-controlled retention. Use
`with_capacity` for an entry budget, `clear_cache` to release entries, or
`mark_part_changed` to evict content from the last successful input ordering.
A single pending set replaces channel-based invalidation. Cache eviction never
invalidates held snapshots. Counters distinguish compilation, hits, evictions,
triangle extraction and attribute/tangent fallbacks; reports on cache hits retain
the original evaluation measurements. Render-only entries retain no topology;
upgrading one to a query snapshot evaluates once.

Existing Assembly compilation is an adapter over this service. Standalone
callers can use these records and snapshots directly without creating an
`Assembly` or loading LayerStack. The crate is `no_std` with `alloc` and requires
pointer-width atomics; the `std` feature forwards host support to dependencies.

## License

Apache-2.0 OR MIT
