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
