---
id: ec-hmjp
status: closed
deps: []
links: []
created: 2026-09-02T10:33:07Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Evaluate constructive planar faces

Evaluate the existing `PlanarFace` recipe node as a deterministic single-sided open body while retaining profile curves, regions, source features, placement, and fidelity.

## Design

Reuse the constructive profile discretization and polygon triangulation seams. Emit one local +Z-facing surface with holes, transform it through `Placement3`, leave its perimeter as a mesh boundary, and preserve segment-level provenance on boundary geometry without synthesizing thickness or a back face.

## Acceptance Criteria

A square evaluates to two +Z-facing triangles; curved boundaries follow `EvalPolicy` discretization; holes remain open; provenance and `FACE_REGION` are retained; transforms, mirrors, and instances compose; stretch recognizes the result as an open shell; deterministic goldens and workspace gates pass.

## Notes

**2026-09-02T10:51:04Z**

Implemented `PlanarFace` evaluation through the existing profile discretization and polygon triangulation path. The result is one single-sided open body: local +Z winding, holes preserved as boundaries, face region zero, `Feature::PlanarFace` on triangles, and `Feature::Wall` segment provenance on boundary vertices. Reflecting placements repair triangle winding; transforms, mirrors, and instances compose normally; crossing stretch reaches the typed `eval.stretch.open_shell` refusal. The evaluator match is now exhaustive, so future node kinds require an explicit evaluation decision.

Advanced `EVAL_SCHEMA_VERSION` from 5 to 6 and refreshed dependent fingerprints and goldens. No separate ADR was added because crate ADR 0001 already owns the evaluation-domain contract and now records these semantics.

Validation passed: `typos`; `cargo fmt --all -- --check`; `taplo fmt --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `cargo doc --workspace --all-features --no-deps`.
