# Level-of-detail chains on parts

Status: accepted

## Context

Real-time assets ship several levels of detail: a full mesh, reduced meshes,
and finally cards or impostors. Before this change the only representation
was separate parts with ad-hoc naming, so nothing tied the levels together,
flattening could not say which level to draw, and interchange lost the
relationship.

## Decision

1. **A chain belongs to a part.** `Assembly::set_part_lods(part, levels)`
   records an ordered chain, finest first, where level 0 is the part itself
   and lower levels are ordinary parts. Every placement of the part, instance
   or placement set, carries the chain, with no change to the instance model.
   An empty chain clears it.
2. **Levels switch by screen coverage.** Each `LodLevel` has a
   `min_coverage`: the smallest projected size, as a fraction of the viewport
   height, at which the level is drawn. A level is drawn from its
   `min_coverage` up to the previous level's, and fades out over `crossfade`
   below its threshold. Below the last threshold the occurrence is not drawn,
   so a last level of `0.0` is never culled. Thresholds are finite,
   non-negative and strictly decreasing, and a crossfade band may not reach
   past the next threshold (compared in `f64`, so a band may meet it
   exactly). Exedra records the policy; renderers measure coverage.
3. **Chains do not nest.** A lower-level part cannot have its own chain, a
   part with a chain cannot be a lower level elsewhere, and a part appears at
   most once per chain. Several chains may share a lower-level part, such as
   one impostor card for similar trees.
4. **Bindings carry by slot name.** Instance and placement-set bindings are
   keyed by the owning part's slots; a lower level's slot takes the binding of
   the owning slot with the same name, else the owning part's default for that
   slot, else its own part default. A chain is the same part at less detail,
   so the owner's materials win. Levels can therefore have different slot
   tables.
5. **Flatten emits the base level unless asked.** `FlattenOptions::lods`
   defaults to `LodEmission::BaseLevel`: only level 0, so consumers without
   level-of-detail support draw full detail and never overlap levels.
   `LodEmission::AllLevels` emits every level. Either way, drawables of a
   chained part carry a `LodTag` with the level, the chain length, the
   coverage range and the crossfade.
6. **Chains are content identity and interchange.** Levels compile and cache
   as ordinary parts. `assembly_fingerprint` appends chains only when present;
   the `exedra-assembly` interchange carries each part's `lods`; `append`
   copies a used part's lower levels with it.

## Consequences

Renderers opt in to level selection explicitly. `RenderList::triangle_count`
counts every emitted level, so it measures the base level by default and the
whole chain under `AllLevels`.

`Assembly::resolved_level_material`, taking an `Occurrence` (instance or
placement set), is the single material rule for lower levels, used by
`flatten` and by consumers that walk the hierarchy themselves.

glTF export writes only the base level by default, which is a valid asset
without level-of-detail support. With `GltfLods::MsftLod`, it writes chains
as `MSFT_lod` with `MSFT_screencoverage` (`exedra_gltf` ADR-0006).
