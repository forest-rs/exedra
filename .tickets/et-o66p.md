---
id: et-o66p
status: open
deps: [et-c9eu, exe-ccxf]
links: [et-c9eu, exe-ccxf, et-jmpb]
created: 2026-08-21T13:39:16Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# Add measured Steiner refinement for polygon caps

Permit deterministic generated interior vertices when boundary-only constrained-Delaunay triangulation cannot meet the measured cap-quality target.

## Design

exedra_triangulate owns deterministic planar point generation and refined triangle output; mesh integration stays in the consuming adapter. Preserve input-index provenance separately from generated vertices, enforce explicit vertex/work budgets, and measure the real sparse-hole, drill-like, and dense-boundary cases before setting any quality claim. Consume the kernel face-replacement operation only for existing-face retriangulation; constructive profile caps may build the refined patch directly.

## Acceptance Criteria

An opt-in refinement path emits deterministic generated points and CCW triangles under explicit budgets; cover, boundary preservation, termination, scale stability, and provenance are pinned; the wind tunnel demonstrates material improvement on quality-limited fixtures with reported time and work; at least one constructive cap consumer uses the result without ad hoc face surgery; no_std, safe Rust, no new production dependency.


## Notes

**2026-08-21T13:58:08Z**

ExecPlan — Goal: replace visibly poor ear-clipped cap triangles with measured deterministic quality improvements. Non-goals: generalized remeshing, unconstrained point insertion, a broad lineage framework, new dependencies, unsafe, or changing EarClip defaults. Steps: exact incircle; boundary-preserving constrained-Delaunay legalization; atomic kernel face-to-triangle-patch replacement; budgeted Steiner refinement plus one constructive consumer. Risks: exact degree-four exponent handling, cocircular termination, hole-bridge constraints, attribute propagation, and generated-point provenance. Each step lands as a separately reviewable commit; development tickets may be dropped before landing.
