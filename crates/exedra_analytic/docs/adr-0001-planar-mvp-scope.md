# ADR-0001: Planar Analytic MVP Scope

- Status: Accepted
- Date: 2026-03-16
- Owners: Exedra analytic spike maintainers
- Ticket: `cam-6z7d`

## Context

The workspace is adopting a multi-domain geometry architecture. The first new
canonical domain must be small enough to earn its existence without dragging in
full CAD scope.

## Decision

`exedra_analytic` starts as a planar analytic topology spike:

- planar faces only,
- line-segment coedges only,
- shell/loop/coedge topology,
- explicit planar opening loops,
- deterministic tessellation into `exedra::Mesh`.

The first slice explicitly does not support:

- curved edges,
- general trims beyond explicit opening loops,
- booleans,
- reverse conversion from polygon mesh into analytic state.

## Consequences

Positive:
- proves the second canonical domain with a bounded implementation,
- preserves `exedra` as the polygon head,
- gives Cambium a real future conversion seam.

Tradeoffs:
- the spike is intentionally narrow,
- some useful analytic workflows remain impossible until later slices,
- opening loops tessellate to multiple mesh faces, so analytic-face provenance is
  one-to-many rather than one-to-one.
