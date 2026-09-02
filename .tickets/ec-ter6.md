---
id: ec-ter6
status: closed
deps: []
links: []
created: 2026-09-02T10:56:20Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Support revolve profiles touching the axis

Allow nonnegative-radius revolve profiles to meet the revolution axis without emitting degenerate rings or faces, while preserving deterministic topology, source features, regions, and placement behavior.

## Design

Collapse each axis profile point to one shared vertex. Emit triangle fans for profile edges with one axis endpoint, quads for off-axis edges, and no wall for the final profile segment when it lies on the axis. Refuse negative radii and non-closing axis segments with typed errors. Include cardinal meridians in full sweeps so symmetric bounds retain exact extrema.

## Acceptance Criteria

A semicircular profile revolved through tau is a closed deep-valid manifold with exact symmetric bounds and wall provenance; a half sweep with both caps is deep-valid; negative-radius profiles and non-closing axis segments are typed refusals; existing off-axis revolutions remain deterministic; workspace gates pass.

## Notes

Implemented pole-aware revolve topology directly in the tessellator. Exact axis points now share one vertex across every angular ring; adjacent bands emit triangle fans; off-axis bands retain their existing quads; and a final authored axis segment emits no wall. Full sweeps include all four cardinal meridians, preserving exact symmetric extrema. The dense vertex source map is built with vertex emission so collapsed poles inherit the adjacent wall rather than the skipped closure.

The former `TessellateError::AxisContact` refusal was replaced by `NegativeRadius`, `NonClosingAxisSegment`, and `RevolveSegmentLimit`. This is the deliberate public error migration recorded in crate ADR 0001. Evaluation semantics changed, so `EVAL_SCHEMA_VERSION` advanced from 6 to 7 and dependent fingerprints and goldens were refreshed.

Tests cover a full semicircle revolution against an analytic sphere-volume oracle, exact bounds, exactly two poles, outward winding, manifold closure, source regions and provenance, capped half sweeps, concave triangulated caps, negative radius, misplaced axis segments, insufficient angular budgets, exact evaluator fidelity, and deterministic off-axis revolutions.

Validation passed: `typos`; `cargo fmt --all -- --check`; `taplo fmt --check`; `cargo check -p exedra_constructive --no-default-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `cargo doc --workspace --all-features --no-deps`.
