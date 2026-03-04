---
id: cam-26fl
title: Dart vertex handling in subdivision vertex classification
status: open
deps: [exe-0wv0, exe-gtg2]
links: []
created: 2026-03-04T05:54:06Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.5]
---
# Dart vertex handling in subdivision vertex classification

Implement dart vertex classification and subdivision rule for Catmull-Clark. A dart vertex has exactly one incident sharp edge — the crease terminates at the vertex. The limit surface is smooth at a dart despite the crease touching it. Without dart handling, crease termination vertices get incorrect limit positions (treated as smooth with no crease awareness, or as crease with wrong stencil). This affects both subdivision topology (cam-inpo) and derived normal computation (exe-o4iu).

## Design

Vertex classification enum: Smooth (0 sharp edges), Dart (1 sharp edge), Crease (2 sharp edges), Corner (3+ sharp edges or explicit vertex sharpness pin). Classification is derived at subdivision/normal computation time, not stored. Dart subdivision rule: use the smooth vertex point mask but with awareness of the single crease edge direction — the limit position and tangent computation differ from the pure smooth case. For derived normals: a dart vertex accumulates normals from the full one-ring (no hard break), but the crease edge may influence tangent frame. Implementation: classification helper function that takes vertex + mesh and returns the enum. Used by cam-inpo (Catmull-Clark) and exe-o4iu (derived normals).

## Acceptance Criteria

1) Vertex classification function exists and returns Smooth/Dart/Crease/Corner. 2) Classification respects vertex sharpness override when present. 3) Dart subdivision mask produces correct limit positions. 4) Derived normals handle dart vertices correctly (no spurious hard break). 5) Golden tests with dart configurations. 6) cargo clippy/test pass.
