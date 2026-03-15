---
id: cam-aou6
status: closed
deps: [cam-m068]
links: []
created: 2026-03-15T18:06:14Z
type: task
priority: 2
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [operators, correctness]
---
# Bind compiled plans to source mesh state

Make Cambium compile/apply semantics honest by binding EditPlan to the mesh state it was compiled from and separating topology-only signatures from full mesh-state signatures.

## Design

EditPlan should carry source revision and, where useful, deterministic signatures that actually match the documented scope. Apply/preview should reject stale or mismatched plans. Keep compile/apply only where precomputation is meaningful.

## Acceptance Criteria

1. EditPlan stores source mesh revision or equivalent state binding. 2. Apply/preview reject mismatched source state. 3. Mesh signatures stop overclaiming attribute coverage. 4. Tests cover stale-plan rejection and attribute-only signature changes.


## Notes

**2026-03-15T23:54:26Z**

Bound EditPlan to source mesh state with PlanSourceState { revision, mesh_state_signature }, split topology-only vs full-state signatures, and made apply/preview reject stale or attribute-drifted plans. Added tests proving face regions, UVs, normal overrides, seam tags, and sharpness all affect the full-state signature.
