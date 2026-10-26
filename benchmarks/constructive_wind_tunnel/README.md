# constructive_wind_tunnel

Wind-tunnel scenarios for the constructive geometry head, following the
workspace convention: argv-selected profiles, one `key=value` summary line
per run, and a signature-determinism assertion *before* any timing — the
wind tunnel is a determinism regression oracle first and a perf harness
second.

**CT-1** compiles and evaluates a batch of parameterized rounded-profile
extrusion recipes (arcs exercise the libm discretization path), asserts
bit-identical trimesh signatures across two full passes, exercises
source-map forward/reverse lookups at scale, and reports timing plus
introspection counters.

**CT-2** compares a cold grouped-recipe rebuild with a one-node edit through
the evaluation cache, while pinning the warm result against a full rebuild.

**CT-3** mirrors both gallery drill paths: the public constructive CSG card and
the direct 16-sided mesh Boolean used by the rounded-drill export. It isolates
constructive evaluation, direct Boolean, sharp-edge rounding, and render
extraction; deep-validates and signature-checks every result before timing;
then reports best and average phase times with Boolean/rounding work counters.

**CT-4** compares constructive stretch's algebraic box and extrusion paths
against its closed imported-mesh fallback. It validates and signature-checks
each output before reporting the paths separately, so tessellation-policy or
topology regressions cannot hide inside an aggregate number. The stress
profile uses a five-digit-vertex closed prism to expose work that scales with
the whole source mesh per output corner.

**CT-5** measures spherical corner tessellation on a 6 mm rail fillet and a
skewed rail at four chord tolerances, from 1 mm display geometry to 0.02 mm.
It reports corner/total triangle counts,
vertex counts, position/normal/index buffer bytes, and rounding/extraction times.
Deep validity and repeatability of geometry and authored render normals are
checked before timing.

**CT-6** repeats the CT-3 constructive through-drill with distinct panel and
cutter material slots. It checks geometry and ordered face-to-slot signatures,
requires both materials to survive, and reports evaluation time and face counts
per slot.

**CT-7** measures a 256-station polyline sweep with a small rectangular
section. It isolates the ordinary sweep path while frame and guide storage
changes, reporting best and average time per sweep.

```sh
cargo run --release -p constructive_wind_tunnel -- --quick
cargo run --release -p constructive_wind_tunnel -- --ct1-stress
cargo run --release -p constructive_wind_tunnel -- --gallery
cargo run --release -p constructive_wind_tunnel -- --gallery-stress
cargo run --release -p constructive_wind_tunnel -- --gallery-sample
cargo run --release -p constructive_wind_tunnel -- --stretch
cargo run --release -p constructive_wind_tunnel -- --stretch-stress
cargo run --release -p constructive_wind_tunnel -- --corners
cargo run --release -p constructive_wind_tunnel -- --corners-sample
cargo run --release -p constructive_wind_tunnel -- --materials
cargo run --release -p constructive_wind_tunnel -- --materials --ct1-stress
cargo run --release -p constructive_wind_tunnel -- --materials-sample
cargo run --release -p constructive_wind_tunnel -- --sweep
```

`--gallery-sample` repeats the unchanged CT-3 workload long enough for an
external sampling profiler to attach; it is not a distinct benchmark case.
`--corners-sample` does the same for CT-5.
`--materials-sample` does the same for CT-6.

## UV authoring

```sh
cargo run --release -p constructive_wind_tunnel --bin uv_mapping -- 128 15
cargo run --release -p constructive_wind_tunnel --bin uv_mapping -- 256 15
```

The dedicated binary measures all eight projection modes, with fresh and
retained authoring scratch. Planar, box, face, existing and perspective cases
use an imported quad grid; angular cases use a closed toroidal grid with half
as many faces. Mesh creation is outside timing. A warmup precedes the median
of the requested repetitions, and alternating transforms require actual writes.
Deep validation and bit-identical UV signatures across fresh/reused execution
paths precede timing. A last-face selection checks that tiny updates remain
proportional to the selection on a large authored mesh.

Reports include corners per second and retained scratch capacity. Capacity
excludes allocator overhead, mesh storage and returned face lists. Reusing
scratch removes buffer growth after warmup; reports still own their face lists.
The kernel merges sorted sparse writes in place. Linear mapping gathers small
batches and separates checks from arithmetic to permit compiler vectorization;
angular mapping reuses face-local sorting buffers.

On the development Apple Silicon machine, release medians over 15 repetitions
for 65,536 corners were 1.17 ms planar, 2.19 ms box and 1.87 ms existing-coordinate
mapping, compared with the initial implementation's 3.49, 4.34 and 3.02 ms on
the same workload. At 262,144 corners planar throughput remained about 56 million
corners/second. A four-corner last-face update took 0.29–0.35 microseconds across
those mesh sizes. These are local observations, not portable timing guarantees.

