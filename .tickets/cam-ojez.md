---
id: cam-ojez
status: closed
deps: [cam-eoux, cam-jic3]
links: []
created: 2026-03-05T12:55:12Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, demo, web, release]
---
# Publishable static web demo packaging

Prepare static-site output and deployment docs so the demo can be shared publicly after local iteration.

## Design

Keep local-first dev flow. Add static build target, asset bundling, and a reproducible command to produce deployable artifacts.

## Acceptance Criteria

- one command builds static deployable demo artifacts\n- docs include publish checklist and hosting notes\n- build output includes scenario assets and wasm bundle\n- smoke check verifies generated site loads at least one scenario

