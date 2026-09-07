# Exedra procedural definitions, modeling documents, and materials

## Consolidated design and implementation handoff

**Date:** 2026-09-06  
**Status:** Consolidated proposal for review and implementation planning; not an accepted ADR.  
**Recommended procedural crate name:** `exedra_procedural` (recommendation, not a confirmed naming decision or availability check).  
**Suggested repository destination:** `docs/design/procedural-document-materials.md` (new path proposed here).  
**Audience:** An implementing agent with the Exedra checkout and access to the neighboring Forest execution, addressing, and editor crates.

### Purpose and authority

This document combines the preceding naming/language discussion, the supplied **“Design pass: a unified modeling document, and where materials live”**, and the existing Exedra architectural boundaries. It identifies disagreements rather than silently treating the proposals as mutually compatible.

Repository inspection was read-only. The inspected remote `main` was commit `42ec07adf88acb24f49d4a6817ab7a747f526bc8`. No probes, tests, benchmarks, or local branches were executed during this synthesis. The supplied design pass reports results on a `review-hardening` branch; those results are reported evidence, not independently verified results from the inspected `main`. That branch was not present in the remote branch listing retrieved for this review; it may be local or otherwise unpublished. Do not infer that its changes are absent from the implementing agent's checkout.

**Reading convention:** statements labeled **Baseline** describe inspected repository contracts; **Reported** means the supplied design pass; **Proposal** means a recommendation introduced or reconciled here. Unless explicitly identified as a baseline or report, requirements below are proposed requirements. Existing accepted ADRs remain authoritative until deliberately amended.

Source identifiers such as `[S1, §4.2]` resolve in the source register at the end. Proposed API names are illustrative unless identified as existing. Do not generate a new public framework from every conceptual noun in this document.

---

## 1. The recommendation

Build a **typed procedural geometry language** whose authored programs are durable **definitions**, and place those definitions within a **modeling document** that also retains objects, resources, bindings, and publication intent.

These are related but different artifacts:

| Concept | Meaning | What it must not become |
|---|---|---|
| Procedural `Definition` | Authored operations, typed connections, parameters, named outputs, and references to reusable definitions/resources. | A serialized runtime schedule or a node-canvas snapshot with accidental execution semantics. |
| Compiled program | Resolved, validated execution representation derived from a definition. | The only surviving record of author intent. |
| Modeling document | Definitions plus stable object records, resources, placements, bindings, and explicit output references. | A universal geometry representation, a new application kernel, or a duplicate assembly implementation. |
| Evaluation | Revision-pinned native artifacts, outcomes, provenance, and measured execution results. | A hidden mutation of the authored document. |
| Editor projection | Interactive presentation of authored content, including node positions and routing. | The authority for geometry meaning or durable runtime identity. |

**Keep `exedra_procedural` as the name for the language/integration layer.** The new material/document proposal strengthens the case for distinguishing program semantics from the larger document; it does not make `exedra_graph` or `exedra_grammar` a better name. `exedra_program` remains a reasonable alternative if naming around the executable artifact is preferred. Do not create both.

Use a `document` module or a consumer-owned composition layer initially. A separate `exedra_document` crate is an option only when an independent consumer or dependency boundary earns it. A focused `exedra_materials` crate is justified when shared appearance descriptions have actual exporter/renderer consumers; it must not depend upward on the procedural document. Crate publication and availability were not checked.

### 1.1 What changes from the earlier answer

The earlier answer put executable definitions first and left resource lifecycles underdeveloped. The supplied pass adds a useful concrete consumer: material assignment, material resources, mapping, shading policy, and their behavior under composition. Retain that contribution. [S1, §§3–4]

The synthesis makes five further distinctions:

1. The modeling document is broader than its procedural definitions; graph evaluation should produce artifacts and published object results, not imperatively edit arbitrary document state.
2. Material handling has **three** stages: face-to-slot assignment, slot-to-material binding, and material-resource definition. The latter two must remain outside shape computation unless an operation explicitly reads them as geometric inputs.
3. A proposed `FACE_SLOT` carrier needs palette ownership/remapping and a late binding contract. Copying raw local indices between operands is not sufficient.
4. Cut-face geometric provenance, material selection, and texture mapping are distinct policies. One cannot substitute for the others.
5. Shading/mapping changes may leave geometric topology unchanged while changing attributed artifacts, render vertices, batching, or export bytes. Cache contracts must name the particular artifact they protect.

### 1.2 Non-goals for the first implementation

Do not build a full general-purpose language, arbitrary cyclic evaluator, shader-graph language, universal attribute/value enum, general shape-grammar interpreter, CAD-grade B-rep kernel, complete scene-composition system, or alternate scheduler. Do not make `exedra_assembly` depend on the document or every native geometry domain. Do not turn the materials work into a physical-materials database for mechanics or joinery.

---

## 2. Preserve the existing architecture

**Baseline.** Exedra already separates native geometry heads, mesh operations, assembly structure, execution infrastructure, and editor infrastructure. `exedra_ops::OperatorRunner` is mesh-specific; a future heterogeneous network must not pretend that the unary mesh-edit interface is its universal node contract. The accepted ADR requires typed ports, explicit conversions, immutable cached graph-boundary artifacts, and a native-executor seam in the shared execution stack. [S2]

| Owner | Retained responsibility | Integration obligation |
|---|---|---|
| `exedra_mesh` | Polygon topology, typed domain attributes, edits, validation, extraction. | Carry relevant attributes and lineage according to explicit operation contracts; no knowledge of material-resource tables or documents. |
| `exedra_constructive` | Immutable construction intent, fingerprints, evaluation, source maps, fidelity and refusal reporting. | Own feature parameterization and recipe-level conversion/Boolean attribution policies where these belong. |
| `exedra_analytic`, `exedra_isosurface` | Their native values, algorithms, and bounded guarantees. | Expose explicit native-to-other-domain crossings without pretending all domains are equivalent. |
| `exedra_ops` | Mesh operation lifecycle and focused native-domain adapters. | Bind existing operations into typed procedural nodes; do not move domain algorithms into the graph crate. |
| `exedra_assembly` | Parts, instances, placements, binding, compilation/sharing, and `RenderList`. | Retain the independent native structure contract; accept explicit integration results from above. |
| `exedra_materials`, proposed | Portable appearance descriptions and texture/resource references. | Supply data to renderers/exporters without owning object hierarchy, geometry evaluation, or resource I/O. |
| `exedra_procedural`, proposed | Geometry-specific definitions, node schemas, typed native-artifact references, compilation adapters, outcomes. | Connect native operators to the shared execution and editing layers. |
| Document composition layer, proposed | Authored objects, resources, definition invocations, bindings, revision-pinned publication. | Reuse assembly and shared infrastructure; avoid a second store of independently editable geometric truth. |
| `execution_graph` | Dependency capture, invalidation, targeted execution, execution causality. | Verify/add the native-node executor seam required by ADR-0005. |
| `execution_tape` | Suitable scalar expressions, portable programs, explicit control flow. | Remain an execution backend, not the definition of a geometry node. |
| `understory_node_graph` | Live authoring machinery, projections, sessions, routing, hit testing. | Project Exedra meaning without owning it or conflating handles with persistent keys. |
| `exedra_gltf` | Explicit rendering/export adaptation. | Resolve supported descriptions and mapping/shading artifacts; expose unsupported capabilities and fallbacks. |
| `setout`, `setout_generate`, `joiner` and adapters | Domain-specific relations, labeled generation, construction/joinery semantics. | Consume/produce definitions or native artifacts without leaking scenario nouns into foundational crates. |

