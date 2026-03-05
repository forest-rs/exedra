---
id: cam-leng
status: closed
deps: [cam-41um, cam-eoux, cam-v3t9, cam-jic3, cam-ojez]
links: []
created: 2026-03-05T12:54:46Z
type: epic
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, demo, web, architecture]
---
# Web demo vertical slice (wasm + three.js + scenarios)

Bridge the gap from library APIs to visible, interactive demos: wasm execution + browser viewer + curated scenarios + docs.

## Design

Fence: Cambium web surface owns demo orchestration and presentation; Exedra/Cambium core crates continue to own mesh/operator semantics. Build a local-first web demo that executes deterministic scenario flows and visualizes step-by-step outputs, then promote to publishable static hosting.

## Acceptance Criteria

- local web demo runs with scenario picker and step scrubber\n- wasm bridge executes named scenarios and returns renderable buffers + metadata\n- at least 6 curated scenarios are included\n- docs cover local run and static publish process\n- deterministic fingerprints/stats visible in UI

