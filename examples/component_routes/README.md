# Component routes

One mounting plate built two ways. `build_plate` is handwritten: it draws a bolt
hole and an adjustment slot with `LoopCursor`, cuts them from a rounded outline
with `Profile2::with_cutouts`, and adds a symmetric extrusion and a boss revolved
about Z with the `ops` records. `plate_parts` emits the same tables at once, as a
spec compiler would, for `Recipe::try_from_parts`. Both freeze to the same
recipe fingerprint.

`build_plate` builds into a caller's `RecipeBuilder` and is all-or-abandon: when
it fails, the caller drops the builder instead of continuing with its partial
tables. The demo changes a dimension, selects the slot walls through segment
tags and the source map, and diagnoses and corrects a cutout that escapes the
outline.

```sh
cargo run -p component_routes
```
