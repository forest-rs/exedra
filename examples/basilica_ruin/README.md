# Byzantine basilica ruin

This executable scenario is the first concrete realization of
[`docs/worked_example_basilica.md`](../../docs/worked_example_basilica.md). It
builds a recognizable Byzantine basilica ruin through the public constructive,
assembly, extraction, and glTF seams.

The default model has a tall central nave, two attached lower aisles, genuine
interior nave-to-aisle arcades beneath a continuous clerestory, an eastern
apse, and a shallow dome carried by an articulated twelve-sided drum, four
named faceted pendentive webs, and a pierced square crossing stage. One
restrained broken clerestory and roof bay supplies the ruin cue. Its
architecture is deliberately clearer than its ornament: the scenario is a
workflow proof and visual regression fixture, not an archaeological
reconstruction.

## What setting-out means here

In building work, **setting-out** is the translation of design intent and
controlling measurements into the datums, points, dimensions, and alignments
from which construction is located. It is the connective tissue between “the
nave has this span and rise” and “this wall plate, rafter seat, ridge, gable,
and roof face belong at these exact points.”

The basilica now has one exact setting-out hypothesis for its primary massing.
`BasilicaPremises` contains positive physical dimensions and a bay count;
`BasilicaSetout` derives signed plan coordinates, vertical datums, the aisle
roof, nave roof, crossing, and east-end sections. Architecture modules consume
those resolved sections and lower exact values to floating point only where a
constructive recipe or placement requires it. No module receives the raw
premises or reconstructs an intermediate floating-point layout.

This extends the earlier roof correction: the nave roof still passes through
the top of a modeled continuous wall plate before continuing through its eave,
but its wall seats, ridge, gables, crossing height, aisles, apse, and the
separate Joiner structure laboratory now share the same exact spine.

Migration note: `BasilicaParams` is replaced by `BasilicaPremises`; its `f64`
dimensions are now `setout::Length`, `crossing_x` is the positive station
distance `crossing_station`, and `arcade_bays` is `setout::Count`.
`BasilicaRoofSetout` is now `BasilicaSetout`, with the section accessors
`plan()`, `levels()`, `aisle()`, `roof()`, `crossing()`, and `east_end()`.
`BasilicaReconfiguration::topology_changed` distinguishes a bay-count edit
that must add or remove repeated instances from an ordinary geometry update;
the named dirty frontier alone can describe only identities that already
exist. `buttress_stations` carries the new exact generated fragment and
`buttress_delta` reports retained, added, removed, and moved semantic items.

The migration also corrects the south lean-to roof's old placement frame. Its
baseline was mirrored, but its right-handed thickness axis pointed inward and
down, so that skin sat one roof depth below the north-side mirror. Both skins
now use proper-rigid frames with outward/up thickness, pinned by a geometric
symmetry test and the rendered review artifact.

Run it from the workspace root:

```sh
cargo run -p basilica_ruin
```

This writes deterministic artifacts to:

```text
target/basilica_ruin/basilica_ruin.obj
target/basilica_ruin/basilica_ruin.glb
```

The grouped OBJ is the accepted Z-up visual artifact. The GLB uses
`exedra_gltf`'s explicit Z-up-to-Y-up conversion and preserves stable instance
paths and part keys on its nodes.

Choose a different OBJ path with `--obj`; the GLB is written beside it:

```sh
cargo run -p basilica_ruin -- --obj target/my-basilica.obj
```

The summary printed to stdout includes assembly, triangle, diagnostic, and
content-signature counters.

To exercise incremental part compilation, rebuild the concrete assembly after
changing only `BasilicaPremises::dome_height`:

```sh
cargo run -p basilica_ruin -- --warm-reconfigure
```

This compiles the accepted default once, rebuilds the edited assembly through
the same `PartCompiler`, and verifies it against a fresh edited compile. The
summary reports cache hits per miss, the single changed named part, ordered OBJ
group count, and the byte-identical warm/fresh signature. It is a cache-work
proof rather than a wall-clock claim because the example separately evaluates
every recipe to retain its diagnostics until `ea-8tpb` lands. The edited
artifacts are written as `target/basilica_ruin/basilica_ruin_warm.obj` and
`target/basilica_ruin/basilica_ruin_warm.glb`.

