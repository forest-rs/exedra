---
id: cam-vbea
status: closed
deps: []
links: []
created: 2026-03-08T18:06:59Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Opening corner normal splits are incomplete

Wall opening and related composed face-edit workflows still emit too few distinct render normals at some opening corners. Add a regression that inspects incident faces / sharp edges / extracted render-vertex variants at opening corners, then fix the sharpness or extraction logic so orthogonal feature faces do not collapse into one smooth group.

## Acceptance Criteria

1. A regression covers a wall-opening corner with three orthogonal feature faces and asserts three distinct render-normal variants at the shared topology vertex. 2. The fix preserves existing primitive and face-edit normal tests. 3. Web wall_openings normal debug view reflects the corrected split behavior.

