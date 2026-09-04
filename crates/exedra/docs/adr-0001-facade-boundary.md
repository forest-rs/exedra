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
is `exedra_mesh`; the constructive, assembly, operations, analytic,
isosurface, and export crates remain independent owners of their APIs. No
domain crate may depend on this facade.

The always-present `mesh` namespace exposes the mesh kernel. Optional
namespaces map one-to-one to their owners: `constructive`, `assembly`, `ops`,
`analytic`, `isosurface`, `primitives`, and `gltf`. The root exposes only
`Mesh`, plus `Recipe` with `constructive` and `Assembly` with `assembly`. Each
namespace is a direct crate reexport, not a second manually mirrored API
surface.

The default features are `std`, `assembly`, and `ops` because they describe
the common host application. `assembly` selects `constructive`: assembly's
public part source admits recipes, so making that relationship explicit avoids
an incoherent feature surface. `analytic`, `isosurface`, `primitives`, and
`gltf` are opt-in; `gltf` selects `assembly` and `std`. `libm` is the
alternative backend for `no_std` consumers. Interchange remains behind
`serde`, which selects `std` and the constructive and assembly serialization
features. When the `ops` feature and an adapter-bearing native head are both
enabled, the facade forwards the corresponding adapter feature to the
operations crate.

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
