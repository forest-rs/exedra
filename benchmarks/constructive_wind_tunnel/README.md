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

## Browser authoring bridge workload

```sh
cargo run --release -p constructive_wind_tunnel --bin authoring_bridge
python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --output /tmp/exedra-authoring-allocations
```

This runs the actual browser panel/vent/recess/pin graph through the public Rust
bridge: JSON decode, native construction, metadata encoding, both part display
realizations, buffer staging and transfer, report parsing and prior-document
release. A new revision is installed before releasing the old one, preserving
the browser's real cache lifecycle. Each quality has three warmups and 101 edit
samples. Inputs are prepared outside timing. Output buffers stay alive at the
stop time and are released afterward. The native-host JSON parsing and Vec
moves are reported separately; they do not measure JavaScript parsing/copies,
SDK graph encoding, UI scheduling, scene updates or GPU presentation.

The allocation script reuses the existing isolated `dhat` profiling pattern;
its dependency stays outside the workspace and production graph. It prepares
requests and warms the session before counting exactly three edits. Counts
include host metadata/report parsing, output staging and destruction. Separate
checkpoints show allocations still retained by source/cache owners, then after
releasing the final document/cache. The script accepts `--repo` for a matched
baseline checkout, while keeping the same workload implementation. Inspect
`dhat.json` by owning stack before attributing all traffic to the kernel.

For CPU sampling, run the release binary with `sample`, attach
`sample <pid> 4 1 -file authoring.sample.txt`, then press Enter. This executes
repeated edits for 15 seconds. The `allocations` mode supports the repository's
external macOS stack-logging workflow, with pauses before/after three edits and
after release; exclude synthetic `flush_stack_log` stacks from that analysis.
The benchmark is host-only so existing Wasm geometry CI does not pull a browser
bridge into its dependency path.

The retained baseline/optimized summary is in `authoring-bridge-results.json`.
Four interleaved release rounds on arm64 with Rust 1.99.0 (404 edits per quality)
measured these median complete native bridge edit times:

| Quality | General triangle triangulation | Stack-based existing triangles |
| --- | ---: | ---: |
| Coarse 0.45 mm | 916 µs | 602 µs |
| Medium 0.12 mm | 1,149 µs | 719 µs |
| Fine 0.035 mm | 1,938 µs | 1,177 µs |

Medium realization and output staging fell from 993 to 565 µs; native build
stayed approximately 142 µs. Triangle counts, render bytes and source semantics
stay unchanged. The matched three-edit allocation window fell from 65,554 to
17,074 allocations and from 6,215,331 to 3,597,411 allocated bytes. Live tracked
source/cache allocations after the edits were unchanged at 529,739 bytes in 386
blocks, returning to zero after final release. This is allocation traffic and
tracked live heap, not RSS or total residency (warmup allocations are excluded).

CPU samples identified general polygon validation/ear clipping inside display
extraction of already-triangular native geometry. The final sample removes that
work; real native cap triangulation remains. The kernel's triangle path preserves
projection arithmetic, exact orientation checks, fallback diagnosis and historical
corner rotation, while using stack records. Native ARM64 release assembly already
uses packed two-double arithmetic in Newell projection without a SIMD crate.
These measurements establish the structural optimization, not a browser speedup
or a claim that the Wasm bundle uses SIMD.


## Bounded native accuracy refinement

```sh
cargo run --release -p constructive_wind_tunnel --bin accuracy_refinement
python3 benchmarks/constructive_wind_tunnel/compare_accuracy.py --before /path/to/before --after /path/to/after --output /tmp/accuracy-comparison
python3 examples/step_exchange/scripts/profile_bores.py --workload refinement --repo /path/to/before --output /tmp/accuracy-alloc-before
python3 examples/step_exchange/scripts/profile_bores.py --workload refinement --repo /path/to/after --output /tmp/accuracy-alloc-after
```

The same public workload runs against both source trees through isolated Cargo
packages. Three alternating rounds measure 101 release realizations per case,
after warmup, with source construction, validity/hash checks, output destruction
and file I/O outside timing. No compiler cache, JavaScript or GPU is involved.
Source heads and an analytic patch hash accompany results when testing uncommitted
changes. Optional target arguments reuse existing dedicated build directories.

The ordinary 0°/30°/60° bores, revolution, sweep and solid/hollow loft must retain
identical f32 positions, topology, native correspondence, sharpness and deviation.
A translated small bore must achieve the original target after refinement; an
edge-budget-capped variant must preserve its original unmet output. Reports
separate delivered triangles from completed work across both attempts. The
allocation window excludes native construction and warmup, includes retry work,
holds only the delivered output at its retained checkpoint, then releases it.
The isolated dhat package is a QA dependency outside the production graph.

