---
id: sg-ty1b
status: closed
deps: []
links: [bsl-6ihj, set-mnga, set-mpbt]
created: 2026-08-31T17:46:35Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Generate nave-truss stations from exact setout

Replace the Basilica ruin's floating Cambium truss repeats with exact, stable station fragments produced by `setout_generate`. Preserve the authored west-roof loss and expand each surviving station into the existing seven fitted `joiner_timber` members.

## Design

Fence: `setout_generate` owns station identity, omission state, exact coordinates, and fragment deltas; Basilica architecture owns expanding a station into timber members and lowering once to assembly. `joiner` and `joiner_timber` remain unchanged. Derive west endpoints and the east singleton from named exact setout quantities, model the west loss as a semantic omission, bind invalidation from generated labels, and retain geometry/artifact parity except for the intentional identity migration. Plan: add exact truss datums; generate fragments; migrate assembly and bindings; pin omission, identity, delta, geometry, and artifact behavior; document the existing setout ADR; run the full Definition of Done and render review.

## Acceptance Criteria

No floating truss repeat arithmetic remains; every station coordinate is exact until the assembly adapter; the missing west station is an explicit non-orphan omission; each surviving station expands deterministically to the complete seven-member fitted truss; warm reconfiguration reports station-level changes and binds all generated member identities; tests explain their behavioral intent; the owning ADR and migration note describe identity changes; accepted geometry is unchanged; full workspace Definition of Done passes.


## Notes

**2026-08-31T18:20:45Z**

Implemented the second production `setout_generate` consumer. `BasilicaPremises` now authors `nave_truss_bays`; the exact network derives west truss anchors and the east singleton datum; a semantic `interior/000002` omission preserves the ruin without ordinal rebinding; generated west stations expand through an example-local adapter into the seven existing `joiner_timber` members. The east singleton intentionally remains an exact named datum rather than a degenerate linear invocation. `setout_generate` and `joiner_timber` remain consumer-neutral; Basilica drops its Cambium dependency. `setout_joiner` now owns rational-iota lowering and uses `joto_constants::length::i128::METER` as the unit source. Migration: `PlanSection`, `BasilicaSetout`, and `BasilicaReconfiguration` expose truss station data/deltas; ordinal truss paths become semantic `start`/`interior`/`end` names; identity-bearing fingerprints change. The accepted OBJ payload is byte-identical to `origin/main` after excluding only `o`/`g` identity lines; the new assembly fingerprint is `79f62c3d1e8abaa2f2b2b1ae87b0c638` and OBJ signature is `42908f1a6dc9d4a3`. Validation passed: `typos`; `taplo fmt --check`; `cargo fmt --all -- --check`; `git diff --check`; `cargo check` without default features for `setout_generate` and `setout_joiner`; warnings-denied workspace all-target/all-feature Clippy; all-feature workspace tests; workspace all-feature rustdoc without dependencies; focused default and non-default topology, omission/orphan, exact-delta, full-member expansion, identity-order, geometry, and determinism tests; release Basilica run with 118 instances, 7,550 triangles, and zero diagnostics. No new ADR was added: the owning setout ADR 0001 was amended with the boundary, migration, and tradeoffs.