The table expresses ownership, not a mandatory linear dependency chain. In particular, operations, native assembly values, resources, and exporters are not all stages through which every program must pass.

**Reconcile the supplied document proposal carefully.** “Assembly becomes the realization of a document” is a useful consumer path, but too strong as an ownership statement. Assembly is already an independently useful structural value, admitting recipe and baked-mesh parts. Keep that capability. A document-to-assembly adapter may realize supported objects for mesh consumers; the document itself remains richer than that realization. A third retained native part kind still needs the consumer-driven integration boundary described in ADR-0005. [S1, §3; S2; S3]

---

## 3. Language, grammar, notation, and executable document

### 3.1 Computational semantics come before syntax

The initial procedural core should be typed acyclic dataflow. Define operation identity, parameter semantics, port types, conversions, outcomes, and resource dependencies before inventing a textual syntax.

A node canvas, a Rust builder, a serialized structured form, and agent-issued semantic edits can all author the same definition. They must not have subtly different connection or conversion rules. A shared definition is the semantic authority; the compiled runtime graph is a derivative.

Native values retain their own representation. A public port may carry a typed handle to a `Recipe`, `Mesh`, assembly, field, selection, quantity, or another registered native artifact. Private type erasure or static composition is acceptable. A public, centrally growing `GeometryValue`/`Value` enum is not the extension mechanism. [S2]

A node definition should be the source of its stable operation identifier, version, parameter schema, typed port schema, runtime binding, and editor-facing metadata. Start with static registration where sufficient; do not infer a requirement for a dynamic plugin ABI.

### 3.2 Collections are language semantics

Expose matching behavior rather than hiding it in wires. The first useful vocabulary is `map`, `zip`, `broadcast`, Cartesian product, and explicit grouping/flattening as consumers demand them.

A component receiving ten profiles and ten heights must say whether it produces ten paired extrusions or one hundred combinations. Unequal lengths need a declared policy: refusal, truncation, padding, or repetition must never be an accidental implementation detail. Initial `zip` should reject unequal lengths unless another policy is explicitly authored.

Collection position is not persistent identity. Named generated parts retain semantic keys through insertion, omission, and reordering. Reuse the stable-key lessons of `setout_generate`; do not replace them with “whatever was element 7 this time.” [S7]

### 3.3 Other computational models stay explicit

Reusable definitions with named inputs/outputs and mapping should precede general loops. Introduce bounded iteration or feedback regions only for a real workload, with termination/budget, dependency, state, and provenance semantics. Reject accidental connection cycles in the initial graph.

A constraint or quantity-propagation subsystem should appear as an explicit operation or region with named inputs, outputs, solve policy, and reports. Its internal relations are not ordinary one-way dataflow edges. Do not force the surrounding scheduler to become its solver.

A shape grammar can produce structure, invoke reusable definitions, or expand into executable operations. Preserve the authored rules and generated identity; the expansion is not the only durable artifact. Rule selection, recursive expansion, stochastic choices, termination, and identity require an explicit owner. Calling the grammar a frontend does not remove those responsibilities.

Implement grammar machinery only after a consumer needs it. The first network and material work do not require one.

### 3.4 Expressions, axes, units, and conversions

Scalar expressions may use the tape backend. Heavy geometry operations use native executors with the same dependency-access obligations. Do not wrap every native operation in a ceremonial one-host-call tape as the permanent interface. [S2]

Parameter schemas should carry quantity/unit and coordinate-frame meaning where required. Use existing measurement types instead of introducing another units package. Document local axis, handedness, placement composition, angle units, and tolerance policy. Preserve the current `Revolve` convention until a deliberate versioned change; do not silently reinterpret saved recipes to resemble another primitive. [S1, §§1, 5]

Converting a recipe to mesh, extracting a field, realizing instances, or changing approximation policy must be explicit in the authored meaning. The editor may assist by inserting a conversion, but may not hide it in an unversioned preference.

---

## 4. What the modeling document contains

### 4.1 Retained content

A minimal modeling document can contain:

| Retained element | Required meaning |
|---|---|
| Definition records | Stable keys, parameters, operations, typed connections, named outputs, versions. |
| Object records | Stable object keys, a source/output reference, placement, typed object attributes, binding environment, optional structure references. |
| Resource records | Keyed material descriptions, policies, source assets, texture/image references, and their revisions/content identities. |
| Invocation records | A definition plus argument/resource bindings and stable identity for the invocation. |
| Publication records | Which outputs are exposed as objects, assemblies, previews, or explicitly requested exports. |
| Authoring metadata | Labels, comments, node layout, grouping, and other presentation state, separated from computational identity. |

The supplied pass's attribute-carrier idea is useful, but an untyped attribute bag must not become the semantic foundation. Object/instance attributes, native face/corner attributes, resource metadata, and execution diagnostics have different domains and lifecycles. Give semantic attributes declared types, ownership, dependency behavior, and transfer rules at operations/conversions. Opaque optional metadata may be preserved without interpretation; a kernel or scheduler must not secretly depend on it. [S1, §§2–3; S2; synthesis]

An object's source can be an authored asset/native value reference or a named output of a definition invocation. This is a structural choice, not a requirement to store every native domain in one public union. Native artifact stores or adapters retain their types. An object can retain its source recipe while a mesh derived from it is cached for a consumer.

Do not duplicate assembly placements, instance trees, and bindings as a second independently mutable representation. Choose which document records adapt into an existing assembly and make the mapping explicit. Use a snapshot/view when presenting assembly-owned data in a document.

### 4.2 Separate authoring edits from evaluation

Authoring edits are revisioned changes to definitions, objects, or resources. Evaluation reads a consistent snapshot and produces new immutable native artifacts and outcomes. Publication associates results with that snapshot/revision.

Do not make an ordinary procedural node “mutate object X in the live document.” That introduces evaluation-order-dependent state and obscures undo and dependency capture. A node may instead produce an object description, assembly, or explicit proposed edit artifact whose application is a separate host operation.

A direct modeling edit can operate on an explicitly editable native asset or be retained as a procedural operation over an upstream result. In either case, it must not mutate a value shared by two cached branches. Use unique ownership or copy-on-write behind the native edit adapter. [S2]

For a long-running evaluation, publish only against the revision it evaluated. An older result may be retained for inspection, but must not overwrite a newer result as though it were current. A last-good preview must be visibly labeled stale when the current evaluation refuses.

### 4.3 Dependency ownership

Keep program connections, object/source references, resource references, and execution dependencies conceptually distinct. Derive and capture execution dependencies through the existing shared runtime. Do not require the user or editor to keep a second mutable dependency graph in sync with the first.

Resource references used by a node must be read through the dependency contract, not looked up in an ambient map invisible to the scheduler. A material-binding node and a geometry node can read different resources and invalidate independently.

The host resolves asset bytes, permissions, files, and network access. Core evaluation receives pinned inputs or an explicit resolver contract with recorded revisions. A filename or URI alone is not a content fingerprint. No implicit filesystem/network authority is granted by loading a document.

Exports and other side effects require an explicit host request or effect boundary. Preview invalidation must not write files or send network requests.

### 4.4 Identity and persistent addressing

