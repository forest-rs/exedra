# Placement scale: bounds policy, sibling index and placement sets

Status: accepted

## Context

Vegetation and other scatters place one part tens of thousands of times.
Measured on the `assembly_wind_tunnel` AS-1 scenario (a 2,562-vertex part
placed under one frame), the previous assembly did not scale:

| placements | build | flatten |
|---|---|---|
| 10,000 | 81 ms | 33 ms |
| 100,000 | 11,141 ms | 338 ms |

Building was quadratic because every `add_instance` scanned its siblings for
a duplicate key. Flattening transformed every emitted vertex of every placed
body to compute exact world bounds, O(placements × vertices). Each placement
was also a heap node with its own key string, binding table and metadata.

## Decision

1. **World bounds follow an explicit `BoundsPolicy`.** The default,
   `TransformedBox`, transforms each compiled body's part-local box: the
   tightest box around its transformed corners. Part-local bounds are
   computed once per compiled body per flatten. It is exact under
   translation, axis permutation and axis-aligned scale (bit for bit under
   translation), and never smaller than the placed geometry under rotation or
   shear; a sphere rotated about its axis gains up to a factor of √2 in the
   rotated plane. `BoundsPolicy::Exact` keeps the previous per-vertex result
   through `flatten_with`. Box bounds are the default because world bounds
   feed culling and budgets, where conservative is correct and per-vertex
   cost is not.
2. **Sibling keys are indexed.** A `HashTable` of handles, keyed by parent and
   key and read back from the owning records, makes duplicate checks O(1) and
   `resolve_path` O(depth). It stores no key strings and is never iterated,
   so its seeded hasher cannot affect output order.
3. **Placement sets are the representation for scatters.** A
   `PlacementSet` places one part under one parent as a placement array, with
   optional per-placement seeds and linear RGBA tints, one binding table and
   one metadata table. A set places its part at least once; an empty
   placement list is `AssemblyError::EmptyPlacementSet`. Sets share their
   parent's key namespace with instances. Placement `i` is addressed as
   `PlacementPath` `parent/key#i`; to keep that unambiguous, keys and pattern
   key fragments may no longer contain `#`.
4. **`flatten` emits sets as `RenderBatch`es.** One batch per compiled body
   carries every world placement and per-placement bounds, and resolves
   materials once. World placements, seeds and tints are shared `Arc`
   slices: every batch of a set shares one placement array, and seeds and
   tints are the set's own storage, so a renderer consuming only the
   `RenderList` has per-placement appearance data without copies. `RenderList` accounting (`triangle_count`, `bounds`,
   `placed_body_count`) includes batches. Consumers must handle both lists.
5. **Consumers that cannot represent sets refuse them.** None drop
   placements silently. glTF export writes sets as addressed nodes or
   `EXT_mesh_gpu_instancing` batches (`exedra_gltf` ADR-0006).
   The `exedra-assembly` interchange format carries sets as a
   `placement_sets` list after the instances.
6. **Identity covers sets.** `assembly_fingerprint` appends set records only
   when present, and `append` copies
   sets with their parents, prefixing and composing root-level sets like root
   instances. `append_selected` selects instances only: sets travel with
   their selected parents, and root-level sets are always copied, because a
   set is a scatter of its parent's content rather than an independently
   authored node.

## Consequences

AS-1 after this change, with default bounds:

| placements | mode | build | flatten |
|---|---|---|---|
| 10,000 | instances | 1.5 ms | 1.5 ms |
| 10,000 | set | < 0.1 ms | 0.1 ms |
| 100,000 | instances | 11 ms | 16 ms |
| 100,000 | set | 0.2 ms | 1.4 ms |

Build times exclude generating the part's mesh, which is not assembly work.

Exact bounds still cost about 330 ms at 100,000 placements, as before.

`RenderItem::world_bounds` is no longer exact by default; callers that relied
on exact placed bounds use `flatten_with(.., BoundsPolicy::Exact)`. Keys
containing `#` are now rejected, including when loading interchange. `RenderList` gained `batches`, and
consumers that iterate only `items` do not see placement sets.

Per-placement bindings, metadata and paths beyond the index are
intentionally absent: a placement that needs its own identity or materials
should be an instance.
