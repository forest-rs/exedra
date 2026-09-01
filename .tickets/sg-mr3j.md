---
id: sg-mr3j
status: closed
deps: []
links: [set-mnga, sg-ty1b, set-mpbt]
created: 2026-09-01T00:55:38Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Generate exact Basilica arcade bays

Replace the Basilica ruin's independently calculated floating arcade and clerestory opening centers with exact, semantically identified bay fragments. Preserve the accepted default geometry while making topology edits and the open crossing gap explicit.

## Design

Fence: setout_generate owns bounded linear-bay identity, exact bay extents/centers, fingerprints, omissions, and key-based deltas. Basilica setout owns the three building-specific invocations (outer wall, west nave, east nave) and their relationship to resolved plan datums. Basilica architecture lowers generated centers into masonry profiles; setout_generate does not learn arches, walls, or constructive recipes. Add one no_std bay-distribution shape beside endpoint stations; do not add dependencies or a generic modeling language. Reuse the existing FragmentDelta contract where practical and document any public API addition in the owning setout ADR.

## Acceptance Criteria

Default outer-wall, west-nave, and east-nave round-head opening geometry is unchanged; all opening centers come from exact rational-iota bay fragments; the crossing remains physically open; count/extent edits produce deterministic identities, deltas, fingerprints, and fresh/warm agreement; tests explain the exact subdivision, omission/crossing behavior, architecture profile topology, and parametric change; the Basilica artifact and a changed-count artifact are rendered/reviewed; the full repository Definition of Done passes; ticket closure ships with the implementation.


## Notes

**2026-09-01T01:21:38Z**

Implemented a second bounded `setout_generate` shape: `LinearBayDistribution` produces stable one-based bay identities with exact rational start, end, and center coordinates, normalized omissions, canonical fingerprints, shared key-based `FragmentDelta` comparison, and warm/fresh parity. Basilica now owns separate exact outer, west-nave, and east-nave fragments; a local architecture adapter is the only rational-to-meter boundary, and `arcaded_wall_profile` accepts explicit centers instead of recalculating floating pitch and margins. The seven exterior bays intentionally remain independent of the accepted five-west/one-east nave openings: an eight-bay render experiment proved that coupling the counts compresses the interior arches into overlapping holes, so that false relation was removed. `topology_changed` is derived from generated additions/removals and reconfiguration exposes all three bay fragments and deltas. Public migration is additive for `setout_generate`; Basilica adds fragment accessors and deltas and clarifies `PlanSection::arcade_bays` as the exterior count. The owning setout ADR was amended; no new ADR was needed. The reviewed eight-bay variant has 41 total openings, retains 12 interior openings and the open crossing, and changes both the semantic assembly and rendered geometry. OBJ byte determinism remains a same-target contract; cross-target tests use semantic assertions instead of serialized floating-point text. Validation passed after rebasing onto `origin/main`: `typos`; `taplo fmt --check`; `cargo fmt --all -- --check`; `git diff --check`; `cargo check -p setout_generate --no-default-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `cargo doc --workspace --all-features --no-deps`; and `cargo test --workspace --locked --all-features --target wasm32-wasip1 --no-fail-fast --exclude cambium_web_bridge`. The only notice is the pre-existing future incompatibility in `proc-macro-error2 2.0.1`.
