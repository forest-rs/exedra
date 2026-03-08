---
id: cwv-hngf
status: closed
deps: []
links: []
created: 2026-03-08T13:06:04Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Strengthen normal line debug overlay

Normal line overlays are too subtle to trust on hard-edged meshes like cubes.

## Design

Increase line visibility by offsetting the line start along the normal, increasing line length, and using a brighter debug color. Keep it as a viewer-only diagnostic mode.

## Acceptance Criteria

Normal lines are clearly visible on cubes and poked grids; viewer build/tests remain green.

