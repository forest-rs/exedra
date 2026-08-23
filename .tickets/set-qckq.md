---
id: set-qckq
status: in_progress
deps: []
links: []
created: 2026-08-23T10:37:16Z
type: epic
priority: 1
assignee: Bruce Mitchener
---
# Establish setout through the first basilica vertical slice

Create the consumer-neutral setting-out layer and prove it through exact roof quantities, endpoints, provenance, reconstruction assessment, and one roof hypothesis consumed by both `basilica_ruin` and `basilica_structure_lab`. The work spans public contracts and multiple sessions; this ticket owns the first earned implementation milestone.

## Design

Keep setout, setout_reconstruction, setout_generate, and setout_joiner as narrow replaceable crates. setout_joiner is the only construction adapter; joiner remains independently usable. Durable selections use semantic producers plus expected structural claim keys, and discrete candidates use stable candidate keys. The first milestone reaches the basilica before generative expansion.

## Acceptance Criteria

The crate-local ADR records fences, rejected options, decision identity, and sequencing. The first implementation milestone later lands no_std setout with exact iota propagation and provenance, setout_joiner with exact-endpoint integration, and setout_reconstruction with analysis-only invalidation. One resolved roof hypothesis drives the accepted ruin and the structure lab; independent roof calculations in `basilica_ruin::geometry`, `basilica_ruin::architecture::nave_trusses`, and `basilica_structure_lab::model` are removed. Roof skin, wall plates, ridge, principal rafters, and gables share derived datums instead of copied coordinates or anonymous gaps. A roof-rise or pitch reconfiguration reports the exact affected quantities and named construction/assembly elements, produces a warm result identical to a fresh build, and leaves unrelated systems clean. Joiner contact, recipe, and load-path validation still pass, explain reaches the authored roof premises, and any intentional visual change is reviewed in the accepted artifact. Required fixtures and the full repository Definition of Done pass; the ticket close note records migration notes and validation.
