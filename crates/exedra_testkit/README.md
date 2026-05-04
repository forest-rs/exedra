# exedra_testkit

Deterministic test helpers for Exedra:

- fixture mesh generators (`fixtures`)
- mesh golden dumps and TriMesh golden snapshot helpers (`golden`)
- topology/attribute/OBJ debug dumps (`dump`)

This crate is intended for tests and examples.

## Golden Mesh Format

`dump_golden(mesh)` emits `exedra-mesh-golden-v1`, a line-oriented,
human-readable format with only ordered lists:

- `positions`: live vertices in stable vertex traversal order, with floats as
  IEEE-754 bit patterns.
- `face_loops`: live faces in stable face traversal order, with source vertex
  indices in loop order.
- `attributes`: built-in authored layers in deterministic domain order.
- `selections`: optional named face/edge selections emitted by
  `dump_golden_with_selections`.

Use `assert_mesh_golden(mesh, expected)` for string comparisons, or
`assert_golden(mesh, path)` with the `std` feature to compare against a file.

## Fixtures

Core fixtures include `triangle_mesh`, `quad_mesh`, `tetrahedron_mesh`, and
`grid_mesh`. Primitive-backed canonical fixtures include `unit_quad`,
`unit_box`, `capped_cylinder_8`, and `sphere_mesh`.
