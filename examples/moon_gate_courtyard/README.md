# Moon-gate courtyard

An authored Chinese garden scene: a circular masonry entrance frames a tree,
pierced garden stones, bamboo, paving, and the fitted timber shelter from the
[Yingzao Fashi example](../yingzao_fashi_pavilion/). This is a visual construction
study, not a reconstruction of a particular historical courtyard.

```sh
cargo run --release -p moon_gate_courtyard
cargo run --release -p moon_gate_courtyard -- --radius-mm 1350 --output target/courtyard-small-gate
```

The default entrance has a 1550 mm clear radius; supported variants range from
1350 to 1700 mm. The circle's bottom stays below the approach as its radius
changes. Outputs are `target/moon-gate-courtyard/courtyard.glb` and
`metrics.json`. Curves compile at a 1 mm chord tolerance. Export refuses
constructive geometry warnings.

## Render

With Blender on the command path:

```sh
blender --background --python-exit-code 1 --python examples/moon_gate_courtyard/tools/render.py -- target/moon-gate-courtyard gate
blender --background --python-exit-code 1 --python examples/moon_gate_courtyard/tools/render.py -- target/moon-gate-courtyard garden
blender --background --python-exit-code 1 --python examples/moon_gate_courtyard/tools/render.py -- target/moon-gate-courtyard overview
blender --background --python-exit-code 1 --python examples/moon_gate_courtyard/tools/render.py -- target/moon-gate-courtyard wall-cap wall-corner
```

Each requested view writes its named PNG. The script saves `courtyard.blend`
with the last camera active; multiple view names can be rendered in one run.
The script imports the exported GLB and supplies lighting and cameras. Every
wall, brick, tile, branch, leaf, and stone is already in the GLB; the render uses
no external assets or replacement materials. `masonry` is another close view.

## Construction and ownership

- `joiner_masonry::RunningBondRule` generates the entrance wall's individual
  units, circular opening and radial surround. Repeated recipes share parts;
  per-instance material bindings vary their appearance. The logical wall has
  no duplicate solid, and units retain joiner evidence and generation metadata.
- `architecture.rs` composes the enclosure, plaster skins, tiled caps, lattice,
  paving and planting bed. Side and rear walls are simple solid recipes; only
  the entrance has individual masonry units.
- `wall_caps.rs` lays pan channels and cover tiles down both outward slopes of
  pitched stone coping. A supported ridge shelters their upper ends. Fixed
  spacing preserves the clay fit; square end cuts and matched corner mitres
  keep adjacent wall coverings within their own footprints. Interior tile
  recipes remain shared. The exported triangles are checked for coverage,
  corner bounds and sampled clay interference.
- Paving falls 1% toward two channels, which fall 0.3% toward the entrance.
  `drainage.rs` cuts outlets through the buried masonry using existing joiner
  part edits. Channels extend beyond the entrance; no flow capacity or downstream
  drainage network is modeled.
- The shelter reuses `yingzao_fashi_pavilion::pavilion`, including its setting
  out, timber joinery, rafters and tiled roof. `Assembly::append_selected`
  mounts its hierarchy under the courtyard while excluding its standalone ground.
  The returned scene retains shared parts, bindings and metadata.
- `planting.rs` is a small, deterministic example-local mesh generator. Tapered
  polygon tubes form branches and bamboo; folded leaf meshes form the foliage.
  It is stylized vegetation, with clustered leaves rather than botanical twig
  attachments. No general plant API is introduced.
- `rocks.rs` extracts an authored scalar field with the existing dual-contouring
  implementation. A single pierced stone mesh is reused at two sizes.
- `main.rs` maps caller-owned material keys to untextured glTF appearance.
  Changing a gate material binding reuses the compiled geometry.

The GLB retains a courtyard frame with entry, ground, enclosure, shelter,
planting and rock groups. The shelter retains its own foundation, timber,
roof, support, purlin, rafter-course, pin and tile groups. Moving the courtyard or a module changes local placements;
a retained `PartCompiler` reuses the same geometry and reports.

The default scene has 2,147 masonry units, 313 parts, 5,337 geometry instances
and 23 spatial frames. Multi-body parts produce 5,677 placed bodies.
Its 57,713 stored triangles become 321,193 triangles with placement multiplicity;
the GLB is approximately 6.0 MB. `metrics.json` reports generation, compilation
and export times separately, excluding file IO. These are individual observations,
not benchmark distributions. Repeated glTF meshes share geometry; renderer
batching remains the consuming application's responsibility.

The masonry rule models one unit through the wall thickness and open mortar
joints. It does not claim structural capacity or contact coverage across those
gaps. Closure cuts at rectangular wall ends respect stock dimensions; trimming
around the circle can still leave small pieces. Mortar solids, interlocking
wall corners and detailed closure selection around openings remain outside this
first slice.
