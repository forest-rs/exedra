---
id: cam-2fau
status: closed
deps: [cam-an35]
links: []
created: 2026-03-07T18:57:12Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Face-region extraction and boundary edge classification

Add internal Cambium helpers that derive a deterministic selected-face region model from canonical face sets, including interior edges, boundary edges, per-face membership, and adjacency metadata needed by patch-style operators.

## Design

Introduce an internal patch/region module for face-edit operators. Given a canonical face set and a mesh, produce a stable region structure with selected faces, boundary edges, interior shared edges, incident vertices, and deterministic ordering suitable for downstream loop extraction and migration of face-edit operators. Keep this internal to Cambium.

## Acceptance Criteria

- Internal region helper derives stable region data from canonical face selections.\n- Boundary edges are identified as edges with exactly one selected incident face.\n- Shared/interior selected edges are identified separately.\n- Tests cover single-face, adjacent multi-face, disjoint-face, and closed/open surface cases.\n- No public API changes.

