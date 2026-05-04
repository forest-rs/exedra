---
id: exe-rxny
title: Expand exedra_testkit: golden dump schema and fixture builders
status: closed
deps: [exe-lopy]
links: []
created: 2026-03-03T07:07:17Z
type: Expand exedra_testkit: golden dump schema and fixture builders
priority: P2
assignee: Bruce Mitchener
---
# Expand exedra_testkit: golden dump schema and fixture builders

Define the exedra_testkit scope more concretely. The testkit provides: (1) golden dump format using RON or JSON with ordered lists (not maps) for determinism, (2) fixture builder helpers that construct common test meshes via exedra_primitives, (3) snapshot comparison utilities (dump mesh to golden format, compare against stored golden file), (4) debug visualization helpers (optional). Golden format must be human-readable and diff-friendly.

## Design

Golden format: RON preferred (Rust-native, concise), JSON as alternative. All collections serialized as ordered lists, never unordered maps. Schema includes: vertex count, face count, positions as ordered list, face loops as ordered list of vertex index lists, attribute layers present, selection contents. Fixture builders: thin wrappers around exedra_primitives with fixed params for common test cases (unit_quad, unit_box, capped_cylinder_8, etc). Snapshot utilities: dump_golden(mesh) -> String, assert_golden(mesh, expected_path). The testkit is std-only (file IO, pretty printing).

## Acceptance Criteria

Golden dump format defined and documented. dump_golden produces deterministic output. Fixture builders for quad, box, cylinder, sphere with canonical params. assert_golden compares mesh against stored golden file. Format is human-readable and diff-friendly. testkit depends on exedra_primitives and std.


## Notes

**2026-05-04T16:59:17Z**

Expanded exedra_testkit with a deterministic exedra-mesh-golden-v1 dump format, string and std-gated file-backed assertions, and ordered optional GoldenSelection payloads. Added primitive-backed canonical fixtures for unit_quad, unit_box, capped_cylinder_8, and sphere_mesh/uv_sphere_8 while keeping the existing lightweight fixtures. Updated built-in attribute dumps for vertex sharpness and corner normal overrides, documented the golden schema and fixture set in the crate README, and wired exedra_primitives as an internal workspace dependency without adding external dependencies. No new ADR: this is test infrastructure; the durable schema and scope are documented in exedra_testkit/README.md and rustdoc. Validation: cargo fmt --all; cargo test -p exedra_testkit --all-features; taplo fmt crates/exedra_testkit/Cargo.toml; cargo clippy -p exedra_testkit --all-targets --all-features -- -D warnings; cargo doc -p exedra_testkit --no-deps; typos crates/exedra_testkit .tickets/exe-rxny.md; cargo fmt --all --check; git diff --check.