For a visible parametric comparison, extend the rectangular basilica by two
exact meters while retaining the fixed west datum and crossing station:

```sh
cargo run -p basilica_ruin -- --parametric-length
```

This writes `basilica_ruin.glb` and `basilica_ruin_length_plus_2m.glb`. The
artifact test proves that the apse moves east by exactly two meters while the
west facade and crossing dome retain their world bounds; it also proves the two
GLBs differ.

## Code map

There is one normal path through the example:

```text
BasilicaPremises
    -> exact BasilicaSetout
        -> plan + levels + aisle + roof + crossing + east-end sections
        -> provenance + reconstruction assessment + dirty frontier
    -> build_basilica_assembly
        -> architecture::{nave, aisles, interior_arcades, east_end, crossing,
                         crossing_transition, buttresses, nave_trusses}
            -> geometry recipes and profiles
                -> editable Assembly
                    -> optional name/role-based edits
                        -> compile, flatten, export
```

`basilica_setout` owns exact premises, the immutable network, resolved section
DTOs, reconstruction evidence, and quantity-to-element invalidation.
The crate root owns the public entry points, name vocabulary, and query helpers.
`architecture` owns architectural meaning and deterministic insertion order;
each submodule adds one building system through the narrow example-local build
context. `geometry` owns only low-level constructive recipes, profiles, and
placement frames. `output` compiles the finished assembly and owns CLI/OBJ
plumbing. This keeps future details as focused architectural modules rather
than new capabilities on a general-purpose building DSL.

The buttress elevations use one exact `setout_generate` fragment. Their setout
network derives west and east anchors from explicit inset roots, generation
assigns stable `start`, `end`, and interior-rank identities, and a small
architecture adapter lowers each rational coordinate once. Invalidation reads
the same fragment rather than recreating its count-to-name mapping. Buttress
paths therefore use names such as `buttress-north-start`,
`buttress-north-interior-000001`, and `buttress-north-end`; an arcade-count edit
cannot rebind the old endpoint path to a new interior station.

Nave-truss stations still use a `cambium::assembly` named linear pattern. Their
authored ordinals remain the exported identities: the missing west truss at
ordinal `02` stays a named gap while later stations retain their existing paths.
That distinct topology has explicit omissions and multiple generated members;
it is intentionally left for a later `setout_generate` slice.

Each intact nave station is a complete `joiner_timber` braced king-post truss,
composed once and then repeated as immutable assembly parts. Housed heels,
strut bearings, paired rafter-head bearings, and the keyed king-post-to-tie
connection are real recipe edits; the transverse timber key is a separate
named instance below the tie. North rafters and braces are the canonical
recipes; their south counterparts use winding-correct `Recipe::mirrored`
composition. Both remain distinct named parts under proper-rigid assembly
placements, so no exporter-specific winding repair or mesh baking is needed.

## Name-addressable assembly API

The crate is both a library and an executable. Build the uncompiled assembly,
query it through stable names, apply edits, and only then compile it:

```rust
use basilica_ruin::{
    BasilicaPremises, build_basilica_assembly, instances_with_role, names,
    resolve_instance_path,
};

let mut basilica = build_basilica_assembly(&BasilicaPremises::default());

let dome_part = basilica
    .part_by_key(names::parts::CROSSING_DOME)
    .expect("the stable dome part exists");
basilica
    .set_part_material(dome_part, "surface", "restored-lead")
    .expect("the dome declares its surface slot");

let apse = resolve_instance_path(&basilica, names::instances::EAST_APSE)
    .expect("the stable apse path exists");
assert_eq!(basilica.path_of(apse).unwrap().to_string(), "east-apse");

let buttresses = instances_with_role(&basilica, names::roles::AISLE_BUTTRESS);
assert_eq!(buttresses.len(), 16);
```

