---
id: em-u8k7
status: closed
deps: []
links: []
created: 2026-08-31T05:27:28Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Adopt exact measurement domains in setout and joiner

Replace setout's signed type named Length with exedra_measurements::Length for positive sizes and Offset for coordinates and differences. Migrate authored Joiner measurement thresholds while retaining floating geometry results and numerical tolerances.

## Design

Setout owns propagation and quantization policy; exedra_measurements owns value domains. Add explicit cross-domain relations where lengths locate offsets, update Point3 to Offset components, and bump the canonical fingerprint schema. Joiner lowers exact authored thresholds only where they meet analytic geometry.

## Acceptance Criteria

Public quantities cannot represent negative Length values; signed coordinates use Offset; setout_joiner performs the sole exact-to-f64 lowering; persisted fingerprint/API migration is documented; representative call sites and regression tests state their intent; no_std and full workspace Definition of Done pass.


## Notes

**2026-08-31T05:27:55Z**

Plan — Goal: one exact measurement vocabulary from authored setout values through Joiner parameters. Non-goals: replacing floating geometry vectors, derived measurements, or tolerances; adding unit inference; preserving the obsolete signed-Length API. Steps: (1) strengthen shared arithmetic/conversion seams, (2) migrate setout domains, relations, fingerprints, and consumers, (3) migrate authored Joiner thresholds where exact inputs are available, (4) document migration and validate no_std/workspace/render oracles. Risks: reverse multi-way methods can leave the positive Length domain; Point3/fingerprint encodings change; exact-to-float lowering could accidentally spread outside setout_joiner.

**2026-08-31T07:41:33Z**

Completed the exact-domain retrofit. exedra_measurements now owns strictly positive Length, signed Offset, full-range cross-domain arithmetic, and exact Angle limits. setout uses Offset for coordinates and Length for sizes, adds semantically named relations, records i128 root selections, and bumps fingerprint schema to version 2. setout_joiner is the exact-to-f64 boundary. joiner uses typed authored length and angle limits while keeping analytic geometry and derived thresholds explicitly in meters or radians. Tradeoffs: persisted version-1 setout identities must be rebuilt; old unit-ambiguous rejection variants and direct contact-overlap field access intentionally break. Validation: cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features; cargo doc --workspace --all-features --no-deps; no-default-feature tests for affected core crates; cargo fmt --all -- --check; taplo fmt --check; typos; git diff --check. All passed. The branch and origin/main basilica exports matched exactly at assembly fingerprint 175332234ae1c211886b3b447c760f26 and OBJ signature 72cd48bba9b5fb21; the structure-lab signature remained ca8568e71945d5e3.
