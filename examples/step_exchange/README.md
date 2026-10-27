# Native STEP exchange

This example exports native analytic bodies directly, before or independently of
any display mesh. It exercises shared topology and orientation, through holes,
a blind pocket, translated and rotated occurrences, a 270° circular boundary,
metre units, and a full periodic cylinder whose wall reuses one seam edge twice.
Additional native revolutions exercise positive/negative conical slopes, inward
wall sense, full and partial turns, a toroidal cavity joined by partial-turn caps,
small radii, near-horn regular tori, near-cylinder cones, near-rigid placement and
near-seam trims. A single-face torus shares both periodic seam curves twice.
Two oblique-bore fixtures retain ellipse rims and include material witnesses,
including a rotated occurrence in metre units. The periodic cylinder also exercises quote, backslash, BMP and non-BMP text encoding.

```sh
cargo run --release -p step_exchange --bin step_exchange -- /tmp/step-fixtures
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
volume and area within numeric tolerance, retained elementary support families,
positive canonical cone angles, placed bounds and metre-to-millimetre scaling.
Bounding-box comparisons report OCCT's separate extrema-padding allowance;
small-solid measure checks use scale-sensitive absolute floors. No additional shape-repair operation is applied. This is independent
reader evidence for these fixtures, not a general CAD certification claim.

Success prints JSON with reader package version and per-fixture evidence.
Missing bindings print `status: unavailable` and exit 77; failed checks print
`status: failed` and fail. Neither outcome is a passed validation.

## Export timing

```sh
cargo run --release -p step_exchange --bin step_exchange -- /tmp/step-fixtures --bench
```

This writes `benchmark.json` for plates with 0, 4, 64 and 256 circular holes.
After a warm export, 21 iterations time validation, output allocation and STEP
serialization. Native authoring, file writes and document destruction are
outside the timed interval. Counts/bytes accompany minimum and median times so
results can be interpreted across machines. The exporter uses one bounded
String plus topology tables and reusable reference scratch; it does not build a
temporary String for every entity. This timing does not measure peak residency
or allocator call counts.

## Exact bore workload

```sh
cargo run --release -p step_exchange --bin bore_study > /tmp/bore-study.json
python3 examples/step_exchange/scripts/profile_bores.py --output /tmp/bore-allocation-profile
```

The first command times 101 release iterations at 0°, 30° and 60°: construction,
display realization and native STEP export separately. Preauthored source/axis,
output destruction, compiler caches, JavaScript, GPU and file I/O are excluded.
Each result retains its original Extrusion construction history. Capability
counts separate required scale/angle cases from exploratory numeric limits.
A supported native model can still refuse f32 emission or report an unmet display
target. The translated small model explicitly records curve-budget exhaustion
with small emission error; it is not classified as an unrepresentable placement.

The second command creates an isolated QA package using dhat, outside the
workspace and production dependency graph. Its three-result allocation window
starts after source/request/container preparation and warmup. Checkpoints retain
all outputs, release display meshes/STEP text while retaining native models, then
release models. Original Extrusion copies inside each result are included;
the shared preauthored input is excluded.
