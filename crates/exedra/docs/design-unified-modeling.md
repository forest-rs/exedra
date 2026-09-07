# Design pass: a unified modeling document, and where materials live

Status: design pass, 2026-09-06. Not a decision. It records what the
composition probes (`examples/constructive_probe`) taught, what a
Houdini- or Rhino-like unification would mean for Exedra specifically,
and a staged proposal with the material system as the first stage.

## 1. What the probes taught

Twenty-one composed models were built through the public constructive
API, checked for every contract the evaluator promises, exported to GLB,
and rendered headlessly. The results sort into four kinds, and the kind
matters more than the count.

**Wrong results.** None were found in the probes themselves, after the
seven review fixes. The one wrong-result class found on the way there
(an `Instance` under a `Mirror` refused instead of mirrored) is fixed on
the `review-hardening` branch.

**Typed refusals the user did not cause.** Equal-radius perpendicular
cylinders refuse with dangling cuts because one cylinder's edges lie
exactly on the other's surface; unequal radii or a five-degree skew
succeed. Three-way cylinder intersections refuse for the same reason.
These are the kernel's degeneracy gaps, and they are common shapes in
generated content: pipe crossings, equal-bore holes, symmetric joints.

**Hard errors for a reasonable request.** A loft between two circles of
different radius fails the *whole evaluation* with `SectionMismatch`
because the two circles discretize to different point counts. The user
asked for a cone frustum. This should be a typed refusal at worst and a
resampled loft at best.

**Convention traps.** `Revolve` spins about local Y while `Cylinder`
stands on Z, and the IR doc said both "XZ half-plane" and "Y axis" in one
paragraph (fixed). Nothing was wrong, but the probe author (me) built the
cutter in the wrong place, which is exactly what a generator will do.

Two observations that are not bugs but shape the design below: an
evaluation that refuses is retried on every warm run because refusals
are not cached; and a definition used both directly and as an instance
source is tessellated twice within one uncached evaluation.

## 2. What Houdini and Rhino get right, precisely

The two systems are usually cited loosely. The transferable ideas are
narrow and worth stating exactly.

**Houdini.** Geometry is a stream carrying *attributes* at four levels
(detail, primitive, point, vertex). Every operation is a node with typed
inputs; the network is the document. Instancing is a primitive kind
(packed primitives) that shares geometry until something needs to unpack
it. A material is assigned by a primitive-level string attribute that
names a shader; assignment and definition are different things, and the
assignment travels with the geometry through every operation. Domain
crossings (polygons to volumes, curves to polygons) are explicit nodes.

**Rhino.** The document is a table of objects. Each object has
*attributes* separate from its geometry: layer, name, user text, and a
material *source* that is one of by-layer, by-parent, or by-object. Render
materials are document resources with their own table; an object stores a
reference, not a material. Geometry types coexist (B-rep, mesh, curve,
extrusion, SubD) with explicit conversion commands. Texture mapping is an
object attribute too (box, planar, surface parameterization), with
real-world scale as a first-class notion.

The common lesson is not "one geometry type". It is:

1. Attributes are the universal carrier; geometry types are not.
2. Material *assignment* is an attribute on geometry or structure;
   material *definition* is a resource. They have different lifecycles.
3. Assignment inherits down structure and is overridable at any level.
4. Instances are native and cheap, and they carry their own overrides.
5. Every operation reports, and crossings are visible.

Exedra already agrees with 1, 4, and 5 in principle (ADR-0002 and
ADR-0005 in `exedra_ops`). It half-agrees with 2 and 3: `exedra_assembly`
has slots, region-to-slot maps, part defaults, and instance bindings,
which is a two-level inheritance with opaque keys. It has no definition
side at all beyond the glTF stub that hashes a key into a color.

## 3. Where a document would sit

Exedra's layers today, bottom up: `exedra_mesh` (polygon kernel and
attributes), the heads (`constructive`, `analytic`, `isosurface`),
`exedra_ops` (typed operations and crossings), `exedra_assembly` (parts,
instances, materials as keys, flatten), `exedra_gltf` (export), and the
domain layers `setout` and `joiner` above. ADR-0005 already reserves the
place for a procedural network above `exedra_assembly`, built on
`execution_graph` and `understory_node_graph`.

A unified *document* is the thing that network edits. It is not a new
geometry type. It is:

- **Objects**: a stable key, one native value (a `Recipe`, `Mesh`,
  `AnalyticShell`, field, or an instance of another object), a placement,
  an attribute bag, and a material assignment.
- **Resources**: material definitions, evaluation policies, and source
  references, each keyed and content-fingerprinted.
- **The dependency graph** between objects and resources, which is what
  makes an edit incremental.

`exedra_assembly` then becomes the realization of a document for mesh
consumers: the flatten seam stays, and `PartCompiler` stays the cache.
Nothing below assembly needs to know the document exists, which is the
tenet-compatible way to grow upward.

This does not need to be built all at once, and should not be. Stage 1 is
materials, because they are needed soon and because their contract
touches every layer just enough to prove the shape.

## 4. Materials

### 4.1 Two concepts, two lifecycles

- A **material slot** is an assignment: an opaque key on geometry or
  structure that says which material a face wears. It lives with
  geometry identity and is content-fingerprinted with it.
