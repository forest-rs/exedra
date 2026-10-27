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

## Loft correspondence preparation

```sh
cargo run --release -p constructive_wind_tunnel --bin loft_correspondence
```

The binary prepares a circle, rounded rectangle and rectangle with default
seams. It measures 101 fresh preparations after warmup, keeping the result alive
until timing stops; input construction and output destruction are excluded.
JSON includes every sample, input/output segment counts and normalized-knot
adjustment. Meshing is a separate operation. On the development Apple Silicon
host, preparation took a median 0.056 ms for 14 input and 30 output segments
across three sections. This is a local observation, not a portable guarantee.

## Retained ruled lofts

```sh
cargo run --release -p constructive_wind_tunnel --bin native_loft
cargo run --release -p constructive_wind_tunnel --bin native_loft -- charted
```

Solid and hollow circle-to-rectangle fixtures share exact prepared source
profiles. Preparation is outside timing. The binary measures retained native
construction, fresh remeshing with and without charts, and direct mesh lofting
separately. Each phase warms up before 31 timed results; output destruction is
excluded. Closure, deep validity, tolerance and repeatable native construction
are checked before timing. The `native` and `charted` modes repeat a hollow
remesh for 20 seconds for an external profiler.

Native realization uses a combined 0.05 mm geometric bound. The mesh path uses
0.025 mm profile and 0.025 mm side-surface policies; its evidence reports those
separately. Geometry and triangle counts are not claimed to match exactly.
On the development Apple Silicon host, tighter mixed-derivative bounds and a
resolved final subdivision count reduced solid output from 23,916 to 4,956
triangles and remeshing from 3.92 to 1.21 ms. The hollow output fell from 34,480
to 8,848 triangles and 6.63 to 2.22 ms. The direct mesh path produced 4,640 and
8,532 triangles in 0.73 and 2.02 ms. Native construction took about 0.04 ms;
charted native remeshing took 1.54 and 3.26 ms. These are local measurements,
not portable guarantees or evidence of identical mesh quality.

An external DHAT run over three fresh charted hollow remeshes measured traffic
falling from 15,838,868 to 4,262,404 bytes per call, and 577 to 448 allocation
events per call. Peak live heap fell from 9,329,468 to 2,404,060 bytes. Inputs
and warmup precede the profiler window; output destruction is included in that
window. The profiler dependency remains outside the workspace. CPU sampling
shows the remaining work primarily in cap triangulation, mesh import and chart
delivery. Source-curve evaluations are cached across rows; per-cell heap
allocation is unnecessary.

## Native axial finishes

```sh
cargo run --release -p constructive_wind_tunnel --bin native_axial_finish
```

The binary fillets four selected native edges on blocks with 0, 16 and 64
existing blind pockets. Setup is outside timing. It checks native closure,
transition count and repeatable complete results before 31 fresh timings after
warmup; output destruction is excluded. A native-solid/provenance signature
supports comparisons across implementations.

Four alternating runs against a prefix-validation reference confirmed identical
solid and provenance signatures. Validating each complete end once reduced the
local median from about 0.19 to 0.044 ms with 16 pockets and from 6.48 to 0.34 ms
with 64. The empty-pocket case remained about 0.007 ms. Depth and opposite-floor
web checks remain active, and one native table build follows validation. These
are local Apple Silicon measurements, not portable guarantees.

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

### Native sweep realization

```sh
cargo run --release -p constructive_wind_tunnel --bin native_sweep
```

The smooth S rail uses the same 20 mm square section and radius/tangent path
through native and constructive realization. The native tolerance is 0.01 mm;
the recipe's centerline tolerance is 0.008 mm to match the outer radius 50 mm
to the centerline radius 40 mm. Both produce 484 vertices and 964 triangles.
Native closure, deep mesh validity and the native deviation report are checked.
Prepared inputs stay outside timing; outputs remain alive at the stop time and
are destroyed afterward. Four rounds alternate backend order, each with three
warmups and 21 individual samples. JSON retains every sample.

Pass `native` or `constructive` to the built executable for a 20 second CPU
sampling loop. Capture each backend separately, without concurrent builds.
An external DHAT capture over three fresh realizations measured 218 allocation
events and 365,732 allocated bytes per native call, with a 272,892 byte peak.
Before reserving known triangle-import sizes and eliminating duplicate incidence
records, the same native call used 244 events, 674,788 bytes and a 410,188 byte
peak. The exported sample GLB is byte-identical through that importer change.
Alternating before/after executables on Apple Silicon measured native medians
of 0.201/0.179 ms; the constructive control measured 0.171/0.167 ms.

### Doubly curved native sweep

```sh
cargo run --release -p constructive_wind_tunnel --bin rounded_sweep
```

A retained hollow rail has two toroidal bends and a cylindrical run. Fresh
realization at 0.01 mm produces 60,320 vertices and 120,640 triangles. Inputs,
closure/deep-validity checks and one warm realization stay outside timing.
Each of 21 samples stops while the output remains alive; destruction follows.
Pass `profile` to the executable for a 20 second CPU sampling loop.

macOS `sample` identified repeated trigonometry in grid emission. Reusing one
U phase schedule and two rows throughout realization reduced alternating
before/after medians from 20.5 to 15.9 ms on Apple Silicon. The exported four-part
sample remains byte-identical. External three-call DHAT windows measured total
allocation traffic of 123,448,332 bytes before, 120,553,308 after, with peak
live memory of 32,921,212 / 31,963,724 bytes. Imports without welding now borrow
positions and omit an identity remap; no allocation profiler is linked into
production crates.

### Matched build123d / OCCT comparison

[Reproduce the eight matched workflows and read the reference measurements](build123d/README.md).
The comparison separates authoring, prepared construction, exact queries, fresh
meshing, indexed output and end-to-end work, with independent geometric checks
and explicit angular/tolerance/cache policies. It includes both public build123d
extraction and indexed OCCT extraction through Python bindings.

## Native retention

```sh
cargo run --release -p constructive_wind_tunnel --bin native_retention
```

The rounded pocketed panel is installed once and displayed at coarse and fine
policies. Before timing, the workload verifies closed valid meshes, native floor
workplane access, final render-triangle correspondence, fresh equal-content
selection rejection, shared cached buffers, and snapshot survival after eviction.
It also registers the native part in a two-occurrence assembly and checks that
legacy interchange returns its typed unsupported-source diagnosis.

It reports installation with constructor cloning separately from cold realization
and prepared cache-hit snapshots. All raw timing samples are emitted, with three
warmups and destruction outside timing. Native boundary-table payload and cache
work counters are separate from mesh/render buffers; the payload is explicitly
not total source memory. Cache-hit samples must perform no new native realization.
