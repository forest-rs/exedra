# ADR-0020: Explicit closed planar mitered sweeps

## Decision

Extend `Path3::MiteredPolyline` with `closure: PathClosure` and
`section_origin: [f64; 2]`. `Open` retains ordinary spatial rails;
`ClosedPlanar { normal }` declares a plane through the first point and a
last-to-first segment. Authors supply unique stations, without a duplicated
closing endpoint. No proximity test infers closure or repairs the path.

The authored section datum maps to the path: sample coordinates are relative
to `section_origin`. Section X is the projection of `section_x` perpendicular
to the first outgoing tangent; section Y is tangent cross X. Closed planar
frames derive their orientation from the plane and initial frame at each run,
avoiding accumulated closure rotation. Reversing traversal does not implicitly
mirror the section: callers must author the corresponding frame/profile when
they intend identical geometry. Changing the first station likewise requires
the section-X direction at that station.

Every corner, including station zero, uses the same bounded miter cut. The
last band reuses the first ring's vertex identities. Closed paths require
`CapMode::None`. Local forward-span and f32 wall checks include the closing
band. They do not certify unsampled profile interiors or distant intersections.
Curved closed paths and arbitrary spatial frame closure remain unsupported.

Sweep construction belongs to constructive; no crate ownership or dependency
changes are needed. Boolean surface ancestry retains originating sweep evidence,
separately from checks on the current result.

`SourceMap::sweep_sampling(face)` retrieves the original profile sampling policy
and, for curved open sweeps, authored path spans and bounds. Boolean splits carry
this alongside the original surface role. Shared `Arc` storage avoids copying
the span table per face or per Boolean descendant. The current body's
`sweep_checks` and `path_sampling` remain absent after CSG; ancestral bounds do
not describe the changed body's world-space accuracy or solid validity.

Controlled sweeps honor `sweep_path.max_path_edges` and `max_sweep_vertices`;
the latter bounds the ring product before vertex allocation. The default is
one million vertices. Profile and curved-path sampling retain their own budgets.
The closing band participates in both limits and all local checks.

## Migration

Existing `Path3::MiteredPolyline` literals add
`closure: PathClosure::Open` and `section_origin: [0.0; 2]`.
The immediate `tessellate_mitered_sweep` receives the same authored closure and
datum after `section_x`. All workspace callers are migrated together.

JSON's older `mitered_sweep` operation still reads as an open path at datum zero.
Writers use `mitered_path_sweep`, with required closure and datum fields, so older
readers reject changed geometry rather than silently ignoring new fields.
Text uses the same new operation name. Evaluation schema 37 invalidates cached
outputs and schema-stamped text, including changed sampling ancestry.

`TessellatedBody::path_sampling` now contains `Arc<PathSampling>`: readers can
keep dereferencing it; code constructing it wraps the value in `Arc::new`.
`SourceMap` retains `PartialEq` but no longer derives `Eq`, because sampling
evidence contains floating-point policies. `EvalPolicy` literals should keep
using `..Default::default()` and can set `max_sweep_vertices` explicitly.
