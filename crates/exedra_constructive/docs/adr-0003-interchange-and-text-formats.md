# ADR-0003: Interchange and Text Formats

- Status: Accepted
- Date: 2026-08-19
- Owners: Exedra maintainers
- Tickets: `ec-yzc4`, `ec-71sj`

## Context

External spec compilers (living in separate repositories) need a wire
format to hand recipes to this workspace without linking Rust, and the
workspace needs a human-diffable rendering for goldens and review. These
are different jobs with different stability requirements.

## Decision

Two formats, one identity oracle:

1. **`exedra-recipe-v1`** (module `interchange`, behind the off-by-default
   `serde` feature): versioned JSON defined by dedicated DTO types — never
   serde derives on internal IR types, so internal refactors cannot
   silently change the wire. Header `{"format": "exedra-recipe",
   "version": 1}`.
2. **`constructive-ir-v1`** (module `text`, always available): the
   golden/debug text rendering — hex f64 bit patterns, ordered sections,
   canonical (re-dumping a parsed recipe is byte-identical).

Both parsers rebuild through `RecipeBuilder`, re-running all validation;
**round-trip fingerprint equality is the correctness oracle** for both.

### Stability policy (v1)

- Additive-only evolution: new optional fields and new node kinds may
  appear; existing fields never change meaning. Breaking changes bump the
  version, and old versions keep parsing.
- Unknown JSON *fields* are ignored (additive tolerance). Unknown node
  *kinds* are hard errors: recipes are executable content, and skipping an
  operation would silently change geometry.
- Floats are plain JSON numbers; serde's shortest-round-trip rendering
  re-parses to identical `f64` bits, so fingerprints survive the wire
  exactly. The text format uses raw bit patterns for the same guarantee.
- Curve segments serialize as explicit records (`line`/`arc`/`cubic`),
  insulating both formats from kurbo's types and versions.
- A frozen corpus (`goldens/recipe_v1.frozen.json` + its fingerprint) pins
  the schema: the corpus never regenerates silently, and its test failing
  means the schema drifted. Re-blessing is a reviewed, deliberate act.

### Feature posture

The `serde` feature implies `std` (the interchange is a host-side surface
and the workspace serde dependency is std-flavored). Core evaluation and
the text format remain `no_std` and serde-free; the canonical byte
encoding (`CanonBytes`) — not serde — remains the fingerprint contract.

## Consequences

- External repos can emit recipes from any language; the fingerprint in a
  parsed recipe proves the wire preserved intent.
- Two formats to maintain — accepted because their jobs (machine
  interchange vs golden diffs) pull in opposite directions, and both are
  thin over the same builder.
