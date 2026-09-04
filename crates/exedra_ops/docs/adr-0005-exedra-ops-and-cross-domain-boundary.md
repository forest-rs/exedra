# ADR-0005: Exedra operations and the cross-domain boundary

- Status: Accepted
- Date: 2026-09-04
- Owners: Exedra maintainers
- Ticket: `cam-ddhq`
- Amends: ADR-0001 and ADR-0002

## Context

Before its first publication, the operator crate needs a durable name. Its
internal working name is unavailable and, more importantly, makes one layer
sound like a separate product rather than part of the Exedra geometry system.

The implementation and its architectural description have also diverged. Its
mesh surface is substantial: deterministic planning, stale-plan rejection,
preview/apply parity, reports, policies, selections, and direct mesh edits.
Its `EditOperator` trait and `OperatorRunner` nevertheless accept only
`exedra_mesh::Mesh`. Analytic, constructive, and assembly workflows use separate
functions with separate lifecycles, while scalar-field extraction is not
present in the crate at all. The former `OperatorDomain` enum classified
intent; it never dispatched heterogeneous values.

Calling that implementation the cross-domain orchestrator would turn an
architectural direction into a misleading present-tense guarantee.

Established procedural systems point toward a more durable shape:

- Houdini networks can carry several primitive kinds, use explicit conversion,
  and retain packed shared geometry until it must be unpacked.
- Blender Geometry Nodes carries separate mesh, curve, point-cloud, volume,
  and instance components, with instance realization as an explicit operation.
- Grasshopper wires typed values and makes conversion between geometry types a
  visible part of component communication.

The useful common idea is not one universal geometry representation. It is a
graph that can carry several honest representations, preserve sharing, and
make every lossy crossing inspectable.

## Decision

Name the implemented SDK `exedra_ops` and keep its present capability honest.

The system story is:

> Exedra keeps geometry in the native domain that can express it best, changes
> it through typed operations, and crosses domains only through explicit,
> observable conversions.

### `exedra_ops` fence

`exedra_ops` owns:

- deterministic planning, preview, application, and reporting for mesh edits;
- mesh selections, edit policies, diagnostics, bounded artifacts, and runtime
  scratch/cache invalidation;
- curated mesh operations and fluent mesh-edit composition;
- typed workflow adapters, including the domain-neutral pattern planning and
  expansion in `exedra_ops::assembly`.

It explicitly does not own:

- polygon topology or attributes (`exedra_mesh`);
- constructive recipes or their evaluation (`exedra_constructive`);
- analytic shell state or tessellation (`exedra_analytic`);
- scalar fields or isosurface extraction (`exedra_isosurface`);
- parts, instances, or render flattening (`exedra_assembly`);
- a universal mutable geometry type;
- a heterogeneous graph, scheduler, or plugin ABI.

The `EditOperator`/`OperatorRunner` contract remains mesh-specific. The
non-mesh modules are typed adapters with their own input, output, and failure
contracts. Remove `OperatorDomain` and `EditOperator::domain()`: every
implementation returned `Mesh`, and retaining unused non-mesh variants would
make an unimplemented dispatcher look like an extension point. A future graph
expresses domains through typed ports instead.

### Feature boundary

The default `exedra_ops` build is the mesh-operation surface with `std`. Native
head adapters remain independently selectable. The application-facing
`exedra` facade owns the broader common feature bundle; direct operation users
do not compile unrelated heads unless they request them. A `no_std` mesh
consumer selects `libm` with `default-features = false`:

| Feature | Enables |
| --- | --- |
| `analytic` | analytic edits and the analytic-to-mesh conversion seam |
| `constructive` | constructive recipe lifecycle and recipe-to-mesh conversion |
| `assembly` | domain-neutral pattern planning and expansion over `exedra_assembly` |

`profile_section` is part of the `constructive` feature because it produces
constructive profiles. `convert` is available when either `analytic` or
`constructive` is enabled, with each set of items gated independently. The
`assembly` feature activates its own direct assembly dependencies and supporting
placement types; it does not activate the separate constructive adapter API.
The `std` and `libm` features forward weakly to an adapter only when that
adapter is enabled, so either backend remains valid on its own.

