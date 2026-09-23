# ADR-0005: GPU instancing

## Status

Accepted

## Context

A forest places one tree part thousands of times. The exporter wrote one node
per logical instance, each with its own matrix: correct and inspectable, but
large, and slow for engines that must rediscover the repetition. The
`EXT_mesh_gpu_instancing` extension lets one node carry per-instance
`TRANSLATION`, `ROTATION` and `SCALE` accessors for a mesh.

## Decision

1. **Opt-in.** `GltfExportOptions::instancing` defaults to
   `GltfInstancing::Nodes`, which writes exactly what earlier versions wrote.
   `GltfInstancing::GpuInstancing` enables batching.
2. **What batches.** A batch is every leaf instance (no children) of one part
   that has geometry, shares its parent, and resolves every body's regions
   to the same materials. Batches of one keep their node. Batching never
   crosses parents, so the hierarchy and its transforms stay as authored; a
   batch node is a child of the shared parent, or a scene root.
3. **One node per body.** A part with several bodies gets one instanced node
   per non-empty body, all referencing one set of transform accessors.
4. **Placements must decompose.** Each local placement is split into
   translation, a unit rotation quaternion (`exedra_math::Quat`) and a
   positive per-axis scale. Axes that are degenerate or not perpendicular
   (`|cos|` above `1e-6`, which admits rigid placements that passed through
   `f32` while rejecting deliberate shear) have no such decomposition: the
   instance keeps its matrix node and counts in
   `GltfStats::unbatched_sheared_instances` when it would otherwise have
   joined a batch.
5. **Reflections keep their node.** glTF defines front faces by the
   determinant of the node transform, but instanced renderers commonly apply
   one winding to all instances. A negative determinant therefore keeps its
   matrix node, counted in `GltfStats::unbatched_mirrored_instances` when it
   would otherwise have joined a batch, rather than risk inside-out instances.
6. **Identity stays inspectable.** A batch node's `extras` carry `partKey`,
   `body` and an `instances` table with each member's `instancePath` and
   metadata (the shared `partKey` is not repeated), in instance order, which
   is also accessor order.
   `GlbDocument::instancing_components` reads the accessors back.
7. **Required when used.** No fallback describes batched instances, so the
   extension is listed in both `extensionsUsed` and `extensionsRequired`.
8. **Precision.** The extension stores transforms as `FLOAT` accessors, so
   batched translations have `f32` precision, unlike matrix nodes.

## Consequences

- Repeated props and vegetation export as one draw-friendly node per mesh
  and parent, with per-instance identity preserved in extras.
- Assembly placement sets and LOD chains (proposed separately) will export
  through this same path once they land: a placement set is already a batch.
- Exports with instancing enabled require viewer support for the extension.
