# ADR-0004: Material extensions

## Status

Accepted

## Context

Core glTF materials cannot describe thin translucent leaves, glass, clear
coats, cloth sheen or explicit indices of refraction. Khronos covers these with
material extensions, and exedra_gltf refused every extension (ADR-0003), so
such materials had no way out of Exedra. Forest-rs materials are authored in
OpenPBR; glTF is a lossy projection of them. That projection belongs above
Exedra, in the tooling that owns OpenPBR values (dapple's packing profiles,
the `openpbr` crate). This crate only transports and validates what the
caller projects.

## Decision

1. **An explicit allowlist, validated like core fields.** Material
   `extensions` may contain `KHR_materials_diffuse_transmission`,
   `KHR_materials_transmission`, `KHR_materials_volume`, `KHR_materials_ior`,
   `KHR_materials_specular`, `KHR_materials_clearcoat`, `KHR_materials_sheen`
   and `KHR_materials_emissive_strength`. Each extension's fields are checked
   against its specification: unit ranges, non-negative and positive values,
   three-component colors, `ior` of `0` or at least `1`. An unlisted
   extension or field is `GltfError::UnsupportedMaterialField`; an invalid
   value is `GltfError::InvalidMaterial` naming the dotted path. Extensions
   nested inside an extension object are refused; `extras` are kept.
2. **Volume needs transmission.** `KHR_materials_volume` only describes the
   medium behind a transmitting surface. Without `KHR_materials_transmission`
   or `KHR_materials_diffuse_transmission` on the same material it has no
   effect, so it is refused rather than exported as a silent no-op.
3. **Extension textures are ordinary textures.** Their references join the
   fixed slot table after the four core slots, in declaration order. They are
   resolved through `MaterialResolver::resolve_texture`, shared by content,
   and checked per primitive for their UV set exactly like core textures.
   `clearcoatNormalTexture` takes `scale` and counts toward
   `GltfStats::normal_maps_without_tangents`, like `normalTexture`. Color
   extension textures are sRGB, the others linear; the channel each reads is
   the caller's contract, since the exporter checks signatures only.
4. **`KHR_texture_transform` on any texture reference.** `offset`,
   `rotation`, `scale` and `texCoord` are validated; its `texCoord`
   overrides the reference's own for viewers that support the extension. The
   per-primitive check covers both sets: the transform is used, not required
   (5), so a viewer without it samples the reference's own set, which must
   therefore be exported too.
5. **Used, not required.** Every extension a written material uses is listed
   in `extensionsUsed`, sorted. None goes in `extensionsRequired`: a viewer
   without one still loads the asset with core appearance. The exception is
   opt-in: `GltfExportOptions::require_texture_transform` lists
   `KHR_texture_transform` as required for callers who prefer refusing to
   load over wrong tiling.

## Consequences

- Leaves (diffuse transmission), glass and liquids (transmission, volume,
  ior), lacquer (clearcoat) and fabric (sheen) can be exported, with their
  maps, from any resolver.
- `GltfExportOptions` gains `require_texture_transform`; construction through
  `Default` and the constructors is unchanged.
- The allowlist grows only by explicit decision; `KHR_materials_unlit`,
  `KHR_materials_anisotropy`, `KHR_materials_iridescence` and texture-format
  extensions remain refused.
