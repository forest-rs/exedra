# ADR-0006: Placement sets and level-of-detail chains

## Status

Accepted

## Context

`exedra_assembly` gained placement sets (one part at many placements, with
optional per-placement seeds and tints) and level-of-detail chains on parts
(`LodLevel`: a part per level, a minimum screen coverage and a crossfade
band). Export refused sets and wrote only the base level of a chain. Both
have natural glTF forms: `EXT_mesh_gpu_instancing` for sets (ADR-0005) and
`MSFT_lod` with `MSFT_screencoverage` for chains.

## Decision

1. **Sets always export; nothing is dropped.** Under the default
   `GltfInstancing::Nodes`, every placement becomes a node named and
   addressed `set-path#index`, with its matrix, the set's metadata, and its
   seed and tint in `extras`. Seeds are decimal strings, because JSON numbers
   cannot hold every `u64` exactly.
2. **Under `GltfInstancing::GpuInstancing`, a set is one batch.** Every
   decomposable placement joins, in placement order, even when there is
   only one: the set is authored repetition. Mirrored and sheared
   placements keep their own `set-path#index` node, as instances do
   (ADR-0005), and are counted separately from instances in `GltfStats`.
   The batch identity is `extras.setPath` and `extras.placementCount`, plus
   `extras.placementIndices` only when some placements were left out.
   Storing a count instead of one table row per placement keeps large sets
   small.
3. **Seeds and tints are custom instance attributes.** `_SEED` is a `VEC4`
   of `UNSIGNED_SHORT`: the seed's four 16-bit words, least significant
   first. That is exact for all `u64` and uses a vertex-attribute-legal
   component type. `_TINT` is a `VEC4` of `FLOAT`, linear RGBA. Both are
   written only when the set has them, with the same count as the
   transforms.
4. **Instances that own sets keep their node.** A set needs its parent's
   node, so such instances never batch.
5. **Chains are opt-in through `GltfLods::MsftLod`.** The default,
   `GltfLods::BaseLevel`, writes exactly what earlier versions wrote. With
   `MsftLod`, an occurrence whose part has a chain of at least two levels
   moves its geometry to a child node `… [lod 0]` with an identity
   transform. That node carries `MSFT_lod.ids` for its lower-level nodes,
   `… [lod k]`, which also have identity transforms and are referenced by
   nothing else. The logical node keeps its transform, children and
   identity. The spec does not say whether a viewer replaces the node or
   swaps its meshes, so this layout is correct either way.
6. **Thresholds.** `extras.MSFT_screencoverage` lists every level's
   `min_coverage`, one per level, matching the spec's example of three
   values for three levels, where the last value is the cull threshold.
   Crossfade bands, which `MSFT_lod` cannot express, go in
   `extras.exedraLodCrossfade`. Values are the shortest decimal form of the
   authored `f32`.
7. **Instanced chains share transforms.** A batch whose part has a chain
   becomes a group node carrying the batch identity and `MSFT_lod`. Its
   children are the per-body instanced nodes of level 0. Each lower level is
   a group of per-body instanced nodes that reference the same transform
   accessors. Batching then groups instances by the material resolution of
   every written level.
8. **Materials follow the assembly rule.** Lower levels resolve materials
   through `Assembly::resolved_level_material`, the same function `flatten`
   uses: the occurrence's binding of the same-named slot, then the owning
   part's default, then the level part's own default.
9. **Extension listing.** `MSFT_lod` is listed as used, never as required:
   viewers without it draw level 0. `EXT_mesh_gpu_instancing` is required
   whenever any instanced node is written.

## Consequences

- Scattered vegetation exports as a few instanced nodes per set, with exact
  per-placement seeds for shader variation.
- Placement paths stay addressable either as nodes or as
  `setPath` + placement index.
- Under `MsftLod`, the output has more nodes, and occurrences with chains
  gain a geometry child. That's the price of an unambiguous layout.
- `GlbDocument::lod_node_names` reads chains back.
