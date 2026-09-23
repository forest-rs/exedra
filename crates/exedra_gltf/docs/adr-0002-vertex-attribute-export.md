# ADR-0002: Vertex attribute export

## Status

Accepted

## Context

`exedra_mesh` extraction now emits MikkTSpace tangents
(`TriMesh::tangents`) and caller-requested attribute streams
(`TriMesh::attributes`): second UV sets, colors, and caller-defined data such
as wind pivots. The exporter previously wrote only `POSITION`, `NORMAL`, and
`TEXCOORD_0`, so that data never reached a glTF consumer.

## Decision

1. **Tangents are exported whenever present.** A body compiled with
   tangents writes `TANGENT` (`VEC4` float, unit `xyz`, `w = ±1`). They are
   derived geometry with one fixed semantic, so no mapping is needed.
   Normals and tangents stay in local coordinates, and the coordinate
   conversion root transforms them coherently (ADR-0001).
2. **Streams need an explicit mapping.** `GltfExportOptions::attributes`
   borrows a list of `GltfAttribute { domain, stream, semantic, integers }`,
   built from the source layer's `AttrKey` and matched by domain and name,
   where
   `semantic` is one of:
   - `TexCoord(n >= 1)`: a `[f32; 2]` stream;
   - `Color(n)`: a linear `[f32; 3]` or `[f32; 4]` stream;
   - `Custom("_NAME")`: any kind of stream.

   Stream names are Exedra-internal (`corner.uv1`) and glTF semantics are
   a consumer contract, so the caller owns the translation. The options
   borrow the list, so they remain `Copy` and one value can serve many
   exports.
3. **Invalid mappings fail before any output.** The exporter rejects:
   - `TEXCOORD_0`, which is reserved for the primary UV set;
   - a custom name without a leading `_` (glTF requires one);
   - a semantic mapped twice;
   - an indexed set whose lower sets are unmapped. glTF requires indexed
     semantics to start at 0 and be contiguous, so `TEXCOORD` sets must be
     `1..=k` (set 0 is the primary UV) and `COLOR` sets `0..=k`.

   A stream whose type does not fit its semantic fails at the body that
   carries it.
4. **`u32` streams have an explicit, exact encoding.** glTF reserves
   `UNSIGNED_INT` for indices. `IntegerEncoding::Float` (the default) is
   exact up to `2^24`. `UnsignedShort` is exact up to `65535`, and each
   value is padded to four bytes with `byteStride: 4`, because vertex
   attribute elements must be four-byte aligned. A value outside the range
   is `GltfError::UnrepresentableAttribute`, never a silent rounding.
5. **Nothing is dropped silently.** Accessor order is fixed: position,
   normal, UV0, tangent, then mappings in list order. `GltfStats` counts
   attribute accessors and bytes. It also counts, per exported geometry,
   streams that no mapping names (not exported) and mappings whose stream
   the body lacks (attribute omitted from its primitives). Omission never
   creates a gap in an indexed run: a body missing its highest mapped set
   is exported without it and counted, but a body missing a lower set
   while a higher set is present fails with `GltfError::AttributeSetGap`,
   naming the part, body and semantic. Renumbering sets per body would
   silently change which stream a material's `texCoord` reads.

## Consequences

- Wind, color, and secondary-UV data authored in Exedra reach renderers
  without a custom exporter.
- `GltfExportOptions` gains a lifetime parameter and a field. The struct is
  `#[non_exhaustive]`, so construction through `Default` and the
  constructors is unchanged.
- Material `texCoord` selection and normal/occlusion textures are separate
  work; materials still read `TEXCOORD_0` only.
