# Matched build123d / Exedra workflows

This benchmark compares eight identical, valid parts: rounded plates with
0/16/64 holes; three instrument-panel variants with through features and blind
pockets; and full/partial revolutions of the same stepped, tapered fitting.
It measures Exedra's retained analytic path and build123d 0.13.0 with its OCCT
backend. It does not compare every capability of either system.

Both workers read [fixtures.json](fixtures.json). Exact volume and area must
agree with independent formulas before a result is accepted. The driver also
checks oriented, geometrically closed triangle meshes, a broad independent mesh
volume guard, and sampled distance to the exact boundary.

## Reference measurements

Measured on macOS 27 ARM64 with Rust 1.99, Python 3.13.17, build123d 0.13.0 and
cadquery-ocp-novtk 8.0.1.1.0. Source revision: d942d85. Standard medians use three
alternating rounds of three samples after two warmups per phase. OCCT parallel
meshing is enabled; Exedra's current realizer is single-threaded. These are local
measurements, not CI thresholds.

| Workload | Exedra construction, µs | build123d construction, µs | Exedra mesh, ms | OCCT mesh, ms |
| --- | ---: | ---: | ---: | ---: |
| 64-hole plate | 46.38 | 3,554.96 | 81.696 | 166.203 |
| Panel | 255.54 | 16,008.00 | 4.220 | 22.640 |
| Full fitting | 3.42 | 372.79 | 0.504 | 3.672 |
| Cutaway fitting | 4.83 | 528.67 | 0.392 | 3.082 |

Construction starts from prepared profiles/faces and excludes profile authoring.
Native construction still includes its own boundary checks; OCCT's broader
is_valid check runs outside the timers. The standard angular constraint produces
substantially more OCCT triangles, so mesh times alone are not an equal-output
comparison.

| Workload | Exedra triangles | OCCT triangles | Exedra sampled error, mm | OCCT sampled error, mm |
| --- | ---: | ---: | ---: | ---: |
| 64-hole plate | 5,500 | 33,036 | 0.018864 | 0.000466 |
| Panel | 1,556 | 7,172 | 0.019261 | 0.001205 |
| Full fitting | 2,516 | 5,820 | 0.019823 | 0.016712 |
| Cutaway fitting | 1,934 | 4,400 | 0.019471 | 0.015887 |

Relaxing OCCT's angular constraint to 3 radians, while retaining the 0.02 mm
absolute linear target, gives these accuracy-checked sensitivity measurements
(two rounds of three samples):

| Workload | Exedra mesh, ms | OCCT mesh, ms | Exedra triangles | OCCT triangles |
| --- | ---: | ---: | ---: | ---: |
| 64-hole plate | 82.896 | 18.066 | 5,500 | 7,596 |
| Panel | 4.508 | 5.057 | 1,556 | 2,140 |

The holed plate is a real weakness: OCCT meshes it about 4.6× faster with more
triangles and lower sampled error. A six-second native CPU sample is dominated
by ear clipping and orientation predicates in planar-cap triangulation. Both
workers create one outer loop plus 64 circular inner loops and extrude once;
neither performs a sequence of mesh Boolean hole punches. Exedra's profile
authoring/validation takes about 8.5 ms separately from its roughly 49 µs
prepared-body construction.

| Relaxed-angle end-to-end workload | Exedra, ms | build123d public extraction, ms | Indexed OCCT extraction, ms |
| --- | ---: | ---: | ---: |
| 64-hole plate | 90.960 | 346.613 | 60.228 |
| Panel | 4.895 | 224.828 | 38.365 |

The lower-level CAD pipeline wins the 64-hole end-to-end case too. The public
build123d path is slowed by its triangle-array iterator: native sampling found
most main-thread samples in iterator/exception dispatch. Indexed Triangle(i)
access avoids that path without changing output buffers. The separate public
and indexed measurements distinguish binding iteration from meshing work.

| Process measurement | Exedra | Python + build123d + OCP |
| --- | ---: | ---: |
| Startup until metadata | 6.07 ms | 2,050.08 ms |
| Startup peak RSS | 2.11 MiB | 399.09 MiB |
| Whole capture peak RSS | 15.80 MiB | 606.78 MiB |

RSS includes the entire runtime, libraries and capture process. It is not
per-model storage or an allocation count. All eight cases, raw timing samples,
package versions, source/binary hashes and geometric evidence are in
[reference-results.json](reference-results.json).

Both CAD variants construct through build123d. Lower-level OCCT here means
absolute-deflection meshing and indexed extraction through OCP's Python
bindings; this is not a standalone C++ construction benchmark.

## Reproduce

Use Python 3.13 and a release Rust worker. The Python environment is separate
from the Cargo workspace; the CAD packages are benchmark tools only.

~~~sh
python3.13 -m venv /tmp/exedra-build123d-env
/tmp/exedra-build123d-env/bin/python -m pip install -r benchmarks/constructive_wind_tunnel/build123d/requirements.txt
cargo build --release -p constructive_wind_tunnel --bin build123d
/tmp/exedra-build123d-env/bin/python benchmarks/constructive_wind_tunnel/build123d/compare.py \
  --rust-bin target/release/build123d \
  --output /tmp/exedra-build123d-results --rounds 3 --repetitions 3