Keep persistent authored node/object/resource keys, native element identities, editor-local handles, runtime-local node IDs, and content fingerprints separate. An ID answers “which thing?”; a content fingerprint answers “which content?”; a revision answers “which state?” None is a replacement for all the others.

Expose typed keys at APIs even when a canonical string representation is used for interchange. Reuse appropriate existing structured-addressing infrastructure after inspecting its contracts; do not add a competing general path/query system to this crate.

Generated identity should incorporate stable invocation/rule/element identity rather than positional indexing. Preserve explicit omissions and surface orphaned references. A persistent selection should name a source feature/region or another supported semantic target with revision/correspondence information; do not serialize raw transient face handles as durable intent.

Missing and ambiguous targets must remain diagnosable. No silent retargeting to “the nearest face” without an explicit, recorded policy. [S2; S7]

### 4.5 Persistence and capability handling

Save operation/type/schema versions, required capabilities, resource references, authored conversion policies, seeds where used, and source references. Distinguish semantic content from layout so moving a node does not invalidate geometry.

Preserve unknown operation payloads and connections where safe so a document can round-trip without executing them. Required unknown operations/types block affected evaluation with a structured diagnostic. Optional opaque metadata can survive without acquiring executable meaning.

Persist definitions, not compiled schedules. Compiled artifacts may be stored as validated caches with full compatibility keys; they do not substitute for source. Reuse the repository's existing `serde`, `std`, `no_std` and `alloc` feature policy rather than imposing a different policy on native heads. [S2; S3]

---

## 5. Materials: three stages and separate identities

### 5.1 Terminology

The supplied pass correctly separates assignment from material definition. The implementation should refine that into three stages. [S1, §4.1]

| Stage | Question | Identity and owner |
|---|---|---|
| Face-to-slot assignment | Which semantic material role does this surface use? | A slot in a declared owner/palette namespace; carried by geometry/attribution. |
| Slot-to-material binding | Which appearance is used for that role in this placement/context? | A binding from semantic slot identity to a material-resource key; owned by structure/document context. |
| Material definition | What is the referenced appearance? | A versioned material resource with factors, textures, and supported extensions. |

Do not call a local slot index a globally meaningful material ID. A slot such as `cut_surface` is a role; a resource such as `oak_oiled` is an appearance; an instance can bind the same role to a different appearance without changing the shared shape.

Do not weaken the existing guarantee that rebinding a material is a structure-level edit and does not retessellate part geometry. Current assembly also keeps binding and metadata edits out of its part-compilation invalidation channel. [S3; S4]

### 5.2 Two pure resolution functions, not one ambiguous override chain

First determine a face's semantic slot:

```text
assign_slot(face, producer/operand context, assignment policy)
    -> AssignedSlot(owner, stable_slot_key) | Unassigned | InvalidAssignment
```

Then resolve the slot in an object/instance binding context:

```text
bind_slot(assigned_slot, binding_environment)
    -> MaterialResourceKey | Unassigned | InvalidBinding
```

Finally resolve the resource key to a versioned description or an explicit missing-resource outcome. `Unassigned` is not the same state as “bound to a resource that cannot be found.”

For new binding layers, distinguish `Inherit`, `Use(key)`, and `Clear`; an absent override is not an explicit unassignment. This is a small domain-specific state, not a universal value model.

**Baseline:** the inspected assembly resolution is instance binding, then part default. Do not describe it as existing arbitrary ancestor inheritance. [S3; S5]

**Proposed extended precedence:** nearest explicit occurrence override, then applicable parent/structure overrides, then object/part defaults, then an explicitly configured document fallback, then unassigned. `Clear` stops inheritance. Add this only as a versioned, tested extension; old records without new layers retain their existing behavior.

Hierarchy does not make slot index 0 in one part equivalent to slot index 0 in another. Parent overrides must target a semantic slot key/interface or an explicit remapping. The first slice may keep current two-level binding while defining the extension seam.

### 5.3 `FACE_SLOT` is promising, but needs an ownership contract

The supplied pass identifies mixed `FACE_REGION` producer namespaces as a material-assignment problem and proposes resolving each operand's regions to a categorical face-slot attribute before Boolean composition. Retain that direction. Keep provenance regions; do not repurpose them as material identity. [S1, §4.2]

A valid implementation must specify:

| Concern | Required behavior |
|---|---|
| Palette ownership | A local `u32` is meaningful only with its palette/owner. A mesh/artifact carrying it retains or resolves that context. |
| Operand composition | Remap operand-local slots into a deterministic output palette. Never concatenate faces and copy indices unchanged. |
| Equal labels | Equal text in unrelated producer namespaces is not automatically equal identity. Merge only through an explicit semantic mapping. |
| Persistent identity | Interchange stores stable slot meaning; private dense palette indices can be rebuilt. |
| Unassigned | Define whether sparse absence encodes explicit unassignment or unresolved fallback. Do not conflate the two across stages. |
| Invalid indices | Validate at import/conversion boundaries; report out-of-range or orphaned palette entries. |
| Attribute propagation | Face slots are categorical: copy/select/remap them; never interpolate them numerically. |
| Sharing | Carry semantic slots before Boolean evaluation; bind actual materials afterward so instances can share geometry. |

A part-local palette is sufficient for a contained first implementation. A procedural artifact that can outlive part registration needs its own valid palette context or an explicit adapter that establishes one. Do not add an assembly dependency to the polygon kernel to obtain it.

### 5.4 The flatten/export seam must change with the carrier

**Baseline:** `CompiledBody` groups triangle indices by `FACE_REGION`; `flatten` creates `ResolvedRegion` records and resolves material through the part's region-to-slot map. Merely adding `FACE_SLOT` to mesh storage would not make render/export binding correct. [S4; S5]

Update the downstream partition contract so different slots within a region cannot be collapsed into one material assignment. Use ranges or equivalent metadata retaining both the applicable provenance identity and slot identity. Final render batching can group by resolved materials, but inspection must still recover source/slot meaning.

Define deterministic ordering, triangle coverage, and migration for existing region-only bodies. Legacy bodies may use the existing region map/default path as an explicit adapter. Preserve that compatibility until a deliberate format/API change removes it.

---

## 6. Boolean cut surfaces and new faces

### 6.1 Preserve three independent answers

For a surface created by `Difference(timber, cutter)`, the geometric boundary can come from the cutter while its appearance belongs to the timber. The texture mapping appropriate to timber can differ from the cutter's mapping. These are distinct facts. [S1, §4.2; synthesis]

Keep:

- **Geometric provenance:** contributing operand/source face and operation lineage.
- **Material attribution:** selected semantic slot under the authored Boolean policy.
- **Mapping attribution:** retained or generated coordinates under an explicit mapping policy.

Do not rewrite geometric provenance to make a material choice look simpler.

### 6.2 Refine `CutFacePolicy`

The supplied `Minuend` / `Contributor` proposal is directionally right but underspecified for a multi-material minuend. “Use the material of the body being cut” has no unique answer when that body has several slots. [S1, §4.2]

A small first contract should cover these meanings, using existing naming conventions where possible:

| Policy meaning | Required behavior |
|---|---|
| Contributor | Preserve the contributing surface's semantic slot after palette remapping. |
| Explicit cut slot | Assign a specifically named output slot to generated cut surfaces. |
| Minuend default | Use an explicitly designated cut/default slot of the minuend, or its uniquely applicable single slot. Refuse/diagnose ambiguity rather than guessing. |

