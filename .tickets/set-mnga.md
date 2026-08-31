---
id: set-mnga
status: closed
deps: []
links: [set-qckq, set-mpbt]
created: 2026-08-31T16:11:32Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Generate stable exact topology fragments

Add the `setout_generate` sibling crate and prove it by replacing the basilica buttress repeat arithmetic. The crate expands resolved exact inputs into immutable labeled occurrence fragments; assembly lowering remains an adapter concern.

## Design

`setout_generate` is `no_std` + `alloc`, depends only on `setout`, and gives invocations and generated items stable semantic keys. Linear station generation retains non-integral subdivisions as exact rational iota coordinates, accepts explicit labeled omissions, fingerprints its fragment, and compares re-expansions by key so count edits report retained, added, removed, moved, and orphaned identities without endpoint rebinding.

## Acceptance Criteria

A neutral fixture proves stable identity, deterministic fingerprints, strict validation, orphaned overrides, bounded work, full-domain exact arithmetic, and warm/fresh incremental equivalence; `basilica_ruin` obtains buttress station labels and exact rational X coordinates from the generated fragment and lowers once at assembly instantiation. Semantic endpoint names intentionally re-baseline identity-bearing assembly and export fingerprints while preserving the accepted placement geometry; the owning setout ADR and crate docs explain the boundary; the full repository Definition of Done passes.

## Notes

**2026-08-31T16:48:43Z**

Implemented `setout_generate` as a bounded `no_std` + `alloc` sibling with shared semantic-key validation, exact rational-iota linear stations, normalized omission overrides, deterministic fingerprints, orphan retention, and key-based re-expansion deltas. The basilica setout now derives exact buttress anchors, feeds one generated fragment to both invalidation and assembly lowering, and exposes the fragment and delta on reconfiguration. Stable `start`/`end` identities deliberately replace ordinal buttress paths; this re-baselines identity-bearing assembly/export fingerprints instead of retaining endpoint rebinding. The generated OBJ geometric payload is byte-identical to `main` when object/group identity lines are ignored. Validation passed: `typos`; `taplo fmt --check`; `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `cargo doc --workspace --all-features --no-deps`; release basilica run with 118 instances, 7,550 triangles, zero diagnostics, assembly fingerprint 3a0a91c3dadbfc875d1c91631f204c94, and OBJ signature fbd4fe216f8a8649.