- A **material description** is a definition: a keyed resource that says
  what the material looks like (PBR factors, textures, extensions). It
  never participates in geometry fingerprints. Rebinding or editing a
  description must never re-tessellate anything, which the assembly ADR
  already guarantees for keys.

### 4.2 The contract

> Every emitted face resolves to exactly one material slot, or to an
> explicit `Unassigned`. Resolution is a pure function of the face's slot
> attribute, the part's region map and default, and the instance's
> bindings, in that order of override, and it holds through every
> geometry operation: Boolean, stretch, mirror, instance, import.

The last clause is the one that matters and the one that composition
breaks. Today the slot is derived from `FACE_REGION` through the part's
region map, and `FACE_REGION` is carried by tessellation, imports, and
Booleans. That is nearly enough, with two gaps:

**Boolean cut faces.** In `Difference(timber, mortise_cutter)` the
mortise walls come from the cutter's surface. Provenance says operand 1,
and the cutter's region map says nothing useful about timber. The
resolution rule needs an explicit `CutFacePolicy`: `Minuend` (cut faces
wear the material of the body that was cut, the joinery default) or
`Contributor` (cut faces keep the contributing operand's material, the
right default for a union of two differently finished parts). This is a
recipe-level policy on the CSG node, not a global.

**Faces without a region story.** Stretch bands, refinement-generated
faces, and grid side walls all have region values, but the mapping from
region to slot is per part and assumes the region namespace of one
producer. A CSG of an extrusion and a primitive mixes two namespaces
under one part. The fix is to resolve slots *before* the Boolean, per
operand, and carry the resolved slot as a face attribute (`FACE_SLOT`,
sparse, `u32` into the part's slot table) through the pipeline the same
way `FACE_REGION` is carried. Regions stay for fine provenance; slots
become the material carrier. Then the cut-face policy is a choice between
two already-resolved slots.

### 4.3 Descriptions and export

Add a small `exedra_materials` crate (or module under assembly, but a
crate keeps the dependency surface honest): a `MaterialDescription`
aligned with glTF metallic-roughness (base color, metallic, roughness,
emissive, normal and occlusion texture references, alpha mode, double
sided) plus an opaque extension bag, keyed by the same strings assembly
binds. `exedra_gltf` maps descriptions to real materials and keeps the
hashed stub only for keys with no description. Renderers consume the same
table. The description schema is versioned and serializable behind the
existing `serde` policy.

### 4.4 UVs are the real prerequisite

Tessellated constructive bodies emit no `CORNER_UV` today; only imports
and the stretch path carry UVs, and the glTF exporter writes zeros for
the rest. A material system without texture coordinates is a color
system. The constructive head should emit a deterministic default
parameterization per feature, in model units so that texture scale is a
material property (Rhino's real-world scale): caps in profile space,
extrusion walls as (distance along the loop, height), revolve walls as
(arc length, distance along the profile), sweep walls as (path length,
profile length), primitives by their documented regions. Provenance
already names every one of these features, so the parameterization is a
pure function of what the source map records. Booleans keep each face's
UVs, since cut faces are pieces of operand faces.

### 4.5 Normals are a render policy, not a tessellation side effect

The probe cylinders render faceted while the revolved spheres render
smooth, because `sharp_sin_threshold` decides creases at tessellation.
A material system will make this visible immediately. The export should
take an explicit normal policy (authored overrides, then crease by
threshold, then smooth) that the document owns as a resource, so a
change of shading intent does not change geometry fingerprints.

## 5. Evaluation contract refinements

These fall out of the probes and should land before a material system
adds more surface.

- **Typed loft refusal.** `SectionMismatch` and `DegenerateLoft` become
  envelope-only outcomes with an `eval.loft.*` diagnostic, like CSG and
  stretch refusals. Better: sections resample to a common count under a
  declared policy, with the resampling recorded as `PolicyDefined`.
- **Refusal caching.** A refused CSG or stretch is retried on every warm
  evaluation. Cache the typed refusal under the same key so a warm run is
  cheap and the report still replays.
- **Axis conventions.** Either give `Revolve` an explicit axis parameter
  or document the Y/Z split on both nodes (done for now) and check it in
  the interchange docs. A generator will get this wrong.
- **Kernel degeneracies.** Equal-radius cylinder crossings (edges on
  surfaces) and `Robust` triangulation over chained axis-aligned cuts
  (Delaunay diagonals coincident with cutter planes) are the two known
  gaps. Both are edge-on configurations the split stage defers. They are
  the same kind of work and should be scheduled together.

## 6. Staged roadmap

1. **Materials contract** (assembly, constructive, gltf): `FACE_SLOT`
   carried through Booleans with `CutFacePolicy`; resolution as a pure
   function; tests that mirror the geometry contract tests (direct
   versus instanced, cold versus warm, n-ary versus chained) but assert
   per-face slot identity.
2. **UV parameterization** per feature in the constructive head, and
   `MaterialDescription` with real glTF materials.
3. **Evaluation refinements** from section 5, in whichever order the
   consumer hits them; typed loft refusal first.
4. **Document resources**: materials and policies as keyed,
   fingerprinted resources with their own lifecycle, above assembly.
5. **Network integration** per ADR-0005, once a consumer connects two
   native values through a real conversion edge.

Each stage keeps the acceptance criterion from the review: an evaluation
either produces the requested result within its declared policy, or
accurately explains what it could not produce, and caching, instancing,
grouping, and now material binding must not change that truth.