An explicit cut slot is the clearest initial joinery fixture. Preserve `Minuend` as terminology only when its selection rule is documented precisely. Later spatial/layer-aware policies can be added when their semantics and evidence exist.

Specify defaults separately for union, difference, and intersection. Union often preserves contributing exterior surfaces, but it has no “minuend” in the difference sense. Coincident faces and multiple candidates need deterministic precedence or a typed refusal; material work must not quietly paper over kernel ambiguity.

### 6.3 Proposed evaluation order

1. Resolve each operand's feature/region assignments to semantic slots without resolving instance-specific material resources.
2. Establish the output slot namespace and operand remaps.
3. Perform the native Boolean, transporting geometric provenance and supported continuous corner attributes across splits.
4. Select/remap categorical slot attribution according to the operation's cut policy.
5. Apply or retain mapping under its separate policy; report unsupported mapping rather than substitute zero coordinates silently.
6. Bind output slots in the consuming instance/document environment.

Carry explicit policies in authored recipe/node semantics and relevant fingerprints. Do not install them as ambient exporter globals.

Stretch bands, caps, refinement-generated faces, imported surfaces, and grid side walls need the same completeness contract: one slot or explicit unassigned, with valid palette context and declared propagation. They do not all need the same generation policy.

### 6.4 Equivalence tests require semantic equivalence

Use the supplied direct/instanced, cold/warm, and composed tests, but define what is compared. A chained expression is not automatically equivalent to every n-ary expression, particularly for ordered differences, ambiguous surfaces, or policy changes. [S1, §6; synthesis]

Only assert n-ary/chained equality for transformations the operation contract declares equivalent. Compare slot meaning and provenance correspondence, not raw palette numbers or triangle ordering across algorithms that do not promise those properties.

---

## 7. Material descriptions and export

### 7.1 Scope of `exedra_materials`

Start with portable **surface appearance**, not “everything a material means.” A versioned description can cover metallic-roughness factors, base color, emissive appearance, alpha mode/cutoff, double-sidedness, texture references, normal scale, occlusion strength, and supported extension data. This is the bounded glTF-oriented direction proposed by the supplied pass. [S1, §4.3; S9]

Keep texture/image identity and sampler/mapping references explicit. Store data and validation rules; resource acquisition, texture decoding, GPU objects, shader compilation, and renderer caches belong elsewhere. Geometry kernels must not need this crate to carry opaque slot identities.

Untextured PBR materials are useful without UV generation. Therefore “UVs are the real prerequisite” is too strong for the first material-resource slice: UVs are a prerequisite for the intended UV-textured workflow, not for resource/binding semantics or untextured appearance. The first exported material fixture can use factors alone. [S1, §§4.3–4.4; S9; synthesis]

An opaque extension area is acceptable as versioned transport data. Preserve unknown optional fields; distinguish required unsupported capabilities and report/refuse where correctness depends on them. Do not quietly interpret an arbitrary extension as executable behavior. Deterministic serialization and fingerprints must include the relevant extension content.

Keep physical properties such as density, species, stiffness, or manufacturing grade in their appropriate domains. A future geometry operation that reads a displacement resource or other material-related input must declare that dependency. The statement “material edits never affect geometry” applies to appearance-only data not consumed by geometry, not to all future resources called material.

### 7.2 Export behavior

**Baseline:** `exedra_gltf` emits deterministic named PBR stubs derived from material keys and shares glTF meshes when part/body/material resolution match. [S6]

Add a description/resource resolver without moving material ownership into the exporter. Preserve an explicit deterministic stub mode for compatibility or diagnostic preview, but distinguish it from successful resolution of the requested appearance. Provide a strict mode or typed outcome for required missing descriptions/textures/extensions. Missing named resources and deliberately unassigned faces must remain distinguishable in reports.

A material rebinding may change primitive/material tables or mesh wrappers while reusing geometry buffers. “Shared geometry” does not require identical exporter object counts under every binding.

Keep the portable description separate from glTF's transport layout. Validate material factors and texture usage against the supported specification; do not claim complete glTF extension support. [S9]

---

## 8. Surface coordinates, texture mapping, normals, and tangents

### 8.1 Coordinate generation and texture mapping are separate

The supplied pass proposes deterministic per-feature coordinates in model units and real-world texture scale. Preserve that design intent, but define the coordinate contract rather than relabeling arbitrary values as UVs. [S1, §4.4]

Distinguish the source surface/feature parameterization, any metric chart expressed in declared model-distance units, the selected mapping frame/transform, and the dimensionless texture coordinates consumed by a texture lookup.

For a metric chart with coordinates `(s, t)` and repeat lengths `(r_s, r_t)` in the same units, a basic mapping is `(u, v) = (s / r_s, t / r_t)` before rotation/offset. This is a proposed mapping rule, not a universal parameterization for every surface. Imported authored UVs retain their own declared interpretation and must not silently be reinterpreted as lengths.

The glTF specification defines texture coordinates relative to image size; `KHR_texture_transform` adds per-texture offset, rotation, scale, and optional coordinate-set selection. Export can use an explicitly supported transform or bake final UVs into derived render data. It must not assume consumers infer a physical unit from numeric UV values. [S9; S10]

Do not change `CORNER_UV` semantics globally without a migration. The first implementation can generate final UVs under a declared policy using existing attribute machinery; retaining a separate metric chart is an earned optimization/capability, not mandatory new infrastructure.

### 8.2 Feature coverage and limitations

Use the supplied feature list as the implementation inventory, not as a proof that provenance alone already contains enough data:

| Feature | Candidate starting coordinates | Additional contract to define |
|---|---|---|
| Caps | Profile-space planar coordinates. | Frame, units, orientation, holes, seam/slot correspondence. |
| Extrusion walls | Loop distance and extrusion distance. | Loop seam, multiple loops, signed direction, corners. |
| Revolved surfaces | Angular/profile parameters or a declared length-based chart. | Axis, seam, poles, varying radius, distortion and reference radius where used. |
| Sweeps | Path distance and profile distance. | Frame transport, twist, path joints, profile seam and closure. |
| Primitives | Documented per-feature/per-region charts. | Seam locations, orientation, poles and corner discontinuities. |
| Imports | Authored corner coordinates when available. | Coordinate-set identity, missing data, validation and preservation. |
| Boolean fragments | Source-chart interpolation where supported. | Corner discontinuities, orientation changes, cut mapping policy. |

A semantic feature label does not by itself reconstruct local continuous coordinates, chart seams, or interpolation data. Inspect the available source-map/evaluation information; retain enough local information at generation time or report unsupported mapping. Curved surfaces need not have a distortion-free metric chart. Document any approximation rather than imply a universal isometry.

For split faces, interpolate supported continuous corner data under the source face's coordinate convention, preserve seams, and handle orientation correctly. A categorical material slot must never use that interpolation path. Selecting the minuend's material does not automatically make the cutter's UVs appropriate. A cut mapping policy can retain contributor coordinates, use an explicit target-frame projection, or report that the requested mapping is unavailable.

### 8.3 Real-world scale and instancing

Make object/part-local mapping versus world-locked mapping explicit. Repeating or transforming an instance should have a documented effect on texture scale. In particular, nonuniform instance scale needs a policy: texture stretching with the instance and preserving world-space repeat size are different intents.

Prefer instance/resource mapping overrides when they can preserve shared geometry. A renderer/exporter may need derived mapping variants; do not satisfy world-lock by destructively changing every shared mesh. Mirror, negative determinant, seams, and tangent orientation need paired fixtures.

