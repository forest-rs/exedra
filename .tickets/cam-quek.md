---
id: cam-quek
status: closed
deps: [cam-an35, cam-7tc3]
links: []
created: 2026-03-07T18:57:29Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Shared patch helpers for region duplication and loop connection

Add internal Cambium helpers for duplicating selected-region vertices with stable remaps, building ring/frame topology, and propagating authored attributes for generated faces and edges.

## Design

Use the internal region/loop substrate to factor common face-edit machinery out of face_edit.rs: duplicate region vertices with deterministic source->copy maps, build wall/frame faces between paired loops, and centralize face/corner/edge attribute propagation for generated topology. Keep helpers internal.

## Acceptance Criteria

- Internal helpers cover duplicated-vertex remaps, frame/wall face construction, and shared attribute propagation for generated topology.\n- Existing duplicated face-edit support code is reduced materially.\n- Tests cover deterministic remap behavior and generated topology/attribute propagation on representative regions.\n- No public API changes.

