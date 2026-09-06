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