### 8.4 Normals belong to an explicit derived-shading contract

Retain the supplied goal: authored normals and crease intent must not be accidental side effects of arbitrary tessellation settings. But “normals are an export policy” is too narrow. Preview renderers and exporters should consume the same explicit shading preparation contract. Geometry operators that intentionally read or edit normals still need ordinary typed attribute dependencies. [S1, §4.5; synthesis]

**Baseline:** assembly `policy_fingerprint` currently includes `sharp_sin_threshold`, and compiled parts retain extracted `TriMesh` data. Changing the threshold is therefore not presently guaranteed to avoid recompilation. This separation requires an implementation and cache-version transition. [S4]

A proposed shading policy gives authored valid overrides priority, retains authored hard-edge/seam constraints, and explicitly selects angle-based or smooth fallback behavior. Define behavior for missing, invalid, and contradictory authored data. Do not invent a universal threshold during export.

Track at least these distinct effects:

| Change | May require |
|---|---|
| Normal-generation policy | New normal/corner data and render-vertex splits. |
| UV seam or mapping policy | New corner data, render vertices, and possibly tangents. |
| Normal-map usage/coordinate set | Valid tangent-space preparation under the chosen render contract. |
| Placement with reflection/nonuniform scale | Correct normal/tangent transformation and handedness handling. |

Core glTF normal textures use tangent-space data. The supported exporter path must supply or document compatible tangent derivation rather than assume a base-color UV test proves normal-map correctness. [S9]

Do not remove a policy field from a geometry cache key until inspection proves its effect has moved into a separately keyed artifact. If the current implementation still couples geometry and shading generation, retain conservative invalidation until the separation lands. Preserve every attribute-dependent downstream node's correctness.

---

## 9. Evaluation outcomes, reuse, and invalidation

### 9.1 Outcomes are not all errors and not all successful geometry

Define an envelope distinguishing complete success, policy-declared partial output, typed geometric refusal, and execution/infrastructure failure. Malformed definitions/type errors are validation failures. Valid empty geometry is not automatically a refusal. Preserve native diagnostics and fidelity categories instead of collapsing them into a single string. [S2; S4]

A partial result is publishable only under an explicit caller/output policy and retains its report. A refused result must not look like a successful empty object or silently reuse last-good geometry as current output. Expected native refusals are inspectable node outcomes, not VM/scheduler faults.

Keep diagnostic code, severity, source node/output, relevant resource/policy, and native evidence where available. A conversion reports applicable fidelity, approximation, provenance, work, and refusal information; absent evidence remains absent rather than being fabricated. [S2]

### 9.2 Cache deterministic refusals, not arbitrary failed attempts

The supplied pass reports repeated work for warm refused CSG/stretch evaluations. Cache deterministic geometric refusal outcomes under the same complete semantic key as successful outcomes, retaining bounded semantic reports. [S1, §§1, 5]

Do not persist cancellation, interruption, transient I/O failure, memory-pressure failure, or an untracked missing external resource as an eternal geometric refusal. Budget-limited results need the declared budget/capability in their keys or must remain nonpersistent. A changed evaluator/kernel capability or policy must be able to retry a previous deterministic refusal.

Separate cached semantic diagnostics from execution telemetry. A warm hit can reproduce the same refusal explanation while reporting zero new kernel work. It must not claim that old timings/counters describe the current run. Current `CompiledParts::report` already distinguishes retained evaluation counters from current `PartCompiler` counters; preserve and extend that distinction. [S4]

### 9.3 Fingerprint and dependency contract

For every input declare one of: stable content fingerprint/revision, caller-provided epoch, or explicitly uncacheable. Include the relevant operation/type/evaluation schema, scalar inputs with canonical representations, native content identities, conversion policy, seeds, and declared resource dependencies. Native compatibility versions must participate when algorithm behavior changes. [S2; S4]

Resource identity and resource content are separate. Editing a description under a stable key changes its revision/fingerprint; it does not turn every unrelated geometry node into a dependent. Conversely, a global lookup that is omitted from the dependency set is a correctness bug even if its key has a nice name.

Use a hash/dependency structure that can distinguish shape, attribution, mapping/shading, bindings/resources, structure, and final consumer artifacts. This is an invalidation model, not an instruction to build six cache systems immediately.

| Edit | Expected invalidation after the relevant separation is implemented | Must remain reusable |
|---|---|---|
| Node position/comment only | Authoring presentation. | All computational artifacts. |
| Geometric parameter or tolerance | Affected native results and their downstream consumers. | Unrelated graph branches/resources. |
| Cut-slot/face assignment | Attribution and affected derived partitions; recompute more conservatively until separately cached. | Shape-only work where the implementation actually separates it. |
| Instance slot rebinding | Binding resolution, render/export material partitions. | Native shape evaluation/tessellation. |
| Appearance factors/texture content | Dependent material/resource/render/export outputs. | Geometry not explicitly reading those resources. |
| Mapping policy | Coordinates/tangents/render extraction as required. | Native shape/topology evaluation when independent. |
| Derived normal policy | Shading preparation/render extraction. | Shape/topology after the audited separation. |
| Instance placement | World bounds, composition, and any world-dependent mapping. | Part-local tessellation. |
| Source asset content | Actual asset consumers and their downstream outputs. | Unrelated assets and instances. |
| Kernel/schema version | Affected successful and refusal cache entries. | Unrelated compatible domains where keys permit reuse. |

An attributed mesh fingerprint can legitimately change when its UVs, normals, or slots change. Do not promise it stays identical merely because its topology did. Similarly, rebinding can change export bytes without requiring tessellation. Acceptance tests must name the correct identity and work counters.

### 9.4 Shared evaluation and provenance

The supplied pass reports duplicate tessellation when one definition is used directly and through an instance within an uncached evaluation. Inspect existing native memoization before adding another graph-wide cache. Reuse must be keyed by semantic inputs and policy, not by incidental traversal position. [S1, §1]

Content-equivalent geometry can be shared while occurrence provenance remains distinct. Do not deduplicate away authored node identity or make one instance's diagnostic source masquerade as another. Share immutable computational payloads and retain/rebind contextual provenance through an explicit mapping.

A cache optimization cannot change requested results, typed refusal meaning, or fidelity. Compare incremental/cached execution with a clean evaluation under the same declared policy. Byte equality applies to the promised deterministic artifact representation, excluding runtime-local IDs, current cache counters, timings, and other deliberately nonsemantic telemetry. Do not silently expand a platform-specific numeric guarantee into universal cross-backend bit identity.

---

## 10. Preserve the probe evidence without promoting it to a guarantee

The supplied pass describes 21 composed public-API models, contract checks, GLB export, and headless rendering after seven review fixes. These are useful regression leads, not evidence that every unsupported composition is correct. [S1, §1]

