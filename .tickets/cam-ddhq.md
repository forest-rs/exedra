---
id: cam-ddhq
status: closed
deps: []
links: []
created: 2026-09-04T04:42:54Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Bring the operator SDK into the Exedra crate family

Give the operator crate an honest Exedra-family name before its first publication. The current implementation is a mature mesh-edit operator SDK plus explicit helpers around sibling geometry domains; it is not yet a heterogeneous procedural graph. The change must preserve that distinction, keep domain-native representations independent, and leave one clear path for future graph evaluation.

## Design

Name the earned operator surface exedra_ops, together with its test and web support packages. Keep mesh mutation, plans, reports, selections, and policy in that crate. Keep constructive, analytic, implicit-field, and assembly representations in their owning crates and make crossings explicit. Record a future procedural-network boundary only as an earned follow-on: Exedra owns typed geometry semantics and explicit conversions, `execution_graph` owns incremental execution, and `understory_node_graph` owns editor state. Do not add a placeholder graph crate or universal mutable geometry value.

## Acceptance Criteria

The workspace contains no current references to the internal working name. exedra_ops has a clear owns/does-not-own fence and independently selectable native-head adapters. Root architecture and roadmap describe domain-native heads plus explicit conversion and the future network boundary without claiming it exists. All workspace formatting, lint, test, documentation, spelling, and manifest checks pass.

## Notes

**2026-09-04T05:33:22Z**

Renamed the unpublished operator package and its test and web support packages into the Exedra family, including workspace paths, Rust imports, documentation, UI labels, protocol identifiers, and CI. Key decisions: `exedra_ops` remains a mesh-specific operation SDK with independently selectable `analytic`, `constructive`, and `assembly` adapters. Its default enables the commonly paired `std`, `constructive`, and `assembly` surfaces without claiming ownership of their native APIs; mesh-only remains an explicitly checked minimum configuration. `Assembly` retains constructive recipes through placement and compilation, admits materialized meshes alongside them, and requires other native domains to cross explicitly rather than growing a universal part-source enum. Removed `OperatorDomain` and `EditOperator::domain()` because every implementation operated on `Mesh`; future cross-domain composition uses typed ports instead of aspirational metadata. ADR-0005 keeps native geometry algorithms and values in their owning heads, assigns incremental scheduling to `execution_graph`, editor document/projection to `understory_node_graph`, and geometry vocabulary, artifacts, conversions, identity mappings, and node bindings to a future Exedra integration layer. That future layer requires distinct authored/editor/runtime identities, immutable or copy-on-write artifacts, multi-input typed nodes, explicit conversion evidence and refusals, deterministic cache inputs, and measured selective recomputation. No placeholder graph, universal geometry value, new production dependency, unsafe code, or geometry behavior change was added. Fresh Lynx review found no Must items; its one stale viewer-backend documentation finding was corrected. Validation passed: `typos`; `cargo fmt --all -- --check`; `taplo fmt --check --diff`; `bash .github/copyright.sh`; `cargo metadata --locked --no-deps --format-version 1`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps`; all 16 meaningful std/libm and adapter feature combinations, including no_std target checks; mesh-only dependency-tree inspection; `npm ci`; `npm run build`; and `npm run smoke:dist`.