### Native heads and crossings

Each representation keeps the lifecycle that gives it value:

| Head | Native value | Character | Typical explicit crossing |
| --- | --- | --- | --- |
| `exedra_mesh` | `Mesh` | mutable polygon topology and authored attributes | render extraction or input from another head |
| `exedra_constructive` | `Recipe` | immutable, fingerprinted construction intent | policy-controlled evaluation to one or more meshes |
| `exedra_analytic` | `AnalyticShell` | bounded editable planar topology | deterministic tessellation to a mesh |
| `exedra_isosurface` | `ScalarField` implementations | continuous inside/outside evaluation | policy-controlled isosurface extraction to a mesh |
| `exedra_assembly` | `Assembly` | named part definitions and retained instances | deterministic flattening to a render list |

Crossing a boundary is an operation, not an implicit cast. Each crossing must
document and expose the evidence that applies to its contract: for example,
provenance, fidelity, approximation, work, or refusal. A crossing must never
silently hide loss; dimensions that do not apply or are not available are
recorded as absent. Converting to a mesh does not erase the source value unless
the caller deliberately chooses that materialization boundary.

### Assemblies and part domains

An `Assembly` is a native structural value, not an implicit geometry
conversion. Structure-only operations such as placement, repetition, material
binding, and metadata edits must leave part geometry in its existing domain.

The current structure head admits two part sources: a constructive `Recipe`
and a materialized `Mesh`. A recipe-backed part remains a recipe after it is
registered and placed. `PartCompiler` evaluates it once per distinct content
and policy, and all of its instances share the compiled result. A mesh-backed
part declares that its materialization boundary has already been crossed.
Both source kinds may coexist in one assembly, and `exedra_ops::assembly`
patterns operate on their domain-neutral `PartId`s.

Other native heads do not become implicit `PartSource` variants merely to make
the enum appear comprehensive. Today they cross explicitly to a `Mesh` before
registration; the conversion's report remains visible to the caller, while
native editability and policy-driven re-evaluation end at that deliberate
boundary. This is an honest limitation, not the final heterogeneous authoring
model.

A future procedural network may build an assembly from typed artifact
references and realize those references under explicit policy when compiled.
That integration belongs above `exedra_assembly`, so the structure head does
not acquire a dependency on every geometry representation. It is earned when
a real assembly must retain a third native part kind; until then, the existing
recipe-or-mesh model remains smaller and clearer.

### CSG is domain-native

There is no single CSG algorithm hidden behind a lowest-common-denominator
interface:

- constructive CSG combines recipe intent and can preserve exact operations;
- polygon CSG edits or produces meshes;
- implicit CSG composes fields before extraction;
- a future surface/boundary-representation head may own analytic booleans once
  it has real surface and intersection semantics.

A future graph may route values through these operations, but changing domains
must remain an explicit node with a typed report. Operations are not assumed to
commute across a conversion.

### Future procedural network

Do not add an Exedra graph crate in this change. A procedural network is earned
only with an end-to-end consumer that connects two native value kinds through a
real conversion or realization edge and demonstrates incremental invalidation.
Its eventual package name follows that implementation instead of being
reserved here.

The surrounding Forest crates already establish two important ownership
boundaries:

- `execution_graph` owns dependency capture, dirty propagation, targeted
  execution, and execution-causality reporting. `execution_tape` is a useful
  backend for scalar expressions, authored control flow, and portable programs;
  it is not the definition of a geometry node.
- `understory_node_graph` owns the live authoring document, projections,
  sessions, routing, and hit testing. It intentionally owns neither execution
  nor application-specific node meaning.

A future Exedra integration layer owns only the geometry-specific node
vocabulary, typed ports, native value storage, conversion rules, domain
reports, and adapters into those two shared layers. It must not duplicate the
incremental scheduler or move geometry meaning into the editor model.

