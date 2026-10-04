# Caller-owned export

This example uses facade features `gltf,serde` with defaults disabled. It
serializes and restores a construction recipe, installs immutable source
identity, compiles geometry, and prepares an application hierarchy and resolved
materials for binary glTF export. It validates the exported geometry and repeat
output. Neither Assembly nor LayerStack is needed.

```sh
cargo run -p caller_owned_export
```