| Reported observation | Implementation response |
|---|---|
| No wrong results in those probes after the review fixes. | Preserve the exact fixture corpus and report its tested envelope. Do not summarize this as universal correctness. |
| `Instance` under `Mirror` previously refused instead of mirroring; reported fixed on `review-hardening`. | Locate the actual fix/commit and regression in the current checkout before changing anything. Preserve mirror, shared-use, provenance, and attributes together. |
| Equal-radius perpendicular cylinders refuse with dangling cuts; unequal radii or a five-degree skew succeed; three-way crossings also refuse. | Keep both refusal and nearby-success fixtures. Do not silently skew or perturb requested geometry to manufacture success. |
| Two circles with different radii can discretize to different counts and make a loft fail with `SectionMismatch`. | Distinguish unsupported valid loft requests from malformed input; emit a typed geometry outcome before pursuing general resampling. |
| `DegenerateLoft` is also proposed for envelope-style diagnostics. | Inspect its actual causes before converting every case to the same category. Retain source context and stable `eval.loft.*` diagnostics where consistent with conventions. |
| `Revolve` uses local Y while `Cylinder` uses Z; contradictory documentation was reportedly fixed. | Verify code and interchange docs, test axis conventions, and keep saved semantics stable. An explicit axis parameter is a later versioned option. |
| Refused evaluations repeat work on warm runs. | Add deterministic refusal caching with the transient-failure qualifications in §9.2. |
| Direct plus instanced use can tessellate one definition twice within an uncached evaluation. | Measure native evaluation visits/work, then share payloads without losing occurrence provenance. |
| Chained axis-aligned cuts can encounter `Robust` triangulation diagonals on cutter planes. | Reproduce and classify alongside edge-on cylinder cases. Shared symptoms are not proof that one patch fixes both. |
| Constructive features generally lack emitted UVs; imports/stretch carry some; zero coordinates reach export for other cases. | Audit the exact generation/extraction/export boundary that inserts or propagates zeros. Do not assume the exporter is the sole owner of the missing data. |
| Probe cylinders shade faceted while revolved spheres shade smooth under current sharpness policy. | Reproduce with controlled shading/extraction settings; separate appearance-policy work from tessellation only after auditing current uses. |

For loft resampling, do not implement “make counts match” without specifying closed-loop correspondence, orientation, seams, preserved corners/features, tolerance, and fidelity. A supported resampling policy should report its changes (the supplied pass proposes `PolicyDefined`) and deterministic limits. A typed refusal for an unsupported valid request is a useful first correction and should not wait for a complete resampler. [S1, §5]

---

## 11. Work packages and sequencing

These are proposed work-package labels, not existing repository ticket IDs. Implement reviewable slices. Do not create all the proposed crates, resource systems, and abstractions before demonstrating one consumer.

### P0 — Establish the actual baseline

Read checkout-local instructions/tenets, current accepted ADRs, open work, and tests. Record the checkout commit and branch, locate `review-hardening` and `examples/constructive_probe` if available, and map each reported fix to current code. Check in-flight addressing/material work before creating overlapping infrastructure. The remote listing included addressing-related branches; their changes were not inspected here. [S11]

Useful discovery commands, to be adapted to the checkout:

```sh
git status --short
git rev-parse HEAD
git branch --all
rg -n 'resolved_material|region_slot|SlotIndex|FACE_REGION|FACE_SLOT' crates
rg -n 'SectionMismatch|DegenerateLoft|sharp_sin_threshold|CORNER_UV' crates
rg -n 'EVAL_SCHEMA_VERSION|policy_fingerprint|PolicyDefined' crates
rg -n 'execution_graph|understory_node_graph|procedural' README.md ROADMAP.md crates
```

**Deliverable:** a compact evidence table of implemented, branch-only, reproduced, and still-unverified items. Link existing regressions. Missing probe code does not justify inventing its outputs; construct a minimal replacement fixture only when the underlying request is sufficiently specified and label it as a new fixture.

### Q1 — Tighten evaluation outcomes and refusal reuse

After baseline discovery, fix the valid-loft-request outcome boundary if still needed. Keep malformed input distinct. Add deterministic refusal memoization in the appropriate native evaluator/cache rather than special-casing it only in a future UI. Add current-run telemetry separate from retained reports.

**Exit:** a repeated deterministic refusal performs no repeated kernel work, retains its explanation, and can be retried when policy/evaluator identity changes. Cancellation and transient resource errors do not poison persistent caches. Loft refusal remains identifiable through assembly/procedural adapters.

Kernel degeneracy repairs and the direct/instance duplicate-work fix can be separate changes driven by the relevant fixtures. Do not make all future network work wait for universal Boolean support.

### M1 — Establish the slot carrier through one complete pipeline

Specify stable slot identity, palette ownership, remapping, explicit unassignment, and the two-stage resolution contract. Implement a bounded recipe-to-mesh composition, including one supported difference with an explicit cut slot, and carry attribution through assembly compilation, flattening, and export inspection. Update range partitioning, not just mesh storage.

**Exit:** two operands both using local slot index 0 for different semantic slots remain distinct; no face has an invalid slot; changing occurrence bindings does not rerun tessellation. Reports preserve geometric provenance independently from material choices.

Full ancestor inheritance is not required to pass this stage. Preserve existing instance/default behavior and document the versioned extension seam.

### M2 — Add usable material resources

Introduce the smallest shared appearance description and resolver contract. A dedicated `exedra_materials` crate is appropriate when actual consumers justify it; otherwise establish the module boundary without a premature public package split. Adapt `exedra_gltf` to real supported descriptions, missing-resource diagnostics, and explicit stub fallback.

**Exit:** two instances of one compiled part use different untextured descriptions. Editing roughness/base color or rebinding one instance changes the expected export/material result with zero native geometry evaluation and no mutation of the other instance. Round-trip descriptions and references deterministically.

M1 and M2 can proceed independently where their shared slot/binding contract is fixed. M2 does not wait for all UV generation.

### V1 / V2 — Add mapping and separate derived shading incrementally

**V1:** support one feature's deterministic mapping, its declared units/frame, and a textured export. Add cut mapping and mirror/instance coverage before claiming compositional support. Expand feature coverage from the inventory in §8.2 with per-feature acceptance tests.

**V2:** audit normal/crease policy uses, split geometry and derived-shading work where justified, migrate fingerprints, and share the preparation contract between render/export consumers. Add tangent-space tests where normal maps are supported.

**Exit:** a mapping/normal-policy edit invalidates the correct derived artifacts without stale cache reuse. After the split, shape-only evaluation is demonstrably reused. Unsupported mapping/shading combinations are reported instead of being rendered as unexplained zeros or guessed normals.

### D1 — Retain a minimal document/resource model

Add the minimum document layer that binds stable object/output references, definition invocations, materials/policies, and publication intent. Use existing assembly and editor storage appropriately; do not duplicate their semantic ownership. Persist it without needing an editor or live runtime handles.

**Exit:** load/save preserves stable meaning; a material resource edit invalidates its actual consumers; stale evaluation cannot replace newer document results; unknown required operations/resources remain inspectable. Native crates remain independently usable and know nothing about the document.

D1 needs the small contracts, not complete material inheritance, every UV generator, or a production node canvas.

### G1 — Earn the procedural crate with the ADR's native-network proof

Inspect the current shared execution implementation. If the native-executor seam is still missing, implement it in the execution owner with its own tests and an explicit dependency/revision strategy. Do not add an Exedra-private scheduler to avoid touching the shared layer. If coordinated changes cannot land together, retain a standalone contract fixture rather than publishing a substitute runtime.

Create a headless `exedra_procedural` definition that:

1. Accepts a named geometric parameter and builds a constructive recipe.
2. Explicitly evaluates the recipe to a mesh with a policy/report.
3. Applies one supported `exedra_ops` edit to one branch of that mesh.
4. Sends the unchanged upstream value to another consumer, proving branch isolation.
5. Publishes a supported object/assembly result with named slots and late material binding.
6. Changes the parameter and reports selective execution; includes an independent unaffected branch so selective reuse is actually measurable.
7. Changes only a material description/binding and reports zero geometry work.
8. Saves/reloads the same authored definition and later projects it through `understory_node_graph` without changing its semantic identity.

