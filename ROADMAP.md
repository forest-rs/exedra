# Exedra roadmap

Exedra is an independent mesh kernel and geometry toolkit for constructing
inspectable assets for virtual worlds. The roadmap is deliberately smaller
than the implementation history: completed work remains in commits and ADRs;
only current decisions and unresolved outcomes belong here.

## Architecture and scope

Exedra owns editable polygon topology, attributes, validation, deterministic
extraction, and mesh-mutation invariants. Exedra Ops owns deterministic
mesh-operator planning, execution, diagnostics, selections, and explicit
feature-gated adapters. Sibling heads own their native representations:

- `exedra_constructive`: immutable fingerprinted recipes and deterministic
  tessellation;
- `exedra_isosurface`: scalar fields, Hermite evidence, QEF fitting, and field
  extraction;
- `exedra_analytic`: a narrow planar analytic head;
- `exedra_assembly`: parts, stable instance paths, material-slot binding,
  compilation, and flattening.

The goal is inspectable geometry production: conversions, instances, regions,
source references, diagnostics, bounded fallbacks, and measured work should be
deterministic and visible.

This roadmap does not promise a universal geometry representation, a scene
graph inside the mesh kernel, CAD-grade exact arithmetic everywhere,
structural certification, or acceptance of every field or contact topology.

## Current capability

The workspace currently provides:

1. A `no_std` half-edge mesh kernel with generational IDs, explicit outside
   boundary faces, typed attributes, validation, edit scopes, change summaries,
   compaction, and deterministic render extraction.
2. Corner UVs and normals, authored sharpness and seams, deterministic polygon
   triangulation, and render-vertex splitting at shading and parameter seams.
3. A staged mesh-Boolean pipeline with broad and narrow phases, splitting,
   classification, stitching, provenance, through-holes, selected coplanar
   contacts, seam cleanup, and typed diagnostics.
4. Exedra Ops mesh compile, preview, and apply lifecycles with stale-plan
   rejection, reports, timings, bounded artifacts, diagnostics, selections, UV
   projection, face and normal edits, and Boolean orchestration.
5. Constructive profiles and recipes for extrusion, revolution, lofting,
   polyline sweeps, grid surfaces, primitive and import leaves, exact and
   mesh-backed stretch, transforms, mirrors, instances, groups, source maps,
   regions, fidelity reports, interchange, and fingerprint-keyed caching.
6. Assembly parts and named instances with stable paths, material binding,
   cached part compilation, deterministic flattening, interchange, inspection
   payloads, and glTF export.
7. Scalar-field composition, analytic reference fields, QEF fitting,
   interval-culled adaptive dual contouring, mixed-depth balancing, provenance,
   corner normals, and bounded semi-analytic box and cylinder projection.

These capabilities are useful but not universal. Unsupported or ambiguous
configurations must remain typed failures or explicitly counted fallbacks.

## Cross-domain convergence

1. `exedra_ops` is the current direct-operation SDK: its deterministic runner
   plans and applies mesh operations, while focused adapters expose explicit
   crossings into or out of sibling native heads.
2. A future Exedra procedural-network layer is earned only by a real pipeline
   that requires at least two native value kinds joined by an explicit
   conversion. It owns the typed geometry vocabulary, native artifacts,
   conversion reports, and compilation adapters. The shared `execution_graph`
   runtime owns incremental scheduling and execution causality;
   `understory_node_graph` owns editor documents and projections. None owns a
   universal geometry value.
3. Native heads keep their algorithms, identities, and provenance. CSG remains
   domain-native: recipes combine construction intent, meshes use polygonal
   operations, and fields compose before extraction.
4. Persistent implicit identity and a general surface/B-rep head are separate
   earned capabilities. The current field and planar-shell crates do not yet
   provide either one.

## Active milestones

### 1. Kernel correctness hardening

Make the remaining topology failures explicit and non-mutating:

