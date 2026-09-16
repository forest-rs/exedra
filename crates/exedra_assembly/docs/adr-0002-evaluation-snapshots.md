# Optional evaluation snapshots

Status: accepted

`PartCompiler::compile_snapshot` retains the same evaluated bodies used to
extract render buffers, plus a frozen render list with occurrence identity,
placements, and resolved materials. The constructive operations remain owned by
`exedra_constructive`; see its [retained plane operation contract](../../exedra_constructive/docs/adr-0013-retained-plane-operations.md).

The compiler shares immutable evaluated parts through `Arc`. Snapshot and body
handles outlive cache eviction and support thread transfer. Constructive `Rc`
ownership stays internal to evaluation: unique bodies move into the snapshot;
already shared bodies are cloned without reevaluation. Baked meshes retain
imported provenance rather than invented recipe identity.

The existing `compile_parts` path does not retain topology. Opting in after a
render-only cache hit requires one evaluation to populate topology; later calls
reuse it. An opt-in cache entry may subsequently serve render-only calls without
forcing topology into their returned `CompiledParts`.

Queries use part-local geometry. Captured render items supply occurrence world
placements. Snapshot workplanes own their body and check snapshot, part and body
identity, in addition to mesh revision. Snapshot clones share selection scope;
a new compilation has a new scope even when all geometry was cached. Raw mesh
IDs and workplanes are not serialized as persistent feature attachments.

This is additive: render-only callers need no migration. Callers wanting both
rendering and queries use `compile_snapshot`, then `compiled`, `render`, and
`body`; querying never reruns recipe evaluation.
