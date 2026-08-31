# Basilica Structure Lab

`basilica_structure_lab` is a separate structural experiment for one intact
western bay of the basilica. It authors a deterministic graph first, proves
named contacts and gravity-transfer paths, and only then emits geometry. It
does not alter or replace the accepted `basilica_ruin` artifact.

The graph itself is no longer local. The element graph, the three relation
kinds, evidence labelling, layered validation, and lowering to an assembly all
live in the [`joiner`](../../crates/joiner/) construction crate; this lab
authors a hypothesis into it and draws the result.

```text
BasilicaPremises + evidence-labelled hypothesis  (this lab)
    -> joiner::Construction
    -> joiner::validate      schema/contact/load-path
    -> joiner::lower         one Assembly instance per geometry-bearing element
    -> grouped OBJ + semantic diagnostic layers  (this lab)
```

Run the complete first-slice export:

```sh
cargo run -p basilica_structure_lab
```

Artifacts are written beneath `target/basilica_structure_lab/`. Focused
inspection is also available:

```sh
cargo run -p basilica_structure_lab -- --check
cargo run -p basilica_structure_lab -- --explain element:roof-covering-south
cargo run -p basilica_structure_lab -- --explain bearing:bearing-principal-south-east-on-wall-plate
cargo run -p basilica_structure_lab -- --explain evidence:hagia-paraskevi-roof-survey
cargo run -p basilica_structure_lab -- --layer bearings
```

Supported layers are `full`, `structure`, `load-path`, `bearings`, and
`transparent-roof`. Each geometry-bearing graph element remains one stable OBJ
group. The `load-path` layer adds named transfer-edge groups. The `bearings`
layer adds named bearing and support markers; one selected principal-rafter
bearing also gets carried/carrier anchor and frame groups. Nodes and joints
are inspectable with `--explain` but are not emitted as OBJ diagnostics.

Explain selectors are typed: `element:`, `node:`, `member:`, `joint:`,
`bearing:`, `support:`, `transfer:`, or `evidence:`. They keep this lab's own
vocabulary: `joint:` finds the member/member relation that *is* the joint, and
`bearing:` finds the contact patch behind it. A bare key is accepted
only when it exists in exactly one category. For example, bare
`wall-plate-south` is rejected because its element and member deliberately
share that stable key.

The bounded Blender checkpoint renderer consumes those named OBJ layers:

```sh
/Applications/Blender.app/Contents/MacOS/Blender \
  --background \
  --python examples/basilica_structure_lab/tools/render_checkpoints.py \
  -- \
  --artifact-dir target/basilica_structure_lab \
  --render-dir target/basilica_structure_lab/renders
```

Its twelve views cover exposed framing, explicit transfer edges, a transparent
roof, both wall-plate bearings, isolated heel and keyed-tie fits,
assembled/exploded views of one complete braced king-post truss, and assembled
plus exploded inspections of the secondary purlin/common-rafter system.
These are semantic inspection images, not a photoreal acceptance replacement
for the basilica. The joint views use the real rule-edited members and
contrasting role materials; they do not exaggerate the production fit
allowance.

## What this proves

- Every roof and framing element has a deterministic named route to a ground
  support.
- Every transfer is witnessed according to its declared kind: a `Contact`
  transfer requires a matching load-carrying contact patch, a `Joint` transfer
  requires a member/member relation incident to members of both elements, and
  a `Ground` transfer must target the support belonging to its source element.
- Covering, boarding, common rafters, purlins, principal trusses, wall plates,
  masonry, and ground supports form one explicit chain.
- Analytic bearing anchors reject both a floating covering and an embedded
  covering. Bearing frames must be finite and orthonormal, anchors must remain
  in local solid bounds and coincide in the complete bearing frame, and
  overlap minima must be finite and nonnegative.
- Required direct-support counts detect a missing purlin even when another
  route to ground still exists.

Contact uses a documented tolerance of `1e-9 m`; signed gaps and penetrations
within that tolerance are treated as coincident. `--explain bearing:<key>`
reports both world-space anchors, the signed gap, both measured overlaps,
their minima, the frame, the tolerance, evidence class, and evidence-source
key.