The same binary also measures `uv::quality` surveys with fan and robust
triangulation, including a last-face selection for each. On the local Apple
Silicon host, release medians over 25 repetitions for 32,768 stored triangles
were 3.05 ms with fan triangulation (10.7 million triangles/second) and 10.18 ms
with robust triangulation (3.2 million triangles/second). Both retained 96 bytes
of operation scratch. A selected two-triangle face took about 0.22 microseconds
with fan triangulation and 0.65 microseconds with robust triangulation, retaining
128 bytes. These timings include UV reads, triangulation, differential
measurements and summary construction. Bounded issue witnesses are output
allocations; robust nontriangle polygons also allocate temporary buffers inside
the kernel. Retained scratch is not a measure of those transient allocations.
Repeatable reports are validated before timing. These are local observations,
not portable timing guarantees.

For external allocation inspection, set `EXEDRA_UV_PROFILE=warm` (or `cold`)
when running `target/release/uv_mapping`. The process pauses before and after
three mappings for an OS profiler to attach. Stack-log flush allocations happen
outside this batch under their own stack; exclude them from mapping counts.
Capacity stability alone is not evidence of zero transient allocations.

## Analytic profile regions

```sh
cargo run --release -p constructive_wind_tunnel --bin profile_regions
cargo run --release -p constructive_wind_tunnel --bin profile_regions -- 5
```

The dedicated binary measures a filled cross and radial polygon unions at
48, 192 and 384 segments per input. Input construction is outside timing;
each timed operation includes requested input validation, intersection and
classification, output validation, allocation and drop. Geometry and complete
correspondence are checked for deterministic repeats before timing. Reports
include input and output work, geometry signatures and median microseconds over
seven batches. The optional positive multiplier increases batch repetitions.

On the development Apple Silicon host, three interleaved runs of the initial
per-segment event-list implementation and the final flat representation with
conservative broad-phase bounds gave medians of 7.58 to 5.08 microseconds for
the cross, and 20.05 to 8.62 milliseconds for the two 384-segment radial inputs.
Geometry signatures were identical in every comparison. These are local
observations with full validation included, not portable timing guarantees or
allocation counts. The final path removes per-segment cut-list allocations,
per-pair contact vectors and copies of output ring arrays; graph indexes,
validation buffers, output profiles and correspondence still allocate.

## License

Apache-2.0 OR MIT

### Retained analytic body

```sh
cargo run --release -p constructive_wind_tunnel --bin analytic_body -- 9
```

Times native extrusion construction and compares its specialized realization
with construction-independent `Body::realize` on rounded plates with 0, 8, 32
and 128 circular holes. Inputs are prepared outside realization timings; output
allocation and destruction are included. Each case checks closure, topology,
tolerance and equal triangle counts before alternating the two timed paths.
The generic path emits triangles throughout; the extrusion path retains wall
quads. Both share the same planar triangulator, so complex cap costs are shared.
These timings are comparisons of work paths, not a speedup claim.

For external allocation tracing, set `EXEDRA_ANALYTIC_PROFILE=body` or `extrusion`
on the built executable. It warms once, waits for Enter, runs exactly three
realizations, then waits again so before/after histories can be collected.

### Native and mesh revolution

```sh
cargo run --release -p constructive_wind_tunnel --bin native_revolve -- 15
```

Compares full and partial turns of the sample's stepped, tapered fitting at
24, 64 and 128 angular intervals. Reports native construction, plain realization
and construction-aware UV realization alongside existing mesh revolution.
Both paths must be valid, closed and have equal final triangle counts; the
native report must meet its tolerance. Inputs are prepared outside realization
timings, output destruction is included, two iterations warm each path, and
three-call batches rotate order before taking medians.

Set `EXEDRA_REVOLVE_PROFILE` to `native`, `native-uv`, `mesh` or `mesh-uv` on the
built executable for external allocation tracing. After one warm call it pauses
before and after exactly three realizations. The flush allocations are separate
from realization stacks; exclude them when comparing allocation histories.
For CPU sampling, also set `EXEDRA_REVOLVE_SECONDS=12`. Start `sample <pid> 6 1`
while paused, then press Enter to run repeated realizations for that duration.
Profile each mode separately, without allocation logging or concurrent workloads.

### Matched build123d / OCCT comparison

[Reproduce the eight matched workflows and read the reference measurements](build123d/README.md).
The comparison separates authoring, prepared construction, exact queries, fresh
meshing, indexed output and end-to-end work, with independent geometric checks
and explicit angular/tolerance/cache policies. It includes both public build123d
extraction and indexed OCCT extraction through Python bindings.
