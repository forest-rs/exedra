---
id: ec-0sds
status: closed
deps: []
links: []
created: 2026-09-02T11:20:10Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Allow imported meshes under sanctioned mirrors

Evaluate a MeshImport beneath Mirror or another reflecting constructive transform while preserving topology, winding, mesh attributes, source provenance, fidelity, and cache identity.

## Design

Move orientation repair into the existing mesh transformation seam: after applying a placement with negative linear determinant, rebuild face loops in reverse order and remap face, edge, and corner attributes without changing the import table. Keep reflecting Instance placements refused because instance reuse has a distinct proper-placement invariant.

## Acceptance Criteria

Mirror over a watertight imported solid evaluates as one exact deep-valid body with outward winding and unchanged imported provenance; face regions, edge seams and sharpness, and corner UVs survive; direct non-uniform transforms remain supported; reflecting instances remain a typed refusal; deterministic cache and workspace gates pass.

## Notes

Implemented evaluator-owned orientation repair for reflecting `MeshImport`
placements. A negative composed determinant rebuilds topology with reversed
face loops; built-in face, undirected-edge, vertex, and face-corner attributes
follow their semantic owners, and authored normals use inverse-transpose
transport. Sparse attribute absence remains absent. The source import table is
unchanged, while the output source map is rebuilt and pinned as
`Feature::Imported`.

Reflecting `Instance` placements remain refused, and arbitrary private
attribute layers remain outside the constructive import contract. The behavior
change advances `EVAL_SCHEMA_VERSION` from 7 to 8.

Regression coverage includes closed and open imports, reflecting and proper
non-uniform transforms, attribute ownership, exact bounds and signed volume,
source provenance, deep validation, and cold/warm cache determinism.

Validation passed:

```sh
typos
cargo fmt --all -- --check
taplo fmt --check
cargo check -p exedra_constructive --no-default-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
```
