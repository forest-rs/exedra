# joiner_masonry

Running-bond masonry with an optional circular opening and segmented surround.
The rule generates ordinary `joiner` elements with caller-owned materials,
stable course/unit keys and application provenance. Repeated units share their
recipes through `joiner::lower_shared`.

The crate owns coursing and coordinated unit geometry. Site layout, the
construction graph, meshing, rendering and structural capacity stay in their
existing layers. It is `no_std` with allocation and uses existing workspace
geometry and measurement dependencies.

## Usage

Create a present wall element with `without_part()`, declare an empty
`RelationKind::element_units` relation, then call:

```rust
construction.apply_rule("lay-wall", "wall-bond", &RunningBondRule, &params)?;
let assembly = joiner::lower_shared(&construction, |unit| unit.role.clone())?;
```

`RunningBondParams` supplies exact positive unit length, unit height, joint
width and minimum closure dimensions. Wall-local X runs along the courses,
Y spans the wall thickness and Z rises through courses. Odd courses start
half a module along the bond. Small rectangular edge remainders are
redistributed across two cut units, preserving the specified joint and never
exceeding the stock dimensions. The minimum must fit the unit height and
initial half unit; layouts without a legal closure are refused. Generated cells
retain their indices even when the opening removes intervening cells.

`CircularOpening` supplies its local X/Z center, clear radius, surround width
and segment count. Both circles and their outer joint must fit inside the
wall. Layouts are bounded to 100,000 estimated cells. The surround uses
repeated curved sectors with radial joints; boundary units are trimmed using
constructive differences. The wall may start below
finished ground to bury the opening's lower arc beneath a level approach.

The wall is a logical parent: giving it a part or expanding the same relation
a second time is refused, preventing duplicate wall-and-unit geometry. New
parameters are evaluated by rebuilding the authored construction; this slice
does not replace previously generated units in place. Other joiner rules and
lowering APIs retain their behavior.

This first bond uses one unit through the declared wall thickness. Mortar is
represented by open gaps. No contacts or load paths are fabricated across
those gaps, and no structural verification is claimed. Minimum closure checks
cover rectangular course ends; cuts around the circle can leave smaller
pieces. Through-thickness bonding, mortared interfaces and closure selection
around openings remain future work.

