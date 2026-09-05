---
id: ec-55ov
status: closed
deps: []
links: []
created: 2026-09-05T02:28:05Z
type: bug
priority: 1
assignee: Bruce Mitchener
external-ref: cxc-dmq.3
---
# Correct n-ary constructive intersection

N-ary constructive intersections currently union operands 2..n before intersecting with operand 1, so three or more operands can return volume outside later operands. Preserve union and A-minus-union(rest) difference semantics, and represent empty common intersections explicitly.

## Design

Reproduce in exedra_constructive with nested boxes, then fold Intersection operands pairwise with Intersection while leaving Difference's unioned tail intact. Preserve report/provenance/cache behavior, advance EVAL_SCHEMA_VERSION from 10 to 11, and record the fold and empty-result contract in a crate-local ADR.

## Acceptance Criteria

Nested, overlapping non-nested, empty-common, permutation, four-operand, binary, union, difference, and cold/warm cache cases have semantic geometry assertions; focused constructive/boolean/assembly/export tests pass; required fmt/clippy/doc/typos/taplo checks pass.


## Notes

**2026-09-05T02:44:51Z**

Implemented same-operation left folds for n-ary union/intersection while preserving A-minus-union(rest) difference. Composed Boolean face provenance through ID-keyed intermediate maps, including sparse face IDs. Preserved exact empty results as cacheable/nestable zero-face bodies and made glTF retain instance identity while omitting invalid zero-count geometry payloads. Advanced EVAL_SCHEMA_VERSION 10 -> 11 and refreshed constructive and assembly identity pins. Added ADR-0006. No dependency, unsafe, or public API additions. Validation passed: cargo test -p exedra_constructive --all-features (192 passed, 2 ignored, plus 5 doctests); cargo test -p exedra_mesh --all-features boolean:: (94 passed); cargo test -p exedra_assembly --all-features (31 passed, 1 ignored, plus 2 doctests); cargo test -p exedra_gltf --all-features (13 passed, plus 1 doctest); cargo clippy for constructive/assembly/gltf with all targets/features and -D warnings; cargo doc for those crates with all features/no deps; cargo check constructive/assembly no-default-features; cargo fmt --all -- --check; taplo fmt --check; typos; git diff --check. Existing typed Boolean refusals for undecidable face-chain configurations remain unchanged.
