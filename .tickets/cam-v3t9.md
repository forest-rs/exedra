---
id: cam-v3t9
status: closed
deps: [cam-41um]
links: []
created: 2026-03-05T12:55:01Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, demo, web, scenarios]
---
# Curated scenario pack for web demo

Add initial scenario library: wall openings, boxy hat, region flow, uv gallery, topology delete repair, primitive gallery.

## Design

Each scenario is deterministic, named, and returns ordered step snapshots with human-readable step labels. Reuse existing operators/fluent flows; avoid scenario-specific kernel changes.

## Acceptance Criteria

- six named scenarios implemented\n- each scenario returns at least 3 labeled steps\n- scenario outputs are deterministic across repeated runs\n- wall_openings uses cut_rect + delete + solidify/extrude

