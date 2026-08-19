# exedra_gltf

glTF 2.0 export for Exedra assembly render lists: instances become nodes,
per-region index ranges become primitives with real material bindings, and
instance paths ride in `extras`. Single-file output with an embedded
base64 buffer; deterministic byte-for-byte.
