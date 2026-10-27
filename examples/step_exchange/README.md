# Native STEP exchange

This example exports native analytic bodies directly, before or independently of
any display mesh. It exercises shared topology and orientation, through holes,
a blind pocket, translated and rotated occurrences, a 270° circular boundary,
metre units, and a full periodic cylinder whose wall reuses one seam edge twice.
The last fixture also exercises quote, backslash, BMP and non-BMP text encoding.

```sh
cargo run --release -p step_exchange -- /tmp/step-fixtures
```

Each STEP file has an entry in `expected.json`: exact native volume and area,
expected bounds, physical unit scale, used topology counts and the exporter's
agreement/uncertainty report. Files declare `AUTOMOTIVE_DESIGN` (AP214). The
exporter itself checks native incidence/support agreement and budgets; it does
not establish arbitrary trim nesting or global regular-solid validity.

## Independent CAD reader

Install the reader into an isolated environment; it is not an Exedra dependency:

```sh
python3 -m venv /tmp/step-oracle
/tmp/step-oracle/bin/pip install cadquery-ocp==8.0.1.1.0
/tmp/step-oracle/bin/python examples/step_exchange/scripts/validate.py /tmp/step-fixtures
```

The script refuses model syntax/semantic checks and transfer warnings/failures,
then checks the imported boundary with OCCT's `BRepCheck_Analyzer`. It requires
one solid and shell, preserved unique face/edge/vertex counts, exact reference
volume and area within numeric tolerance, placed bounds and metre-to-millimetre
scaling. No additional shape-repair operation is applied. This is independent
reader evidence for these fixtures, not a general CAD certification claim.

Success prints JSON with reader package version and per-fixture evidence.
Missing bindings print `status: unavailable` and exit 77; failed checks print
`status: failed` and fail. Neither outcome is a passed validation.

## Export timing

```sh
cargo run --release -p step_exchange -- /tmp/step-fixtures --bench
```

This writes `benchmark.json` for plates with 0, 4, 64 and 256 circular holes.
After a warm export, 21 iterations time validation, output allocation and STEP
serialization. Native authoring, file writes and document destruction are
outside the timed interval. Counts/bytes accompany minimum and median times so
results can be interpreted across machines. The exporter uses one bounded
String plus topology tables and reusable reference scratch; it does not build a
temporary String for every entity. This timing does not measure peak residency
or allocator call counts.