`names::parts` identifies shared geometry definitions; `names::instances`
identifies placed building elements; and `names::roles` selects semantic
families in deterministic assembly order. The grouped OBJ preserves instance
paths as group names. glTF nodes preserve `name`, `extras.instancePath`, and
`extras.partKey`, so the same vocabulary survives export.

The nave clerestories are deliberately split into west and east named
segments. Their bounds stop at the crossing stage rather than continuing
behind its arches, so `nave-wall-north-west` and `nave-wall-north-east` expose
a genuinely open crossing bay and remain independently addressable.

## Workflow mapping

The old worked scenario described basilica-specific Cambium operators that do
not yet exist as a serialized program. This example keeps that architectural
meaning local while exercising the mechanisms now available:

| Worked scenario step | Executable realization |
| --- | --- |
| `shape.floor_plan.basilica` | exact `BasilicaPremises` propagated into resolved plan and level sections |
| `shape.extrude.walls` | public profile extrusion recipes |
| `detail.openings.arcade` | clockwise round-headed profile holes; true openings |
| nave/aisle spatial hierarchy | four named lower interior arcade segments carrying the raised clerestory |
| `shape.add.drum` | twelve tangent wall panels, six genuine window openings, and two cornice rings |
| square-to-drum transition | four named faceted ruled-loft pendentive webs beneath an open twelve-sided bearing throat |
| `shape.add.dome` | fixed-correspondence 24-sided ruled loft with capped crown |
| region/provenance | source references, profile segment regions, assembly paths |
| ruin damage | one authored notch in the south clerestory wall |
| render extraction | assembly compilation and deterministic render-list flattening |
| export | example-local grouped OBJ plus `exedra_gltf` |

## Deliberate compromises

- The example composes immutable recipes directly; it is not yet a JSON
  `Cambium Program` with per-step `OpReport` and `ChangeSet` values.
- Major masses stay as named assembly parts. They may overlap where real
  masonry would be joined, preserving semantic identity and avoiding a
  brittle building-wide Boolean union.
- Arcades are profile holes rather than repeated Boolean cutters. The voids
  are genuine and watertight, while one profile tessellation handles the
  repeated curve topology deterministically.
- The aisle roofs are explicit lean-to sections: their inner edges meet the
  nave in the solid spandrel band between interior arcade crowns and
  clerestory sills, while their eaves bear on the outer arcade walls. Their
  paired proper-rigid frames use opposite baseline directions so both roof
  thicknesses point outward/up. They are separate named masses rather than a
  single joined shell.
- The four nave-to-aisle arcade segments use genuine round-headed profile
  voids with a minimal two-centimetre profile boundary at floor level. This
  keeps tessellation watertight while reading as spatial passage, not a window
  or decorative column row in front of a solid wall.
- The crossing uses four ground-bearing piers, pierced upper stage faces, four
  named pendentive webs, a thin open square-to-dodecagonal bearing ring, and a
  windowed polygonal drum. Exedra does not yet provide trimmed spherical
  triangular surfaces, so the pendentives are explicitly faceted three-section
  ruled-loft solids rather than claimed spherical masonry. Their lower sections
  stay over the pier and arch shoulders while their upper chords overlap the
  drum footprint. These overlapping named masses make the load path visible
  without pretending to solve masonry statics.
- The chancel gable closes the nave roof end around one large round-headed
  opening into the apse. C-shaped faceted wall and roof-shell profiles leave
  the sanctuary and its conch hollow, while the main crossing dome carries the
  larger curved silhouette.
- Assembly compilation currently discards constructive evaluation reports.
  This executable audits each recipe directly before compilation so its
  diagnostic counter remains honest; the assembly fix is tracked by
  `ea-8tpb`.
- Ruinization is a single authored upper-wall break. Seeded damage, chipping,
  vines, UV authoring, and incremental edit playback remain future workflow
  layers.

## License

Apache-2.0 OR MIT
