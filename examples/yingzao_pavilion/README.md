# Yingzao-inspired pavilion

A small timber pavilion for exploring Exedra's construction workflow in a game
asset: round columns, curved bracket arms, stepped roof supports, a tiled gable
roof, and a stone platform. The model is assembled by ordinary Rust functions.

```sh
cargo run --release -p yingzao_pavilion -- --variants
```

Outputs go to `target/yingzao-pavilion/`:

- `pavilion.glb`: one 3.6 m bay, 3.6 m deep.
- `lacquered.glb`: the same geometry with red paint and green glaze assignments.
- `bracket-study.glb`: assembled and exploded views of the fitted bracket.
- `seat-study.glb`: assembled/exploded purlin seat, with its nominal bearing
  rectangle marked in green.
- `concave-study.glb`: sharp, chamfered, and filleted internal shoulders, left
  to right. These isolated shapes illustrate the available finishing behavior.
- `span-4500.glb` and `three-bay.glb`: span and repetition comparisons when
  `--variants` is supplied.
- `metrics.json`: setting-out/assembly time, cold geometry compilation time,
  export time, part and instance counts, unique and placed triangles, geometry
  buffer bytes, GLB bytes, material-edit cache work, and measured roof-bearing
  count, checked area, and verification time. Times exclude file IO
  and are individual observations, not benchmark distributions.

Set dimensions directly with `--bays`, `--span-mm`, and `--depth-mm`; use
`--output DIR` to keep another run. The example accepts 1–5 bays and dimensions
between 2400 and 5400 mm. `--variants` substitutes a 4500 mm span and three bays
respectively, retaining the other supplied dimensions. These are geometric
parameter limits, not structural span ratings.

## Render the exported geometry

With Blender available as `blender`:

```sh
blender --background --python examples/yingzao_pavilion/tools/render.py -- target/yingzao-pavilion
blender --background --python examples/yingzao_pavilion/tools/render.py -- target/yingzao-pavilion lacquered eye-level
blender --background --python examples/yingzao_pavilion/tools/render.py -- target/yingzao-pavilion three-bay eye-level
blender --background --python examples/yingzao_pavilion/tools/render.py -- target/yingzao-pavilion bracket-study
blender --background --python examples/yingzao_pavilion/tools/render.py -- target/yingzao-pavilion seat-study
blender --background --python examples/yingzao_pavilion/tools/render.py -- target/yingzao-pavilion concave-study
```

The script imports the GLB and adds lighting and cameras. It preserves exported
geometry, normals, and materials. The default run writes `eye-level.png`,
`brackets.png`, `roof.png`, and `pavilion.blend`; other cases prefix their image
names. On macOS the executable can be
`/Applications/Blender.app/Contents/MacOS/Blender`.

## Construction

`layout.rs` uses a `setout` network for the timber module, overall span, and roof
rise, plus an exact one-fen (15 mm) seat depth. Support tops follow the
finished bearing planes while the round purlins retain their roof datums.
`setout_generate` divides the exact width into bays, and `setout_joiner`
lowers evaluated dimensions and generated stations into geometry coordinates.

`brackets.rs` builds one canonical `joiner::Construction`. It authors a housing
in the bearing block and complementary cuts in the crossed arms, using
`joiner_timber::FitClass::CLOSE` for receiving-profile clearance. The example
composes the fitted recipes once and repeats them through `exedra_assembly`.
The existing timber rules target other connections, so these two scene-specific
fits are explicit `RuleOutput` records rather than a new general rule API.
Tests check removed volumes, bearing anchors, and compiled contact coverage.
`joinery.rs` holds the shared evidence/member registration helpers.

`seats.rs` fits circular purlins with `joiner_timber::RoundPurlinSeatRule` over
the actual support beams and eave pads. The cut opens a flat underside seat;
its bearing width is a circle chord, not the cylinder diameter. Shared fitted
purlin families retain the same part count as bays increase, while added
supports require real additional notches and triangles. Eave pads now center
under their purlins, and the upper bracket arms extend far enough to carry them.
Every generated variant verifies its roof contacts against the same shared
compiled parts used for export. The explicit 2 mm tolerance insets the analytic
bearing rectangle to cover the sideways chord error of coarse circle
sampling; metrics report that checked area. This does not prove contact over
the excluded boundary strip or discover collisions elsewhere in the frame.

`scene.rs` places shared timber, bracket, and tile parts. Curves use a 1 mm
chord tolerance, authored cylinder sections use 32 sides, and the platform
stones have 3 mm chamfers. `main.rs` maps opaque material IDs to untextured
glTF factors. Reassigning the materials must compile zero new parts and emit
zero new triangles. Repeated nodes share glTF meshes; renderer draw-call
batching and instancing remain the consuming application's responsibility.

## Historical scope

This is a modern visual interpretation. The timber section uses the documented
15-by-10 *fen* module; choosing one *fen* as 15 mm is an authored scale for this
example. The module is described in the
[2019 ISPRS computational study](https://isprs-archives.copernicus.org/articles/XLII-2-W15/1209/2019/isprs-archives-XLII-2-W15-1209-2019.pdf).

The roof follows the four-interval *juzhe* construction described in
[Andrew I-kang Li's “Computing Chinese Architecture”](https://link.springer.com/chapter/10.1007/978-3-031-81623-9_24):
raise the ridge by one third of the half-run, then depress successive working
lines by R/10, R/20, and R/40. The bracket outline, fits, remaining dimensions,
and finishes are authored here. The example does not reconstruct a particular
historical building. Sloped rafter seats, beam/post tenons, and longitudinal
purlin splices are still future work; the isolated concave study does not
round the fitted mating surfaces. The example supplies no
structural capacity analysis. Textures, LODs, and collision geometry can follow
from an actual consuming game's requirements.
