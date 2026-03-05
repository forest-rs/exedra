---
id: cam-xnoi
status: open
deps: [cam-mn4h, cam-h7yk]
links: []
created: 2026-03-05T08:05:03Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, api, modeling]
---
# Face-edit semantics matrix (extrude/inset/solidify)

Define a forward-looking face-edit semantic model so current extrude/inset behavior aligns with future solid/surface workflows.

## Design

Write a semantics matrix for face-edit families: extrude, inset, and solidify/thickness-like behavior. Define outcomes for open surfaces vs closed volumes, source-face retention/removal, generated topology ownership, and region/attribute propagation expectations. Specify parameter model and mode naming that can scale (without introducing ambiguous behavior). This is the design contract ticket that informs implementation tickets such as cam-7u7l.

## Acceptance Criteria

- Semantics matrix documented for open/closed contexts; - mode names and parameter contracts are explicit; - interaction with adjacency support and winding policy is defined; - cam-7u7l references this contract and implements against it.

