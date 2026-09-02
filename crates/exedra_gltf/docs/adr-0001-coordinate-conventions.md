# ADR 0001: Coordinate conversion belongs at the scene root

## Status

Accepted.

## Decision

`export_gltf` preserves Exedra coordinates for compatibility. Callers that
author Z-up scenes opt into `GltfCoordinates::ZUpToYUp` through
`export_gltf_with_options`.

The exporter represents that conversion as one right-handed root-node
rotation: `(x, y, z) -> (x, z, -y)`. Item nodes remain children with their
authored transforms, and mesh buffers remain in their authored local space.
Consequently positions, normals, winding, instancing, and local accessor
bounds all follow the same hierarchy without duplicate conversion code.

The Basilica example opts in explicitly because its architectural model is
authored Z-up.

## Inspection contract

`GlbDocument` is a small std-only inspection boundary for tests. It validates
the GLB 2.0 header, declared length, chunk alignment and order, and JSON
syntax, then exposes semantic queries over the parsed document. It is not a
second glTF schema validator. Unknown optional schema entries are ignored, and
the parsed JSON remains available as the escape hatch for assertions outside
the common API.

`position_bounds` unions the extrema of `POSITION` accessors referenced by
mesh primitives. In accordance with the decision above, those values remain
in each mesh's authored local coordinates; item-node transforms and the
optional coordinate-conversion root are not applied. `triangle_count` likewise
describes indexed triangle primitives physically stored in the GLB, so shared
mesh instances count once rather than once per node.

Assembly instance metadata is copied directly into each item node's `extras`
object so exported files preserve the opaque annotations attached at the
structure head. The exporter owns `instancePath`, `partKey`, and `body`; those
three identity keys overwrite colliding opaque metadata rather than allowing a
node to misstate its exported identity.

### Migration note

`GltfError` gains the additive `InvalidGlb` and `InvalidGlbJson` variants. The
enum was already non-exhaustive, so downstream matches with the required
wildcard remain source-compatible. Instance metadata that was previously
absent from glTF output now changes the deterministic export bytes; consumers
that hash whole files should expect annotated assemblies to receive new asset
identities.
