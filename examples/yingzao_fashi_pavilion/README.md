# Yingzao Fashi-inspired pavilion

A small timber pavilion for exploring Exedra's construction workflow in a game
asset: round columns, curved bracket arms, stepped roof supports, a tiled gable
roof, and a stone platform. The model is assembled by ordinary Rust functions.

```sh
cargo run --release -p yingzao_fashi_pavilion -- --variants
```

Outputs go to `target/yingzao-fashi-pavilion/`:

- `pavilion.glb`: one 3.6 m bay, 3.6 m deep.
- `exploded.glb`: supports, purlins, rafter courses and pins separated with 0.9 m
  layer spacing. Tiles and decking remain together as one layer.
  These are frame placements; both variants share compiled geometry.
- `lacquered.glb`: the same geometry with red paint and green glaze assignments.
- `bracket-study.glb`: assembled and exploded views of the fitted bracket.
- `seat-study.glb`: assembled/exploded purlin seat, with its nominal bearing
  rectangle marked in green.
- `rafter-study.glb`: two neighboring rafters assembled over their purlins,
  alongside an exploded view of their side lap, bearing cuts, and wooden pin.
- `concave-study.glb`: sharp, chamfered, and filleted internal shoulders, left
  to right. These isolated shapes illustrate the available finishing behavior.
- `span-4500.glb` and `three-bay.glb`: span and repetition comparisons when
  `--variants` is supplied.
- `metrics.json`: setting-out/assembly time, cold geometry compilation time,
  export time, part and instance counts, unique and placed triangles, geometry
  buffer bytes, GLB bytes, material-edit and roof-separation cache work, and measured roof-bearing
  count, side-fit count, checked contact area, and verification time. Times exclude file IO
  and are individual observations, not benchmark distributions.

Set dimensions directly with `--bays`, `--span-mm`, and `--depth-mm`; use
`--output DIR` to keep another run. The example accepts 1–5 bays and dimensions
between 2400 and 5400 mm. `--variants` substitutes a 4500 mm span and three bays
respectively, retaining the other supplied dimensions. These are geometric
parameter limits, not structural span ratings.

## Render the exported geometry

With Blender available as `blender`:

```sh
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion lacquered eye-level
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion exploded
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion three-bay eye-level
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion bracket-study
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion seat-study
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion rafter-study
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion pavilion eaves
blender --background --python examples/yingzao_fashi_pavilion/tools/render.py -- target/yingzao-fashi-pavilion concave-study
```

The script imports the GLB and adds lighting and cameras. It preserves exported
geometry, normals, and materials. The default run writes `eye-level.png`,
`brackets.png`, `roof.png`, `eaves.png`, and `pavilion.blend`; other cases prefix their image
names. On macOS the executable can be
`/Applications/Blender.app/Contents/MacOS/Blender`.

## Construction

The GLB retains a pavilion frame with foundation, timber and roof groups; the
tile layer is a child of the roof. The default has 47 parts, 1,520 geometry
instances, 17 spatial frames and 1,980 placed bodies. Moving the roof carries
its tiles without recompiling any geometry. Contact verification uses the
instance mapping returned by assembly composition and checks the actual world
placements in the assembled pose before measuring the fitted roof. The exploded
pose intentionally separates those contacts; it preserves geometry and materials.

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
`joinery.rs` holds the shared evidence/member registration and lowering helpers.

`seats.rs` fits circular purlins with `joiner_timber::RoundPurlinSeatRule` over
the actual support beams and eave pads. The cut opens a flat underside seat;
its bearing width is a circle chord, not the cylinder diameter. Fitted purlin
families retain the same part count as bays increase, while added supports
require real additional notches and triangles. Each purlin retains the source
references of its own cuts. Eave pads now center
under their purlins, and the upper bracket arms extend far enough to carry them.
`joiner::lower_shared` compares every composed recipe before sharing it within
an authored family. Different cuts or sources get separate parts; material
differences become instance bindings. Role, evidence and generated-part metadata
travel with each element. Every generated variant verifies its roof contacts
against the actual exported instances, checking their placements and compiled
recipe identities before measuring surfaces. The explicit 2 mm tolerance insets the analytic
bearing rectangle to cover the sideways chord error of coarse circle
sampling; metrics report that checked area. This does not prove contact over
the excluded boundary strip or discover collisions elsewhere in the frame.

`roof_section.rs` turns the setout datums into one piecewise roof section.
Rafters, continuous decking, and tile courses share its slopes. Layer thickness
is measured normal to the roof; adjacent offsets meet at mitres and the two
roof halves meet on the ridge plane.

`rafters.rs` adds eight short rafter shapes, repeated across the building. Their
cutters use local coordinates so repetition shares exact recipes without a
geometry-merging tolerance.
Each end bears on a shallow flat cut in its circular purlin. At every pitch
change, including the ridge, half-width tongues form a 100 mm supported side
lap with a 12 mm wooden pin through matching bores. The top purlin cuts run
continuously along the timber. These scene-specific fits use ordinary
`joiner::RuleOutput` edits, contacts and generated pins; they add no public rule
API. Contact coverage checks use the exported shared parts. Regression samples
also check pin clearance and interference between neighboring timbers; finite
sampling is not a general collision certificate or a pin-bearing calculation.

`tiles.rs` lays courses continuously from eave to ridge, following chords across
pitch changes. The 340 mm clay shells taper so an uphill cover nests outside
the previous cover, while a pan nests inside the previous drainage channel.
Their overlaps have real thickness and clearance; coincident extrusions no
longer produce bands at each purlin. Round lotus end faces and pointed drip
aprons finish the eaves, and closed ridge ends project past the gable rows.
The shell spacing leaves room for bedding, which is not separately modeled.
The clay samples check overlap, decking clearance, ridge clearance and outward
winding at the same tessellation tolerance as the exported scene.

`scene.rs` places shared timber and studies. Curves use a 1 mm
chord tolerance, authored cylinder sections use 32 sides, and the platform
stones have 3 mm chamfers. `lib.rs` maps opaque material IDs to untextured
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
historical building. The round eave faces (*wadang*) and pointed drip tiles
(*dishui*) follow the forms described by
[The Met's roof-tile collection](https://www.metmuseum.org/art/collection/search/49228)
and its [Astor Court guide](https://www.metmuseum.org/-/media/files/learn/family-map-and-guides/edu3337_asianart_astorcourt_family_guide_061721_v8.pdf?sc_lang=en).
Lotus tile terminals are represented among the
[Song–Yuan archaeological finds](https://www.amo.gov.hk/graphics/ePamphlet_sung_wong_toi.pdf);
this example's eight-petal relief is an authored motif, not a copied artifact.
Beam/post tenons and longitudinal purlin splices are still future work;
the isolated concave study does not
round the fitted mating surfaces. The example supplies no
structural capacity analysis. Textures, LODs, and collision geometry can follow
from an actual consuming game's requirements.

The package also exposes `yingzao_fashi_pavilion::pavilion(bays, span_mm,
depth_mm)`, returning the same fitted `Assembly` for other scene examples. The
command-line exporter and its arguments remain unchanged. The courtyard
example uses this entry point for its sheltered edge.