## Native chart compilation and reuse

```sh
cargo run --release -p constructive_wind_tunnel --bin native_chart_compilation
cargo run --release -p constructive_wind_tunnel --bin authoring_bridge
python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --repo /path/to/before --output /tmp/chart-alloc-before
python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --repo /path/to/after --output /tmp/chart-alloc-after
```

`native_chart_compilation` measures native revolution, sweep and hollow-loft
snapshot compilation with and without charts, including actual extraction and
retained triangle/native correspondence. Each case records 101 fresh-compiler
and 101 warmed-cache samples after warmup, with sources/prepared identity and
verification outside timing. Cache-hit cases must keep one native realization;
applied chart evidence must match the validated request. Counts distinguish
native boundary-table payload from render vertices and delivered triangles.

`native-chart-compilation-study.json` also compares the unchanged actual
pocketed-panel authoring bridge at three qualities, across three alternating
before/after runs. Triangle counts, buffer bytes and cache work must match.
The default path keeps its allocation count and UV-buffer behavior; the empty
policy-evidence pointer adds 24 bytes across three retained records in the
allocation window (16 bytes at peak). Coarse timing has a noisy first after run;
medium/fine medians are effectively unchanged. No speedup is claimed.

All timing is native-host release work. It excludes JS/Wasm copies, SDK graph
encoding, UI/GPU, file I/O and returned buffer destruction. The allocation window
includes Rust-host metadata/report parsing and release, with requests/warmup
outside it. Charted fresh compilation does additional coordinate/mapping work;
charted cache hits reuse geometry and boxed policy evidence.

## Delivered vertex bounds

```sh
cargo run --release -p constructive_wind_tunnel --bin render_vertex_budget
EXEDRA_RENDER_VERTICES=250000 cargo run --release -p constructive_wind_tunnel --bin native_chart_compilation
python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --repo /path/to/after --output /tmp/render-alloc --workload render_vertices
EXEDRA_RENDER_VERTICES=4 python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --repo /path/to/after --output /tmp/render-refusal --workload render_vertices
```

`render-vertex-budget-study.json` compares the exact parent version and bounded
extraction across five alternating native-host release rounds. Default uncapped
compilation and the actual bridge are effectively neutral in latency; all bridge
allocation counts, traffic, retained bytes and peak bytes match exactly. The
bridge now requests the delivered bound before extraction, preserving its public
`budget.renderVertices` diagnosis. Native working-topology budgets stay separate.

The fixed extraction fixture has 1,000 disconnected triangles and 3,000 unique
vertices. Uncapped and an exact sufficient bound deliver identical geometry; a
bound of four refuses at the fifth unique vertex. Normal derivation is explicitly
excluded using custom-only missing normals, so the early-refusal measurements do
not imply a bound on normal/tangent scratch or whole-source work. The limit is
per body and successful counters match uncapped extraction. Refused output is
absent, and the reported requirement is a lower bound. Timing excludes returned
buffer destruction; allocation checkpoints include its release.

## Native coordinate poles

```sh
cargo run --release -p constructive_wind_tunnel --bin native_poles -- 9
cargo run -p constructive_wind_tunnel --bin native_poles -- 0
python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --repo /path/to/checkout --output /tmp/pole-alloc --workload native_poles
```

`native_poles` checks closed sphere and pointed-cone realizations at three chord
budgets before timing them. It reports native measures, certified deviation,
triangle count, minimum/first-percentile angle and longest-edge/altitude aspect
ratio. Zero repetitions performs deterministic quality inspection without timing.
Construction includes the retained source-profile clone. Realization uses a fixed
constructor; geometry diagnostics, output destruction, STEP and Wasm/JS/GPU work
are excluded from timings. The isolated allocation window retains the source
outside capture and includes one realization, correspondence, corner normals and
returned-output release. It does not measure total source or process residency.

Pass `sample` to the release executable, attach
`sample <pid> 4 1 -file native-poles.sample.txt`, then press Enter. The 15-second
window repeatedly realizes the retained R20 sphere at tolerance 0.025, including
output destruction. Construction, quality inspection and allocation logging
remain outside that CPU sampling window.

On arm64 with Rust 1.99, four interleaved baseline/candidate release rounds of
31 timed realizations per quality measured the full R20 sphere as follows.
These are medians of round medians, excluding output destruction.

