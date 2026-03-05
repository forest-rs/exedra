---
id: cam-eoux
status: closed
deps: [cam-41um]
links: []
created: 2026-03-05T12:54:55Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, demo, web, ui]
---
# Three.js demo viewer shell

Build a local-first browser viewer that renders scenario steps and metadata from the wasm bridge.

## Design

Implement a thin Three.js app with scenario picker, step scrubber, wireframe toggle, face-region coloring mode, diagnostics panel, and fingerprint/stats display.

## Acceptance Criteria

- app loads and renders mesh snapshots from wasm bridge\n- scenario picker + step scrubber work\n- toggles for shaded/wireframe and region-color mode\n- diagnostics + fingerprints shown per step