~~~

The capture driver uses /usr/bin/time on macOS/Linux for peak process RSS.
No benchmark runs concurrently with its other backend. The order alternates
between rounds; each phase warms twice before recording samples. Startup is a
separate process measurement, outside operation timings. The driver rejects
Rust workers built with debug assertions and records binary/input/source identity,
package versions, raw samples, output sizes and quality evidence.

For an angular-resolution sensitivity check:

~~~sh
/tmp/exedra-build123d-env/bin/python benchmarks/constructive_wind_tunnel/build123d/compare.py \
  --rust-bin target/release/build123d \
  --output /tmp/exedra-build123d-linear-plate \
  --case plate_64 --angular-tolerance 3 --rounds 2 --repetitions 3
~~~

report.json is the machine-readable survey. The per-round JSON captures contain
the actual mesh buffers, and stderr files preserve process resource counters.
Results are not timing assertions in CI.

## What is timed

| Phase | Work |
| --- | --- |
| author | Build checked profiles and pocket descriptions, or CAD faces/wires for the same inputs. |
| construct | Create a retained native body from prepared inputs, including the blind cuts. |
| measures | Query exact volume and area. Native revolutions cache these; these query costs do not represent the same arithmetic as recomputed OCCT mass properties. |
| mesh | Freshly realize an existing body. OCCT triangulation caches are cleared before the timer. |
| extract | Copy an existing triangulation into owned position/index arrays. |
| end_to_end | Author, construct, mesh and extract one new part. |

Creation timers stop while outputs are alive; destruction happens outside
these timers. Native sub-clock measure queries use optimizer-resistant inputs
and a 1,024-call batch. Python measure calls are individually timed.
The capture also records direct OCCT extraction/end-to-end timings, bypassing
build123d's Vector/Face wrappers and the binding's triangle-array iterator.
Indexed Triangle(i) access and the public path must produce identical mesh buffers.

Exedra emits an independent half-edge mesh and then copies its triangle data.
OCCT stores triangulation on the body and emits vertices split at face
boundaries. Native output positions are f32; OCCT's are f64. Coordinate welding
is used only for the driver's closure check; output buffers and timed operations
are untouched.

## Accuracy and cache policy

The shared linear target is absolute model-length deflection (0.02 mm by default).
build123d's public mesh() passes relative=True to OCCT. The worker therefore
uses OCCT's absolute-deflection mesher explicitly; using the same numeric argument
in the two high-level APIs would not match their error policies.

The standard OCCT angular constraint is 0.1 radians. Exedra's chord-tolerance
policy has no equivalent additional angular constraint, so triangle counts can
differ substantially. The sensitivity command relaxes this constraint to show
how much a result depends on output resolution. Read counts and measured errors
alongside timings.

Public tessellate() can remesh an existing body. The worker validates a separate
cache-acceptance threshold before using it as an extractor, preventing a hidden
relative-deflection remesh. That threshold is recorded in each raw capture and
does not change the already generated geometry.

The native realization report is retained. The external survey checks 64
uniformly selected triangles' centroids and edge midpoints against the exact
OCCT boundary shells, including points inside the solid. Distance to a solid
itself would incorrectly return zero for those points. This survey is a witness,
not a certificate of global maximum deviation. All rounds receive volume and
topology checks; the detailed distance survey uses each backend's first capture.

Peak RSS belongs to the entire measurement process: runtime/imports, libraries,
prepared inputs, mesh buffers and JSON capture. It is not per-model storage or
an allocation count. Python tracemalloc is deliberately not presented as OCCT
memory accounting because it misses native C++ allocations.

## Scope and authoring

The earlier executable study matched the three panel variants' exact measures
within roughly 1e-10 and exercised both build123d algebra and builder styles.
It led to semantic pocket selection/workplanes (#120) and exact profile-region
composition plus line-line corner treatment (#122). Reordering pockets changed
which feature an index selected while preserving caller feature labels.

build123d still offers a more compact Python CAD DSL, broad geometric selectors,
arbitrary 3D booleans and operations beyond the native subset measured here.
Our workplane construction creates independent bodies; it is not arbitrary
side drilling into a host. Feature labels do not promise general split/merge
history. Native profile revolution currently excludes arcs and axis contact.
No general CAD-kernel parity or language-wide speed claim follows from these
measurements.

## What to improve next

The evidence points to holed-cap triangulation first: the default EarClip path
used here does not run the optional constrained-Delaunay edge-flipping pass.
Keep robust orientation and boundary preservation while reducing the repeated
candidate/containment work identified by the CPU sample.

Authoring work should continue to reduce profile conversion/validation glue,
feature-to-realization selection, and tag/region/material bookkeeping. Reusable
checked inputs matter: the 64-hole profile costs milliseconds to prepare while
constructing another body from it takes tens of microseconds. Both current plate
recipes construct each circular cutout separately; reusable-circle authoring is
another distinct measurement opportunity, outside the prepared-body timings.

Capability comparisons should remain concrete: selected-face cutting versus
independent workplane construction, line-arc/arc-arc corner transitions, curved
revolution profiles and poles, and useful correspondence across later operations.
The benchmark identifies work and preserves evidence; it changes no geometry
algorithm or production dependency.
