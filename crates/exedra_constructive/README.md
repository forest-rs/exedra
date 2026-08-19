# exedra_constructive

Constructive geometry head: an immutable, content-addressed recipe IR with
deterministic tessellation into Exedra meshes.

This crate is the fourth geometry head under Cambium's multi-domain
architecture (cambium ADR-0002): a *compiler target* for pre-mesh
construction. External spec frontends (geometry frontends, parametric
evaluators — living in their own repositories) build recipes out of kurbo-backed
2D profiles and constructive bodies (extrude, revolve, loft, sweep, CSG,
transforms, instances); evaluation tessellates them into `exedra::Mesh`
values carrying a full provenance source map, semantic region/material
slots, and an honest fidelity report.

Design commitments (see `docs/adr-0001-constructive-domain-scope.md`):

- **f64 construction, f32 emission.** All construction and evaluation happen
  in f64 (kurbo-native); the single narrowing to `[f32; 3]` happens at mesh
  emission and is documented.
- **Determinism as a contract.** Evaluation trig always routes through
  `libm` — even in std builds — and arc discretization is owned here rather
  than delegated to kurbo's trig, so identical recipes produce bit-identical
  meshes on every platform. Content hashes incorporate an evaluation schema
  version so kurbo upgrades invalidate caches explicitly, never silently.
- **Closed by construction.** Profiles are endpoint-chained cyclic segment
  lists with bulge-parameterized arcs: both endpoints of every segment are
  stored exactly, so loop closure is structural, not tolerance-based.
- **Opaque source identity.** Frontends attach their own source references,
  policy ids, and issue ids; this crate round-trips them through source maps
  and reports without ever parsing them. No spec vocabulary lives here.

## License

Apache-2.0 OR MIT