The current `execution_graph` accepts only `execution_tape` entry points. Before
heavy native geometry work uses it, the execution stack must provide a native
executor seam with the same dependency-access contract; wrapping every
expensive operation in a ceremonial one-host-call tape program is not the
long-term interface. Tape-backed and native nodes may coexist behind one
scheduler. This requirement changes the shared execution layer, not the
geometry kernels.

The first accepted slice must provide:

- a persistent authored node key, kept distinct from both the live
  `understory_node_graph` handle and the graph-local `execution_graph` node id;
  adapters own both mappings;
- typed input and output ports rather than an unstructured universal payload;
- nodes with several typed inputs and outputs: a Boolean consumes two values in
  one native domain, while a cage deformation can consume geometry, control
  geometry, weights, and a selection without forcing those values through the
  unary mesh-edit trait;
- one node definition as the source of its stable operation id, port schema,
  runtime binding, and editor metadata;
- values that retain native identity (`Mesh`, `Recipe`, field, shell,
  `Assembly`, and instances as their owners expose them);
- immutable cached artifacts at graph boundaries; mutable mesh operations use
  unique ownership or copy-on-write and never mutate a value shared by two
  branches;
- explicit conversion/realization nodes whose reports expose the evidence
  applicable to their contracts and record absent dimensions;
- deterministic cache keys, with every input declaring a native
  fingerprint/revision, caller epoch, or explicitly uncacheable status;
- expected geometry refusals represented as inspectable node outcomes rather
  than scheduler or virtual-machine faults;
- measured reused and recomputed work after a parameter change, with the
  incremental result equal to a clean rebuild.

The initial proof is deliberately small: construct a `Recipe`, evaluate it to
a `Mesh`, apply one `exedra_ops` edit, branch one upstream result to two
consumers, change one parameter, and demonstrate correct selective re-execution
and byte-identical clean-rebuild output. Project the same authored nodes through
`understory_node_graph` without making its handles runtime identities.

Private type erasure behind typed public ports or static composition are both
allowed. The Exedra integration must not require all domains to implement a
large common `Geometry` trait or expose a public universal geometry type.
Small capabilities such as bounds, transform, preview, or serialization may
be expressed independently where they are genuinely shared.

The current planar analytic head is not renamed into a general shell kernel.
If curved surfaces, trims, topology healing, and analytic intersections earn a
full boundary-representation head, that boundary and public contract will
receive a separate decision. Likewise, a persistent implicit scene model
should be introduced only when field composition needs identity beyond Rust
values.

## Alternatives considered

### `exedra_modeling`

Rejected because it makes one crate sound like the owner of all modeling
semantics. It would attract domain algorithms and become a dependency hub.

### Naming the current crate `exedra_graph`

Rejected for the current implementation because there is no graph. Naming the
crate after future ports, scheduling, and caches would preserve the existing
mirage.

### Immediate split into runtime and per-domain operation crates

Deferred. It could reduce dependency surface later, but the current adapters
are explicit and the workspace has not measured a compile-time or ownership
problem that justifies several public crates. A future split must follow real
dependency pressure rather than aesthetics.

### One universal geometry enum or trait

Rejected. It would either expose every domain through a constantly growing
central enum or flatten their distinct guarantees into weak shared methods.
Typed graph ports and explicit conversions provide composition without moving
ownership.

## Consequences

Positive:

- package names now describe one Exedra family;
- the existing mesh-edit SDK keeps its identity and remains useful;
- documentation no longer claims a cross-domain runtime that is not present;
- an unused metadata enum no longer stands in for typed composition;
- future network work has narrow execution, editor, and geometry boundaries
  plus an objective entry gate;
- constructive, implicit, polygonal, analytic, and assembly semantics remain
  independently replaceable.

Tradeoffs:

- direct users retain a small mesh-only default, while the `exedra` facade
  selects the common multi-crate application surface;
- a Houdini-like graph remains future work rather than a renamed facade.

This change adds no new production dependency, no `unsafe`, and no geometry
output change; it makes existing adapter dependencies optional at the
`exedra_ops` boundary.
