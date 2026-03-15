---
id: cam-m068
status: closed
deps: []
links: []
created: 2026-03-15T18:06:03Z
type: task
priority: 1
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [architecture, docs]
---
# ADR and blueprint for multi-domain geometry architecture

Write the owning ADR and crate-by-crate implementation blueprint for a multi-domain geometry stack. Clarify Exedra as the polygon head, Cambium as orchestrator, and explicit conversions between canonical domains.

## Design

Use Cambium as the owning crate for the architecture record. The blueprint should name crates, module boundaries, invariants, conversion seams, and the execution sequence that improves the existing mesh head while adding one analytic MVP slice.

## Acceptance Criteria

1. ADR merged in Cambium docs. 2. Plan/blueprint document names crates, modules, operator taxonomy, and phases. 3. Existing open Exedra tickets are referenced where applicable. 4. Risks and stop conditions are documented.


## Notes

**2026-03-15T18:09:54Z**

Added the owning architecture artifacts for the Hydra/multi-domain direction: crates/cambium/docs/adr-0002-multi-domain-geometry-architecture.md and crates/cambium/docs/plans/cam-m068-multi-domain-geometry-architecture.md. The blueprint keeps exedra as the polygon head, scopes a narrow exedra_analytic MVP, sequences mesh-head hardening before broader domain expansion, and references linked tickets exe-h2rh and exe-xgtv. Also landed the first runtime seam in code by adding OperatorDomain metadata with a non-breaking EditOperator::domain() default of Mesh, plus tests and crate docs. Validation run: typos crates/cambium/docs crates/cambium/src; cargo fmt --all; cargo test -p cambium --all-features; cargo clippy -p cambium --all-targets --all-features -- -D warnings; cargo doc -p cambium --no-deps.
