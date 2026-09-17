# ADR 0001: Keep the application facade leaf-only

## Status

Accepted.

## Context

Applications need one convenient dependency and a clear path to the major
modeling domains. Those domains have different state, evaluation, and
conversion rules, so a convenience crate must not become an owner of those
rules.

## Decision

`exedra` is a leaf-only facade.

**Fence:** This crate owns suite-level feature selection, curated namespaced
reexports, root anchors, and end-to-end documentation; it explicitly does not
own geometry algorithms, persistent model state, scheduling, or conversion
semantics.

Dependency direction is from the facade to the domain crates. The mesh kernel
is `exedra_mesh`; the constructive, assembly, mesh-operations, edit, analytic,
isosurface, and export crates remain independent owners of their APIs. No
domain crate may depend on this facade.

The always-present `mesh` namespace exposes the mesh kernel. Optional
namespaces map one-to-one to their owners: `constructive`, `assembly`, `mesh_ops`, `edit`,
`edit`, `analytic`, `isosurface`, `primitives`, and `gltf`. The root exposes only
`Mesh`, plus `Recipe` with `constructive` and `Assembly` with `assembly`. Each
namespace is a direct crate reexport, not a second manually mirrored API
surface.

The default features are `std`, `assembly`, and `mesh_ops` because they describe
the common host application. `assembly` selects `constructive`: assembly's
public part source admits recipes, so making that relationship explicit avoids
an incoherent feature surface. `edit`, `analytic`, `isosurface`, `primitives`, and
`gltf` are opt-in; `gltf` selects `assembly` and `std`. `libm` is the
alternative backend for `no_std` consumers. Interchange remains behind
`serde`, which selects `std` and the constructive and assembly serialization
features. The runner is opt-in as `edit`; direct mesh operations use `mesh_ops`.
Native-head adapters are removed: conversions and evaluation live in their
owning crates. See the [consolidation decision](../../exedra_mesh_ops/docs/adr-0001-mesh-operation-boundary.md)
for migration from the former `ops` feature and namespace.

## Consequences

The facade gives applications short, stable imports without becoming a
cross-domain coordinator. Feature forwarding is explicit and does not add new
third-party dependencies.

The facade is curated rather than exhaustive. Specialist support libraries,
backend adapters, test utilities, and domain-specific construction layers stay
as direct dependencies instead of becoming namespaces merely because they are
workspace members.

An extension belongs here only when it can be expressed as a documented
reexport, feature relationship, or root anchor with no new state or behavior.
Any algorithm, state machine, scheduler, or conversion policy belongs in an
owning domain crate first; the facade may then expose it in that crate's
namespace.
