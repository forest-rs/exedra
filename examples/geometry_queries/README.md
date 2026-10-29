# Geometry queries

Run the existing compiled-surface example with `cargo run -p geometry_queries --bin geometry_queries`.

The native solid example builds a 60 × 40 × 20 block with two joined R4 through
passages and a blind stepped bore: R5 to depth 6, then R2 to depth 12. It prepares
one retained-body query and prints point inputs, material/void/boundary results,
native boundary witnesses, preparation storage and actual query work.

```sh
cargo run -p geometry_queries --bin native_solid_query --release -- 10 /tmp/exedra-solid-sections
```

The optional output directory receives `valve-sections.svg` and
`valve-sections.csv`. Four sections at heights 3.3, 8, 14 and 17 show material,
void and boundary at the floor/shoulder levels. Every CSV row contains the query
coordinates, result, boundary face/edge identity and work. These are point samples
of the retained native solid; they do not approximate geometry used by the query.

The first argument is timed batch repetitions. Preparation and repeated-query
windows are measured separately; geometry construction and section export are
outside both. The fixed batch has 2,277 probes. Timings include scalar result
handling and work counters, with no mesh, scene, STEP, JS or rendering work.
Boundary preparation assumes an embedded outward-oriented regular solid; local
validation does not prove global validity. The example also prints a bounded-work
refusal without changing the retained body.

Allocation profiling uses the existing isolated QA harness. No profiling dependency
is added to production crates:

```sh
python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --workload native_solid_query --output /tmp/exedra-query-heap
env EXEDRA_SOLID_QUERY_PHASE=prepare python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --workload native_solid_query --output /tmp/exedra-prepare-heap
```