| Tolerance | Before | Aligned radial rings | Before triangles | After triangles |
| --- | --- | --- | --- | --- |
| 0.4 | 0.286 ms | 0.188 ms | 1,920 | 800 |
| 0.1 | 1.190 ms | 0.620 ms | 7,936 | 3,168 |
| 0.025 | 5.053 ms | 2.264 ms | 32,256 | 12,768 |

The fine sphere's minimum emitted triangle angle improved from 1.405 to 19.590
degrees, with longest-edge/altitude aspect falling from 40.81 to 3.755. Its
certified deviation increased from 0.01355 to 0.01530, remaining below the
requested 0.025; the scheduler uses that tolerance rather than preserving an
incidental smaller prior error. Cone geometry and timings were neutral. This
optimization applies to certified full spheres; cones and trimmed spherical
caps retain their existing fan/grid schedule and have no minimum-angle guarantee.

The matched fine-sphere DHAT window fell from 102 to 87 allocation events and
10,621,945 to 4,304,609 allocated bytes. Peak tracked heap fell from 8,550,397 to
3,400,261 bytes; retained output fell from 5,512,932 to 2,194,788 bytes, returning
to zero after release. These counts cover realization and returned ownership,
not the retained source constructor or total process residency.

## Retained explicit BRep trims

`generic_brep` builds bodies directly from public geometry/topology/trim tables,
without a constructor-family wrapper. It compares support-only and explicit
cylinders, their two-edge and four-edge subdivided seam equivalents, and a
non-pole spherical patch. Every measured output checks topology, closure or the
expected open patch boundary, and the reported geometric tolerance. Construction,
validation/quality inspection and output drop are outside realization timing.
Two warmups precede the reported median.

```sh
cargo run --release -p constructive_wind_tunnel --bin generic_brep -- 9
python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --workload generic_brep --output /tmp/exedra-generic-brep-allocations
```

`EXEDRA_BREP_PROFILE` selects `support_cylinder`, `explicit_cylinder`,
`split_cylinder`, `four_split_cylinder` or `sphere_patch` for the isolated
allocation window (default: `sphere_patch`). DHAT is linked only into that
temporary external profiler; production dependencies are unchanged.
`generic-brep-study.json` records parameters, work counts, timings and tracked
allocation checkpoints.

At tolerance 0.025, ordinary cylinders retain 40 vertices and 76 triangles.
Explicit trim validation adds three allocation events and 30 traffic bytes;
retained output and peak heap are identical. Their approximately 46–50µs timings
are too close for a speedup claim. Two-edge and four-edge seam chains preserve
all split vertices with 78 and 82 triangles at approximately 47µs. These chains
are recognized as monotone collinear strip boundaries, and only their adjacent
panels receive extra triangles. The previous generic fallback produced 23,068
triangles at about 39ms for the same two-edge seam, consuming 14.85MB allocation
traffic and 6.29MB peak heap. The chain path needs 55.7KB traffic and 23.0KB peak.
Ordinary inputs keep their original triangle output and allocation event counts;
the compact optional strip descriptor adds eight bytes per planned patch.

The sphere remains a substantive generic-chart workload: 3,840 triangles at about
5.9ms for tolerance 0.025, including exact analytic corner normals. The report exposes refinement batches, triangle error
tests and generated vertices. Across its coarse/fine cases, 7–8 batches perform
4.9–5.2 error tests per final triangle. These inputs do not show quadratic scan
amplification; worst-case total scanning remains data dependent. Interior edge
incidence uses deterministic ordered maps. Skipping that map on the final
accepted pass and reserving exact split-triangle cardinality reduced the matched
sphere window from 5,087 to 4,173 allocation events and from 3,535,312 to
3,093,560 traffic bytes before analytic normal overrides were added.

Generic chart triangles now retain exact support normals through the ordinary
corner-normal override layer, selected by `CustomOrDerived`. On two independent
spherical patches the regression bounds normal-vector error below 2e-6 at display
tolerances 0.025 and 0.005; previously derived normals differed by up to 0.345 and
0.573. Positional tolerance alone does not constrain derived-normal error on thin
triangles. Evaluating each local UV normal once reduces the matched coarse/fine
normal-producing medians from 1.10/6.93ms to 0.88/5.92ms, compared with the original
derived-only 0.73/5.32ms. Topology and positional-error bounds are identical.
Temporary normal-table capacity peaks at 85,120/394,240 bytes; retained normal
value payload is 35,328/184,320 bytes, excluding sparse spare capacity. The fine
isolated output now retains 925,696 bytes (262,144 added normal-capacity bytes),
peaks at 1,363,800 bytes, and produces 4,199 allocation events with 4,199,416 traffic
bytes. All tracked bytes are released on output drop. These are allocation traffic/live-heap
measurements, not RSS or whole-process residency.