The two-native-value conversion and incremental proof are the accepted entry gate; the material-only edit extends that proof to the new resource concerns. [S2]

**Exit:** clean and incremental deterministic artifacts agree; conversion reports survive; expected refusal is an inspectable outcome; editor movement causes no execution; no shared branch is mutated.

A headless G1 can run in parallel with appearance refinement after the required resource/slot contracts are agreed. Do not make every texture, normal, kernel-degeneracy, or grammar task a prerequisite.

### G2 — Prove composition, then consider additional language features

Add one reusable definition with stable named inputs/outputs and one explicit keyed mapping operation. Demonstrate independent invocations, parameter/resource binding, generated identity through insertion/omission, and correct error attribution to a call site plus source definition.

**Exit:** the system is a compositional language, not just a registry of callbacks. Only then prioritize additional collection matching, bounded iteration, solver regions, textual syntax, or grammar expansion against a concrete consumer.

---

## 12. Acceptance matrix

Use focused unit tests, native integration fixtures, and measured end-to-end tests. Visual renders supplement structural assertions; they do not replace them. The identifiers below are proposed test categories, not claims that these tests already exist.

| ID | Fixture/change | Required assertion |
|---|---|---|
| OUT-1 | Supported valid loft request outside current evaluator capability. | Typed, source-attributed outcome instead of an opaque whole-workflow failure; valid other outputs handled according to explicit policy. |
| OUT-2 | Same refused geometry, cold then warm. | Same semantic outcome/diagnostics, no repeated kernel work on the warm hit. |
| OUT-3 | Refusal followed by policy/kernel-version change. | Cache invalidates and reevaluation occurs. |
| OUT-4 | Cancelled/transient failure then valid retry. | No permanent negative-cache poisoning. |
| OUT-5 | Valid empty result versus no-geometry refusal. | Distinct meanings and publication behavior. |
| SLOT-1 | Different operand palettes both containing index 0. | Output retains two correct semantic slot identities after remapping. |
| SLOT-2 | No assignment, explicit clear, and missing bound resource. | Distinguishable outcomes; no fallback that hides intent. |
| SLOT-3 | One provenance region with multiple slots. | Compiled/flattened/exported partitions preserve every assignment and cover all triangles. |
| SLOT-4 | Multi-slot minuend with no unique default. | Explicit cut slot succeeds; ambiguous automatic choice is diagnosed, not guessed. |
| SLOT-5 | Difference cut surface. | Cutter geometric provenance can coexist with the chosen host/output material slot and separate mapping policy. |
| SLOT-6 | Import, mirror, stretch, supported refinement and grid/cap faces. | Valid slot or explicit unassigned on every emitted face; correct palette context. |
| SLOT-7 | Declared equivalent grouping/direct/instanced paths. | Equivalent semantic attribution; no comparison of meaningless raw local indices. |
| MAT-1 | Two placements of one part with distinct bindings. | One reusable geometric payload, separate resolved appearances. |
| MAT-2 | Edit appearance factors or rebind one instance. | No native shape evaluation/tessellation; only affected binding/resource/export consumers change. |
| MAT-3 | New inherited override and explicit clear, when supported. | Tested precedence and semantic slot targeting; old documents retain old behavior. |
| MAT-4 | Missing description or unsupported required extension. | Typed report/strict refusal or explicitly selected diagnostic fallback. |
| UV-1 | One generated feature under a specified mapping. | Deterministic, non-placeholder coordinates with documented frame, seams and units. |
| UV-2 | Textured Boolean fragments and generated cut surface. | Correct interpolation/seams and explicit cut mapping; no categorical-slot interpolation. |
| UV-3 | Shared part under mirror and nonuniform instance scale. | Declared texture-scale behavior without destructive shared-mesh edits. |
| SHADE-1 | Normal policy change after separation. | Correct normals/render splits; shape work reused; relevant cache keys change. |
| SHADE-2 | Supported normal map across UV seams/reflection. | Valid tangent-space behavior under the declared exporter/renderer contract. |
| SHARE-1 | Same definition used directly and as an instance source. | Measured payload reuse with distinct occurrence provenance. |
| GRAPH-1 | Recipe-to-mesh conversion, edit branch, untouched branch. | Explicit native crossing/report and no branch aliasing mutation. |
| GRAPH-2 | Geometry parameter edit plus independent branch. | Correct targeted reexecution; clean/incremental equality for promised artifacts. |
| GRAPH-3 | Node move/comment edit. | No computational fingerprint or execution change. |
| DOC-1 | Save/reload with editor/runtime recreated. | Stable authored meaning and references; no serialized live handles. |
| DOC-2 | Older evaluation completes after newer revision. | Old result cannot silently overwrite current publication. |
| DOC-3 | Unknown operation/type and unresolved semantic selection. | Round-trip preservation where safe, explicit blocked/ambiguous status. |
| LANG-1 | Keyed map with insertion/omission and unequal zip inputs. | Stable generated identity and explicit matching/refusal semantics. |
| BOUND-1 | Core dependency and feature checks. | No native-to-document dependency, no universal geometry payload, existing portability/lint policy preserved. |

Report geometry work, attribution/mapping work, material resolution, render/extraction work, export work, and current cache hits separately where implemented. Do not claim a tessellation improvement merely because a renderer reused a draw call, or claim stale cached normals are “incremental.”

Use the workspace's documented formatting, lint, test and documentation gates, then the relevant crate-specific `no_std`/feature checks after inspecting each manifest. A single `--all-features` run is not a substitute for minimal-feature checks. Do not claim these commands have been run on the basis of this design document. [S8]

---

## 13. Code-reading and ownership map

These are known source locations or explicit search targets, not an exhaustive implementation audit.

| Location | Inspect before changing |
|---|---|
| `crates/exedra_ops/docs/adr-0005-exedra-ops-and-cross-domain-boundary.md` | Native heads, explicit conversions, typed ports/artifacts, identity mapping, shared execution/editor responsibilities, first-network gate. |
| `crates/exedra_assembly/docs/adr-0001-structure-head-scope.md` | Parts/instances, material defaults, no retessellation on rebinding, independent assembly scope. |
| `crates/exedra_assembly/src/assembly.rs` | `region_slot`, slot indices, bindings/defaults, `resolved_material`, stable keys and invalidation triggers. |
| `crates/exedra_assembly/src/compile.rs` | `policy_fingerprint`, `PartCompiler`, `CompiledBody`, `RegionRange`, report/counter distinction, cache/schema keys. |
| `crates/exedra_assembly/src/flatten.rs` | `ResolvedRegion`, triangle partitioning, binding resolution, publication/render seam. |
| `crates/exedra_assembly/src/interchange.rs` | Existing format conventions; inspect before versioning slots, binding states, or resource references. File presence was observed; contents were not reviewed for this synthesis. |
| `crates/exedra_gltf/src/lib.rs` | Stub material creation, primitive partitioning, sharing criteria, export options and errors. |
| `crates/exedra_constructive` | Locate evaluator caches/outcomes, loft code, `EvalPolicy`, source maps, material-slot IR, CSG propagation, and feature UV generation. Do not assume each symbol lives in a same-named flat file. |
| `crates/exedra_mesh` | Typed face/corner attribute storage and Boolean/split/copy/interpolation/extraction behavior. |
| `crates/exedra_ops/src/uv_planar.rs`, `uv_box.rs`, `uv_cylinder.rs` | Existing mapping operations before writing another UV subsystem. Search excerpts were inspected, not full implementations. |
| `crates/setout_generate/README.md` and its implementation | Stable labeled generation, omissions, consumer-neutral output; reuse rather than absorb domain ownership. |
| Neighboring execution/editor/addressing crates | Locate their actual repositories and inspect current APIs. Their code was not independently audited here; ADR descriptions may lag implementation. |
| `README.md`, `ROADMAP.md`, `Cargo.toml`, local instructions and current ADRs | Existing project scope, accepted milestones, feature/lint policy and in-flight work. |

