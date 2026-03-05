---
id: cam-gvmz
status: closed
deps: [cam-mrwk, exe-cz8g]
links: []
created: 2026-03-05T02:03:21Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, architecture]
---
# Minimal EditPlan and compile/apply operator lifecycle

Introduce a deterministic EditPlan artifact and compile/apply lifecycle so preview and commit semantics are explicit and replayable.

## Design

Add EditPlan data model (coarse-grained steps initially) and runner lifecycle: compile -> plan, preview-on-clone(plan), apply-in-place(plan). Keep existing run_* API as adapters during transition. Plan must be deterministic and stably serialized/normalized (deterministic internal ordering), but not semantically canonicalized across different authorings.
Determinism scope for v0.1: (a) deterministic serialization/replay for identical inputs and mesh state, and (b) deterministic step ordering within a generated plan. Non-goal for v0.1: canonical identity across semantically-equivalent but differently-authored edit sequences.
Step granularity for v0.1: plan steps are kernel/edit-kernel operations, not micro-mutations, and reversibility is out of scope.
Plan compilation must not depend on unordered container iteration or allocator/free-list iteration order.
Ordering rules (v0.1 baseline):
- sort selection/set inputs by numeric ID before decision-making
- avoid iterating HashMap/HashSet directly for plan decisions
- define deterministic tie-breakers for ambiguous adjacency/continuation choices using lexicographic `(FaceId, HalfEdgeId)` (or documented equivalent)
- if loop start choice matters, pick the smallest stable ID as anchor

## Acceptance Criteria

- EditPlan type introduced with deterministic ordering guarantees
- runner supports compile -> plan plus preview/apply from plan
- reference migrations land for at least: `DeleteFaces` and one traversal-sensitive operator (`ExtrudeFaces` or `InsetFaces`)
- plan fingerprint defined and used for determinism checks; production fingerprint includes stable encodings of operator id, params, resolved selections, and plan steps (not full-mesh hashing)
- determinism tests define and use a stable mesh signature/test fixture identity to validate "identical mesh state" conditions
- unit/integration tests in Cambium verify deterministic compile output and stable plan fingerprints for identical inputs
- docs explain lifecycle semantics and ordering rules
