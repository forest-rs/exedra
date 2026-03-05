# Brief: Operator namespace and discoverability

## Decision
Cambium exposes a grouped operator surface at crate root and documents stable
family prefixes as the primary discoverability contract.

v0.1 families:
- `inspect.*`
- `select.*`
- `tag.*`
- `mark.*`
- `uv.*`
- `edit.*`

## Why
- Prevent flat API drift as operator count grows.
- Keep stable IDs and rustdoc navigation aligned.
- Make it obvious where new operators belong.

## Crate-root grouping rules
- Re-exports are grouped by concern:
  - runtime/execution
  - shared Exedra re-exports
  - policy/selection
  - fluent workflow layer
  - operator family surface
- Crate docs include a family-oriented “Where to find X” section.

## Operator docs rules
- Every operator rustdoc should reference its stable family (`name()` prefix).
- Examples should show compile/preview/apply lifecycle, not deprecated flow.
- New operators in v0.1 must use one of the frozen family prefixes.

## Non-goals / deferrals
- Full generated operator catalog tables (handled in `cam-nyws`).
- Renaming operators in this ticket (handled in `cam-suy5` and follow-ups if needed).

