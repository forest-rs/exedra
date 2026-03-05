---
id: cam-g3hn
status: closed
deps: [cam-mrwk]
links: []
created: 2026-03-05T08:04:40Z
type: task
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, api, docs]
---
# Operator taxonomy + v0.1 set freeze

Define a curated operator taxonomy and freeze the minimum v0.1 operator set so naming and discoverability stay coherent as the catalog grows.

## Design

Create a catalog-oriented taxonomy (edit/mark/tag/select/inspect/uv/construct/remesh groups), map current operators into groups, and identify a v0.1 minimum set (target 10-15 core operators). Mark deferred groups explicitly for v0.5+. This ticket is planning/docs and should not rename APIs directly.

## Acceptance Criteria

- Taxonomy doc exists and lists current operators by family; - v0.1 minimum operator set is explicitly listed and frozen; - deferred operator families are listed with rationale; - downstream naming/docs tickets reference this ticket.


## Notes

**2026-03-05T08:16:19Z**

Taxonomy contract documented in crates/cambium/docs/briefs/09_operator_taxonomy_v01_freeze.md. Frozen v0.1 set established and mapped to downstream naming/discoverability/catalog tickets.
