# ADR-0024: Sweep section laws

## Status

Accepted

## Context

Every sweep carried a constant section. Tapered forms (tree branches, horns,
turned and tapered mouldings, cables with twisted strands) had to be built as
lofts over hand-placed sections. A loft does not know its path, so the
caller had to frame each section, and its chart measured section index rather
than distance. The sweep already owns a sampled path, per-station frames, and
realization checks, so the section's variation belongs on the sweep.

## Decision

1. **A `SectionLaw` on `NodeKind::Sweep`.** It holds a `scale` law and a
   `twist` law (radians, right-handed about the tangent, from section X toward
   section Y). A `Law` is a constant or piecewise-linear `[t, value]` keys,
   finite, with `t` strictly increasing from exactly 0 to exactly 1. Scale must
   stay positive. `SectionLaw::IDENTITY` is a constant section.
2. **The parameter is normalized chord length along the sweep's stations:**
   path points for polylines, sampled stations for analytic curves, including
   the closing station of a closed path. Laws are evaluated only there; they
   add no stations, and sections between stations interpolate along each band.
   Resampling for a law would make the station set depend on two policies and
   change existing sampling; authors add stations instead.
3. **Laws act on the frame axes, about the section datum.** Each station's
   section axes are rotated and scaled before the datum offset is applied, so
   the point placed on the path stays on it. Placement is linear in the axes,
   so this is exact for the plain, mitered-cut and analytic frames alike, and
   walls and caps agree. The identity law is skipped, so constant-section
   output is bit-identical.
4. **Validity is checked, not assumed.** Malformed laws are
   `RecipeError::InvalidParameter { what: "sweep section law" }` at recipe
   construction and `TessellateError::InvalidSectionLaw` at tessellation. A
   closed path requires both laws to agree at `t = 0` and `t = 1`
   (`SectionLawError::OpenSeam`); a closed twist must return to its starting
   angle. Agreement is exact, not within a tolerance, and a whole extra turn
   does not count as agreement: the first and last stations share one seam
   ring, which needs a single section, and a turn would also carry the
   chart's U seam around the path. Explicit agreement is preferred to a
   tolerance that could close a seam the author did not intend. An
   identity-shaped law (every value 1 for scale, 0 for twist) is stored as
   `SectionLaw::IDENTITY`, and law encodings write `-0.0` as `+0.0`, so
   equal laws round-trip and fingerprint identically. `SectionLaw` is
   `#[non_exhaustive]`; build it with `SectionLaw::new` or its presets so a
   per-axis scale can be added later. Controlled (mitered and analytic) sweeps run their existing span and
   realization checks on the shaped frames, so a law that collapses or folds a
   band is refused. The legacy polyline sweep keeps its unchecked contract.
5. **Charts measure the unscaled section.** `SurfaceChart::Sweep` keeps U on
   the reference (unscaled) profile perimeter and V on centerline distance.
   Texture stays continuous along a taper and its density follows section
   size. A chart whose U follows the local perimeter would shear texture across
   bands; it is not provided.
6. **Identity stays stable; shaped sweeps are distinct.** A constant-section
   sweep keeps its fingerprint encoding (kind tag 3). A shaped sweep uses kind
   tag 18 followed by the canonical law encoding. Text writes a `shaped scale
   <law> twist <law>` prefix before the sweep operation, and interchange wraps
   the sweep operation in a `shaped_sweep` op. Both are emitted only for
   non-identity laws, so older readers refuse shaped sweeps rather than drop
   the law, and existing documents are unchanged. No evaluation schema bump
   is needed: unchanged recipes evaluate identically.

## Consequences

- Branches, horns and tapered mouldings are one sweep node with an honest
  path, frames, caps, chart, and checks.
- `NodeKind::Sweep` gains a field; constructors add `section`, and
  exhaustive patterns name it or use `..`.
- `tessellate_path_sweep` is the path-generic entry point. The per-path-form
  functions and `tessellate_sweep_with_chart` keep constant sections.
- Per-axis scale, laws in absolute distance, and law-driven station
  refinement are possible later extensions of `SectionLaw`.
