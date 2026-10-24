# constructive_probe

Composition probes for the constructive head. Each probe models something
a frontend would plausibly ask for (joinery cuts, pipe sweeps, lofts,
mirrored instances, drilled plates, curved Booleans) through the public
API only, then checks every contract the evaluator promises:

- deep mesh validity, closed shells, positive volume, and a source map
  pinned to the emitted mesh;
- volume against an analytic value where one exists, and the Euler
  characteristic against the expected topology;
- bit identity of geometry, attributes, and report between a cold and a
  warm cached evaluation;
- glTF export of every probe that emits geometry.

Running the binary writes one GLB per probe, `all.glb` with every probe on
a row, and `report.txt`:

```sh
cargo run -p constructive_probe -- <out_dir>
```

The GLBs are meant for headless review (for example Blender's Workbench
renderer with backface culling to expose flipped normals).

The test module pins each probe's outcome. Probes the kernel currently
refuses are listed with their typed reason, so a kernel improvement shows
up as a test that must be updated, and a regression shows up as a failure.

## Twisted dogbone ring

```sh
cargo run -p constructive_probe --bin twisted_dogbone_ring -- target/twisted-dogbone-ring.glb
```

Builds the build123d-style ring from a closed circular sweep of a dogbone
profile with four full section turns. It subtracts 21 radial cylinders along
the section's thin direction using the sweep's sampled guide frames, checks
that the result is a closed valid mesh,
and exports two colored instances, the second rotated 25 degrees. The final
GLB is written only when both the direct evaluation and assembly compilation
accept the geometry. Optional second and third arguments select a count and
first cutter index for isolating one cut.

## Surface quality and textured dogbone

```sh
cargo run --release -p constructive_probe --bin textured_dogbone_ring -- target/surface_quality
blender --background --python examples/constructive_probe/tools/render_surface_quality.py -- target/surface_quality
```

The shared dogbone builder keeps the original untextured command unchanged.
The textured study compares a toroidal mapping applied after all 21 cuts with
construction coordinates authored on the swept stock and separate cylindrical
coordinates on the cutters before the Boolean. Both paths retain complete UVs.
The construction chart repeats every 40 input units (millimetres in this sample);
profile and closed-path seams are explicitly those of the construction chart.

`comparison.png` shows six controlled cases, including an anisotropy heatmap.
`construction.png` and `construction-detail.png` show the construction mapping.
The renderer imports the GLBs unchanged; it does not repair normals, UVs or mesh
geometry. The grid is embedded in each textured export. `survey.json` records
image dimensions, input units, triangle counts, metric extrema, source faces and
labels, threshold coverage and bounded issue witnesses. The survey obtains its
triangulation from `CompilePolicy::extraction_params()` to match the exported
triangles. UV measurements describe the stored affine map, not an analytic rest
surface or a global-overlap test.

The lost-crease experiment removes only sharp-edge metadata, preserving all
positions and UVs. This changes derived corner normals while leaving the UV
quality unchanged. Baking the source normals before that metadata loss preserves
its shading through an adapter that carries authored corner normals. The native
recipe path retains the cut creases directly. The plain model checks and the
normal-transport unit regression distinguish this shading problem from UV
stretching.

The illustrative 8:1 directional-density limit is a caller choice. On this
fixed input, toroidal outliers cover about 12.5% of the measured surface; the
construction-chart outliers cover roughly 1.55e-9 of it. The remaining extreme
values are tiny geometric slivers, which is why reports include measured area as
well as triangle counts. These observations do not establish a universal quality
threshold. Construction regressions separately pin stock/tool UV coverage,
source attribution through height edits, and warm-cache replay.

## Tube junctions

```sh
cargo run -p constructive_probe --bin junctions -- target/junctions
blender --background --python examples/constructive_probe/tools/render_junctions.py -- target/junctions
```

Joins open tubes with `exedra_mesh_ops::junction` in four configurations: a
Y fork, a T junction, a four-way branch and a root spread whose arms all lie
in one hemisphere. Each result is capped, checked as a valid closed shell and
written as an OBJ with `tube` and `junction` groups; the script renders one PNG
per case with the skin in green.
