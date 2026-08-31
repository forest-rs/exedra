---
id: set-mpbt
status: closed
deps: []
links: [bsl-6ihj, set-qckq, set-mnga]
created: 2026-08-31T13:44:22Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Drive basilica massing from exact setout

Replace independently authored floating basilica dimensions and recomputed layout datums with one exact whole-building setout consumed by every architectural subsystem. This first slice covers massing and deliberately leaves repeated topology to `setout_generate` and masonry knowledge to `joiner_masonry`.

## Design

`BasilicaPremises` stores exact `Length` and `Count` roots. `BasilicaSetout` owns the immutable network, evaluation, provenance, invalidation, and resolved plan, level, aisle, roof, crossing, and east-end sections. Architecture modules consume resolved sections and lower only at recipe or placement boundaries. The module does not own topology generation, constructive profiles, tessellation, or joining rules.

## Acceptance Criteria

The public floating `BasilicaParams` surface is migrated with a documented break; `Layout` and duplicated massing constants are replaced by exact derived quantities; every architecture module consumes resolved sections; edit/invalidation tests cover representative plan, level, and topology changes; the default artifact differs from `origin/main` only in the reviewed correction to the south aisle-roof thickness frame; the structure-lab signature remains unchanged; the full Definition of Done passes.

## Notes

**2026-08-31T15:19:13Z**

Implemented one exact `BasilicaPremises` -> `BasilicaSetout` massing spine with resolved plan, level, aisle, roof, crossing, and east-end sections; all architecture modules now lower those sections only at geometry boundaries. Preserved repeated topology in architecture for the later `setout_generate` slice, and added `topology_changed` because named dirty bindings cannot describe newly added or removed instances. Fresh review completed truss and west-facade invalidation and bounded the arcade inventory domain. Corrected the old south aisle-roof frame so its proper-rigid thickness axis points outward/up; the baseline comparison differs by 35 vertex/normal lines only in `g aisle-roof-south`, and matched Blender renders reviewed the default and exact +2 m length variant. The structure lab remains clean with signature `ca8568e71945d5e3`; the ruin has zero diagnostics, assembly fingerprint `f71b72e3d752bfd5e4486bb5905bcbf0`, default OBJ signature `c7f24458dbcf4529`, and +2 m signature `6e63554fbefb0097`. Validation passed: `typos`; `cargo fmt --all -- --check`; `taplo fmt --check`; workspace all-target/all-feature Clippy with warnings denied; all-feature workspace tests; workspace rustdoc without dependencies; focused artifact, mirror-frame, invalidation, and structure-lab checks. No new ADR was added: the owning setout ADR 0001 was amended with the public migration, topology signal, and intentional visual correction.