- remove the seam-cleanup and rounding panic (`exe-8kli`);
- return a typed refusal for non-manifold Boolean contacts (`exe-n7vs`);
- canonicalize coincident Boolean seam vertices (`exe-ot9t`).

Exit: the in-scope fixtures either succeed or fail deterministically with a
typed result, and failed mutations leave the mesh unchanged.

### 2. Constructive-to-assembly completion

Finish the calm path from immutable recipes to named assemblies:

- preserve constructive diagnostics during compilation (`ea-8tpb`);
- add mirror-safe immutable recipe composition (`ec-b1gl`);
- build mirror-safe named expansion on that recipe seam;
- keep stable instance paths, material bindings, reports, cache counters, and
  byte-stable flattening through parameter changes.

Exit: a changed recipe yields a fully inspectable assembly and a clean rebuild
matches the incremental result.

### 3. Inspectable interchange and provenance

Unify source maps, instance paths, regions, materials, diagnostics,
fingerprints, viewer payloads, and glTF extras into one inspection story. The
first concrete consumer is deterministic end-to-end provenance inspection
(`cwb-ddf7`). Coordinate conversion remains an exporter policy, not a kernel
concern.

Exit: a consumer can identify every rendered body, instance, region, source
feature, and bounded diagnostic without reading internal state.

### 4. Field-extraction stabilization

Preserve the measured phase-1 adaptive dual-contouring envelope: QEF fitting,
conservative intervals, mixed-depth transitions, provenance and seams, normals,
and supported semi-analytic features. Keep the general manifold problem
explicit (`ei-vnhj`) rather than silently broadening current claims.

Exit: supported field classes, approximations, fallbacks, and rejections are
documented and covered by deterministic topology and quality oracles.

### 5. Triangulation quality

Improve poor choices without changing the boundary vertex set: add an exact,
deterministic incircle predicate (`et-jmpb`), then evaluate constrained-Delaunay
edge legalization against the existing quality wind tunnel.

Interior Steiner refinement remains a later, consumer-driven step because it
changes provenance and face-replacement requirements.

Exit: quality improvements are measured, deterministic across scale, and do
not weaken `no_std`, dependency, or provenance contracts.

### 6. Measured incremental workflows

Add deeper dirty extraction, revision-pinned lineage, feature-level diffs, or
parameter deltas only for a named end-to-end consumer. Prefer one measured edit
path over a generalized framework built in anticipation.

Exit: an edit reuses unaffected work, reports that reuse, and produces output
identical to a clean rebuild.

## Deferred, not dropped

The following remain valid future work but are not in the active dependency
chain:

- general manifold dual contouring beyond the bounded phase-1 extractor;
- constrained-Delaunay legalization, Steiner cap refinement, and guaranteed
  minimum-angle meshing after the incircle predicate;
- subdivision operators;
- curved planar sweeps and recipe-level profile fillets;
- broad edit-lineage, feature-diff, and derived-normal patching facilities;
- automatic glTF basis conversion;
- additional semi-analytic primitives and exact feature intersections.

Deferral means the capability has a plausible owner and motivation, but no
active promise or dedicated ticket until a consumer pulls it into scope.

## Separate research and showcase track

The basilica ruin, the structural basilica laboratory, timber-joint specimens,
and future structural graph experiments are valuable integration and
inspection scenarios. They are not dependencies of the kernel.

The current structural lab demonstrates named contacts, geometric coherence,
evidence labels, and load-path witnesses for a bounded roof-bay model. It does
not establish load capacity, stiffness, buckling resistance, deterioration
behavior, code compliance, or engineering certification. Further structural
detail should remain a separately reviewed research/showcase program.

## Completed plans

The former v0.1 critical-path, implicit-surface branch plan, multi-domain
architecture program, assembly foundation, constructive foundation, and first
incremental-regeneration milestone have been absorbed into the current
implementation. Their durable decisions remain in crate-local ADRs and commit
history; their old phase numbering is no longer an active roadmap.
