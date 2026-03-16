---
id: cam-wikx
status: closed
deps: [cam-6z7d, cam-7qig]
links: []
created: 2026-03-16T00:00:24Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [architecture, analytic, convert]
---
# Cambium-facing analytic conversion surface

Expose explicit Cambium SDK helpers for analytic->mesh conversion without forcing analytic state through the mesh-only EditOperator trait.

## Design

Add exedra_analytic as a production dependency of cambium, introduce typed conversion params/output/error in a convert module, and provide explicit helpers for converting an AnalyticShell plus a convenience path for the rect-frame spike. Reuse the existing multi-domain ADR rather than creating a new owning ADR.

## Acceptance Criteria

1. cambium depends on exedra_analytic. 2. Public conversion APIs exist for AnalyticShell->Mesh and the rect-frame spike. 3. Conversion results carry deterministic provenance. 4. Tests and docs cover the explicit conversion seam.


## Notes

**2026-03-16T00:03:40Z**

Added cambium::convert as the explicit SDK seam for analytic->mesh conversion and took the approved production dependency on exedra_analytic. The module re-exports the narrow analytic MVP types needed for authoring, exposes analytic_shell_to_mesh / rect_frame_to_mesh helpers with deterministic provenance, and keeps conversion out of the mesh-only EditOperator runtime. No new ADR was added because crates/cambium/docs/adr-0002-multi-domain-geometry-architecture.md already owns the explicit-conversion policy. Validation: typos crates/cambium/src/convert.rs crates/cambium/src/lib.rs crates/cambium/Cargo.toml .tickets/cam-wikx.md; taplo fmt; cargo fmt --all; cargo test -p cambium --all-features; cargo clippy -p cambium --all-targets --all-features -- -D warnings; cargo doc -p cambium --no-deps.
