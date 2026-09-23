# ADR-0003: Material textures

## Status

Accepted

## Context

The material resolver accepted only `pbrMetallicRoughness.baseColorTexture`
sampling `TEXCOORD_0`. Normal, occlusion and metallic-roughness maps were
rejected, so a normal-mapped or channel-packed asset could not leave Exedra
through glTF even though extraction now emits tangents and secondary UV sets
(ADR-0002).

Exedra has no shared material model. Resolvers return glTF JSON and own their
material representation. The long-term model for forest-rs materials is
OpenPBR; its projection to glTF (with the relevant KHR extensions) belongs
above Exedra, in the material tooling that owns OpenPBR values. This decision
covers only the glTF transport subset this crate validates.

## Decision

1. **Four texture references are supported:**
   `pbrMetallicRoughness.baseColorTexture`,
   `pbrMetallicRoughness.metallicRoughnessTexture`, `normalTexture` with a
   finite `scale`, and `occlusionTexture` with `strength` in `[0, 1]`. Each
   takes `index`, optional `texCoord`, and `extras`; any other field is
   `GltfError::UnsupportedMaterialField`, and an invalid value is
   `GltfError::InvalidMaterial` naming the field. `emissiveTexture` and all
   extensions remain unsupported.
2. **Resources resolve in a fixed order:** materials in emission order, and
   within a material base color, metallic-roughness, normal, occlusion. Each
   caller-local index resolves once; image bytes and samplers are shared as
   before, so an occlusion map packed into the metallic-roughness texture
   (ORM) embeds one image.
3. **`texCoord` is checked per primitive, against what that primitive
   exports.** Set 0 requires finite UVs on every textured corner, as base
   color did (`GltfError::MissingTextureCoordinates`). Set `n >= 1` requires
   the primitive's geometry to export `TEXCOORD_n` through an attribute
   mapping; otherwise the export fails with
   `GltfError::MissingTextureCoordinateSet`, naming the material, texture,
   set, part, body and region. A material may sample different sets on
   different textures.
4. **Encodings and channels are the resolver's contract.** glTF fixes them
   and the exporter checks only image signatures, so they are documented on
   `Texture::image`: base color sRGB; metallic-roughness linear with
   roughness in G and metalness in B; occlusion linear in R (so an ORM image
   packs all three); normal linear tangent-space XYZ with +Y up. Set
   `n >= 1` is checked for export, not for authored coverage, unlike set 0.
5. **Normal maps without tangents are allowed and counted.** glTF directs
   consumers to derive MikkTSpace tangents when `TANGENT` is absent, so such
   files are valid. They are still a portability risk (a consumer's tangents
   may differ from the baker's), so `GltfStats::normal_maps_without_tangents`
   counts the primitives, and compiling with tangents removes it.

## Consequences

- Resolvers can export complete metallic-roughness materials with normal and
  occlusion maps, on the primary or a secondary UV set.
- Base-color textures may now sample `TEXCOORD_n`, under the same checks.
- Validation moves from "base color at set 0" to a table of texture slots, so
  adding `emissiveTexture` or extension textures later is a table entry plus
  its parameter rule.