### Native manifold machining phases

The `native_manifold` binary measures source/tool construction, a retained cut,
mesh realization, and local native support/trim validation separately. Cases
include nonparallel exits, a blind floor, a stepped through bore, overlapping
parallel passage pairs/chains, and a 31-hole plate. Every prepared output must
realize as a closed mesh within tolerance before timing. Validation here means
`Body::validate`, not a global regular-solid certificate. Phase cases carry unit
metadata; the original `native_machining` binary retains its FaceRole/EdgeRole
workload for comparisons with earlier measurements.

```sh
cargo run --release -p constructive_wind_tunnel --bin native_manifold -- 100 7 all
cargo run --release -p constructive_wind_tunnel --bin native_machining -- 500 31
# Prints a PID, then repeatedly executes only the selected native phase.
cargo run --release -p constructive_wind_tunnel --bin native_manifold -- --sample overlap_chain cut 30
```

The existing isolated allocation harness supports the same workload without
adding a production dependency:

```sh
EXEDRA_MANIFOLD_CASE=plate_31 EXEDRA_MANIFOLD_PHASE=cut \
  python3 benchmarks/constructive_wind_tunnel/profile_authoring.py \
  --workload native_manifold --output /tmp/exedra-manifold-allocation
```

`EXEDRA_MANIFOLD_PHASE` accepts `cut`, `realize` or `validate`. Prepared inputs,
profiler setup and warmup are outside the allocation window. `retained` includes
the complete result and operation report; `released` follows their destruction.
Allocator bookkeeping and fragmentation are not included. Raw allocation
traces remain available for stack inspection alongside the summary.

`native-manifold-study.json` records matched source hashes, environment, repeated
phase timings, the original-role plate benchmark, and all scoped allocation
counts. Reusing each validated owned intermediate output as the next input,
sharing immutable source validation across parallel tools, fixed exit/normal
storage and one pattern reservation reduce the 31-hole cut's allocated traffic
by 52.6%. Its original-role median falls from 1.500 to 0.836 ms. Rendering remains
unchanged; the measured single nonparallel cut costs an additional 0.58 µs.

### Native spline-wall bounds

`native_spline_wall` measures complete body construction, ruled-surface regularity
with owned span clones, and mesh realization separately. Three solids share a
cubic spline closed by its chord: a polynomial prism, a rational prism and a
rational tapered loft. Geometry remains retained and exact; tolerances govern
only realization. The output includes per-face triangle counts, shared-edge
sampling, deviation bounds and retained type sizes. Timings include output drop;
validation and optional triangle JSON export run outside timing windows.

```sh
cargo run --release -p constructive_wind_tunnel --bin native_spline_wall -- 51 /tmp/spline-walls
# Retain at least 102 samples on the curved boundaries for matched-error studies.
EXEDRA_SPLINE_MIN_SEGMENTS=102 cargo run --release -p constructive_wind_tunnel --bin native_spline_wall -- 51 /tmp/spline-walls-fixed
# Request triangle shape separately from geometric accuracy.
EXEDRA_SPLINE_BOUNDARY_SPACING=0.1 cargo run --release -p constructive_wind_tunnel --bin native_spline_wall -- 51 /tmp/spline-walls-quality
# Prints a PID; repeats only the selected phase for macOS sample.
cargo run --release -p constructive_wind_tunnel --bin native_spline_wall -- --profile proof 1000000
EXEDRA_SPLINE_PHASE=proof python3 benchmarks/constructive_wind_tunnel/profile_authoring.py --workload native_spline_wall --output /tmp/spline-wall-heap
```

Allocation phases are `construct`, `proof` or `realize`. Source preparation,
warmup and profiler setup are excluded. The `retained` and `released` checkpoints
separate owned result storage from temporary allocation traffic. Minimum sample
counts apply to curved boundaries; realization can add further samples when
needed. These are accuracy-driven meshes. Triangle shape is a separate policy
and should be compared per surface rather than through density-weighted whole
mesh statistics.
General ruled grids still use accuracy-driven scheduling. A requested shape
ratio can remain unmet there; the optional `quality` report records the actual
remaining violations instead of implying that denser sampling met the target.