The bright bearing cubes and frame axes are deliberately oversized,
nonphysical diagnostic markers. `joiner_timber` now fits every primary joint
in each braced king-post truss: housed heels, a keyed through-tenon suspending
the tie, both ends of both compression struts, and both principal-rafter
bearings in the king head. It also fits all three longitudinal purlin lines:
each passes through trenches in both principal rafters, and each common rafter
has a shallow underside seat at every crossing. Every receiving cut is derived
from one nominal interface, with the typed fit allowance applied only to
receiving geometry; the key slot keeps its two load-bearing faces line-to-line.
The upper strut bearing is a physical contact but not a directed transfer
because it closes the truss's internal force triangle, while `joiner`
intentionally validates an acyclic support explanation rather than solving
statics. The separate
principal-rafter/wall-plate `anchor-contact` and masonry relations remain
explicit hypotheses. Masonry knowledge remains stage 3 of `bsl-6ihj` (see ADR
0002).

This is schema, geometric-coherence, contact, transfer-witness, and load-path
validation. It is **not** a
static analysis, finite-element model, capacity check, building-code result,
or engineering certification. Joint stiffness, material grading, buckling,
wind, seismic loads, deterioration, and connection capacity remain outside
this slice.

## Historical basis and inference labels

The graph links every element, member, joint, and bearing to a named evidence
source and one of four evidence classes:
`Observed`, `DocumentedReconstruction`, `RegionalAnalogy`, or
`ModernEngineeringInference`. Source existence and class compatibility are
validated. The complete fitted truss is a modern engineering inference
informed by historic comparanda; it is not presented as a reconstruction of
one known Byzantine roof or as a capacity-checked design.

- The University of Michigan Kelsey Museum reports that the timber nave roof
  at Saint Catherine's, Sinai, was dated to 548-565 through inscriptions
  studied in the 1958 expedition. This is strong evidence for an extant
  sixth-century timber-roofed basilica, but the page does not establish every
  joint used here:
  <https://lsa.umich.edu/kelsey/research/past-field-projects/monastery-st-catharine.html>
- Robison's technical study of San Paolo fuori le Mura uses pre-1823 records
  of the lost Early Christian double trusses. It is documentary evidence for
  a long-span Roman truss form, not surviving joint fabric:
  <https://www.witpress.com/elibrary/wit-transactions-on-the-built-environment/191/37497>
- The multidisciplinary survey of Hagia Paraskevi at Chalkida documents
  mortise-and-tenon trusses, secondary timbers, boarding, and Byzantine tiles.
  Its surviving roof reflects later Venetian/medieval development and is used
  only as a regional construction analogy:
  <https://hdl.handle.net/11583/1956728>
- The Vernacular Architecture Group's glossary distinguishes through,
  trenched, tenoned, threaded, and other purlin forms, with an explicit
  trenched-purlin example. It supports the rule's joint vocabulary, not its
  attribution to this basilica:
  <https://www.vernacularbuildingglossary.org.uk/a-z/purlin/>
- The National Trust's Knole roof report records lower purlins trenched into
  common rafters while upper connections use a different form. It is the
  explicit analogy for the common-rafter seat and also the reason the code
  does not present that seat as a universal purlin joint:
  <https://heritagerecords.nationaltrust.org.uk/LibraryLinkWS/LLFiles/225169/original_225169.pdf>
- The UNESCO/ICOMOS Church of the Nativity mission explicitly records
  uncertainty over which present roof fabric is sixth-century and which
  belongs to later rebuilding. That uncertainty is the reason the lab keeps
  inference labels visible:
  <https://whc.unesco.org/document/185314>
- The Timber Frame Engineering Council design guide informs modern housed
  bearing semantics, heel-relish checks, struts bearing into the king above
  the tie, and the need for an explicit king-post tension connection. It is
  technical guidance, not Byzantine evidence:
  <https://timberframehq.com/wp-content/uploads/2021/12/TFEC-DG-1-2021.pdf>
- Historic England records later king-post roofs with raking struts and metal
  straps under the tie. That is a useful tension-mechanism comparison, not a
  claim that this basilica used the same detail:
  <https://historicengland.org.uk/images-books/publications/south-east-farmsteads-character-statement/se-farmsteads-guidance/>
- NPS documentation illustrates historic king-post-to-chord stirrup straps
  and forelock bolts. The lab deliberately uses an inspectable timber key
  instead of silently implying such unmodeled metalwork:
  <https://www.nps.gov/articles/000/timber-framed-steeples-reproducing-burned-or-destroyed-steeples.htm>
- NPS preservation research explains why hinges, continuity, partial
  continuity, supports, and sheathing stiffness must be modeled explicitly:
  <https://www.nps.gov/articles/000/timber-framed-steeples-engineering-a-steeple-restoration.htm>

The boundary and proof levels are recorded in
[`docs/adr-0001-structural-graph-and-proof-levels.md`](docs/adr-0001-structural-graph-and-proof-levels.md).
