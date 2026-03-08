---
id: cwb-4hmh
status: closed
deps: []
links: []
created: 2026-03-08T04:06:40Z
type: task
priority: P2
assignee: Bruce Mitchener
---
# Add poke-grid web demo scenario

Add a deterministic web viewer scenario that applies edit.face.poke to a few grid faces to preview terrain-style topology buildup.

## Design

Use the new PokeFaces operator on a planar grid mesh in a few deterministic steps. Keep the scenario planar for now and rely on topology overlay. Update bridge/viewer README scenario descriptions.

## Acceptance Criteria

1) Scenario is listed and runnable. 2) It shows at least three poke steps on a grid. 3) Bridge tests still pass.

