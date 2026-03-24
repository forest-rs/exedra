---
id: ef-2oz3
status: open
deps: [exe-vzlq]
links: [exe-vzlq]
created: 2026-03-24T02:47:11Z
type: task
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
---
# Benchmark exedra_fidget against Fidget mesher

Benchmark the exedra_fidget adapter against Fidget's built-in meshing on equivalent inputs once Exedra's dual contouring is mature enough for a fair comparison.

## Design

Own only the benchmark harness, shape corpus, and result documentation. Do not use this ticket to improve meshing quality; that remains separate work. The benchmark should compare equivalent Fidget-authored fields routed through exedra_fidget + exedra_isosurface versus Fidget's own mesher, and should document where geometry quality or topology differences still make the comparison inexact.

## Acceptance Criteria

- benchmark crate or benchmark target compares equivalent Fidget-authored shapes through both pipelines
- documents benchmark inputs, configuration, and comparability caveats
- records at least one baseline result for VM and, when available, JIT backends
- linked from exe-vzlq as the deferred follow-up for performance work
