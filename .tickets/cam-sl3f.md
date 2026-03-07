---
id: cam-sl3f
status: closed
deps: [cam-an35, cam-quek]
links: []
created: 2026-03-07T18:57:36Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Migrate face-edit operators to the patch-region substrate

Rewrite existing Cambium face-edit operators to use the shared internal region/loop substrate instead of duplicating region extraction, boundary walking, duplication, and connection logic inline.

## Design

Migrate ExtrudeFaces, InsetFaces, SolidifyFaces, and CutRect to the shared patch-region helpers. Preserve public operator IDs, plans, outputs, and current semantics. Reduce duplication in face_edit.rs and make the operator bodies read as compositions of region utilities plus exedra::op kernel calls.

## Acceptance Criteria

- ExtrudeFaces, InsetFaces, SolidifyFaces, and CutRect use the shared internal patch-region helpers.\n- Existing behavior, outputs, and tests remain valid.\n- face_edit.rs shrinks materially and repeated region/boundary logic is removed.\n- Public API and operator IDs are unchanged.