### 13.1 Deliverables for each implementation slice

Each slice should include a concrete consumer fixture, tests of its outcome and invalidation behavior, documented migration where serialized meaning changes, and an owning-crate design/ADR update when needed. State the supported envelope and remaining refusals.

Do not rename or repurpose existing public concepts solely to make this proposal's illustrative vocabulary compile. Keep current APIs usable unless a deliberate reviewed migration is necessary. Do not add heavyweight host dependencies to native `no_std + alloc` paths, relax safety/lint policy, or introduce a new generic framework without measured need.

### 13.2 Instructions to the implementing agent

> Start with P0. Read the accepted ownership decisions and compare them with the checkout, including unpublished probe/fix work. Return a factual baseline and select a narrow next change; do not implement the whole roadmap in one pass.
>
> Preserve Exedra's native representations and explicit conversion reports. Keep `exedra_assembly` independently useful. Treat `exedra_procedural` as the proposed geometry-language integration layer, not another scheduler or universal geometry model. Reuse the shared execution, editor, addressing, and native attribute systems after checking their actual APIs.
>
> Prioritize the valid-loft outcome boundary when still broken, and define slot identity/ownership before propagating `FACE_SLOT`. Use explicit cut-slot selection in the first multi-material Boolean fixture. Keep material binding and appearance resources out of geometry evaluation unless declared inputs require them. Do not hide missing UVs or material resources behind silent defaults.
>
> Prove every claimed reuse boundary with current-work counters and a clean-evaluation comparison. Preserve deterministic refusal reports without replaying old execution timings as new work. Record branch/version provenance for every claim about the supplied probes.
>
> Build materials and the headless network as coordinated but separable tracks. Earn the procedural crate with a recipe-to-mesh crossing, a supported mesh-edit branch, immutable sharing, and selective reevaluation. Then add the material-only edit and persistence proof. Defer full grammar, text syntax, hierarchy generalization, and other large features until a consumer requires them.

---

## 14. Source register and limits

**S0 — Preceding conversation, 2026-09-06.** Naming recommendation `exedra_procedural`; durable `Definition` versus compiled execution; typed dataflow; explicit collections/conversions; grammar as an optional specialized producer; shared execution/editor boundaries. This document refines that proposal rather than treating it as an accepted repository decision.

**S1 — Supplied design pass.** “Design pass: a unified modeling document, and where materials live,” status “design pass, 2026-09-06. Not a decision.” Supplied attachment: `Pasted markdown(20260906-155058).md`. SHA-256 of the received file: `26273a1f934b94b53192f34a6858e645fb44cafbe8a41aa71ef79601239e5652`. Its six sections cover probes; Houdini/Rhino lessons; document placement; materials/UVs/normals; evaluation refinements; staged roadmap. All branch-specific test/fix claims here are attributed to that pass. The original was fully available during synthesis; the implementing agent should preserve it alongside this document when possible.

**S2 — Accepted operations/cross-domain ADR.** Repository `forest-rs/exedra`, `crates/exedra_ops/docs/adr-0005-exedra-ops-and-cross-domain-boundary.md`, accepted 2026-09-04. Read in the preceding turn; source blob SHA `eafbbfba3f7f8e09a7d0cd58ec02565e85bc07bf`. Relevant sections: “Native heads and crossings,” “Assemblies and part domains,” “Future procedural network,” alternatives. Primary authority for the ownership boundaries retained here.

**S3 — Accepted assembly ADR.** `crates/exedra_assembly/docs/adr-0001-structure-head-scope.md`, read at `42ec07adf88acb24f49d4a6817ab7a747f526bc8`. Source blob SHA `7a84da74c809701f4aebe09e9877226c837e16f5`. Also `crates/exedra_assembly/README.md` at that commit.

**S4 — Assembly compilation source.** `crates/exedra_assembly/src/compile.rs`, inspected lines 1–270 at the same pinned commit; blob SHA `0f03a04720622e16a877f3c3047d4e211513acb3`. Direct support for current fingerprint inputs, region grouping, part/report structure, binding-independent invalidation contract, and retained-report/current-work distinction. Cache internals beyond that inspected range require further review.

**S5 — Assembly flattening/binding source.** `crates/exedra_assembly/src/flatten.rs`, inspected lines 1–300 at the pinned commit; blob SHA `9ccd513aafea49ac2b09c9fa5df4e40d0dfc7d5f`. Also the `resolved_material` search excerpt from `src/assembly.rs` at the same commit. Supports current region-to-slot material resolution, not a claim of existing general ancestor inheritance.

**S6 — Export source.** `crates/exedra_gltf/src/lib.rs`, inspected lines 1–190 at the pinned commit; blob SHA `0d6db2d7dbe1b035cd0a4a835778f98d5ccef321`. Supports current documented stub materials, sharing criteria, export options, and surface contracts. Not a full audit of all exporter internals.

**S7 — Labeled generation.** `crates/setout_generate/README.md`, read in the preceding turn; blob SHA `32c5ba195671a7ce0410c22fec473fb93204ad29`. Supports stable semantic labels/omissions and separation of generation from geometry consumers.

**S8 — Workspace context.** `README.md`, `ROADMAP.md`, and `Cargo.toml` read in the preceding turn. They establish the facade/native-head scope, roadmap discipline, current feature/dependency conventions and workspace gates. Re-read the checkout's versions before editing; do not treat this document as the latest task tracker.

**S9 — External specification check.** Khronos glTF 2.0 specification, targeted checks of material-factor, image-relative texture-coordinate, and tangent-space normal-texture semantics. Reference: `https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html`. Official-source search passages were available; full HTML retrieval timed out during this review. No exporter-conformance or comprehensive specification audit was performed. Validate actual serialized output against the current supported specification during implementation.

**S10 — External texture-transform specification.** Khronos `KHR_texture_transform`, ratified extension documentation, read 2026-09-06. Reference: `https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_texture_transform/README.md`. Used only to check offset/rotation/scale/coordinate-set transport and fallback implications; it does not prescribe Exedra's internal metric-chart architecture.

**S11 — Remote branch discovery.** `forest-rs/exedra` branch listing retrieved 2026-09-06. It identified remote `main` at `42ec07adf88acb24f49d4a6817ab7a747f526bc8` and several addressing/material-related branches. No `review-hardening` entry appeared in the returned listing. That observation does not establish the implementing agent's local branch state.

### Final acceptance principle

An evaluation either produces the requested result within its declared policy or accurately explains what it could not produce. Caching, instancing, grouping, binding, mapping, and editor presentation must not change that truth. Retain native meaning, separate resource lifecycles, make crossings inspectable, and demonstrate the boundaries through concrete composition tests rather than architectural names alone. [S1, §6; S2]
