# ADR 0002: Make Addressable the native assembly identity and query API

- Status: accepted
- Date: 2026-08-27
- Ticket: `ea-q7tb`

## Context

The first integration placed a complete Addressable evaluator in a separate
`exedra_assembly_addressable` crate. It mapped Exedra's `InstancePath` into
another exact-address representation and left `Assembly::path_of`,
`Assembly::resolve_path`, public material resolution, and Basilica-specific
selection helpers intact. The boundary was structurally clean but made the
system larger without replacing the narrower API.

The consumer evidence supports a different boundary. Exact-address rooted
trees share resolution, pinned-reference, query-policy, and handle semantics;
assembly storage, part identity, metadata predicates, and material policy remain
domain-owned. Addressable ADR 0002 therefore introduces the dependency-light
`addressable_tree` runtime for the shared behavior.

## Decision

`exedra_assembly` uses Addressable directly:

- every `Instance` stores an `AbsoluteAddress<AssemblySpace>` and `Assembly`
  indexes that structured address to its runtime-local `InstanceId`;
- instance keys are validated as Addressable `Name` segments at insertion;
- `Assembly` implements `addressable_tree::TreeHost`, projecting the synthetic
  `/` root, part referents, instance-address occurrences, metadata predicates,
  parent/child relationships, and optional `InstanceId` handles;
- `AddressableAssembly` is a thin domain wrapper over `TreeRuntime<Assembly>`.
  Exact, relative, and pinned resolution, cardinality-shaped queries, ordering,
  deduplication, cycle policy, traversal budgets, and revision-scoped handles
  execute in Addressable's runtime;
- material explanations stay in `exedra_assembly`, because they depend on
  Exedra's instance-over-part policy;
- post-binding structural, metadata, and part-content mutations pass through
  `AddressableAssembly::commit` and advance the runtime revision.
  `Assembly::content_generation` remains a narrower geometry-cache invalidation
  counter, not a second observation revision;
- extraction returns the runtime revision with the `Assembly`, and `resume`
  restores that clock instead of silently restarting it.

Guarded material transactions are deliberately deferred. They add a new
authoring protocol rather than replacing an existing Exedra API, and their
revision, guard, atomicity, and duplicate-target validation overlaps the
reference implementation in Addressable. That shared mutation seam should be
designed once a concrete editing consumer requires it.

There is no separate integration crate. Geometry compilation and flattening
still operate on `Assembly`; the Addressable wrapper exposes immutable access
so compilation can consume the current domain state without bypassing revision
tracking.

## Consequences

- Exedra deletes its custom path type, recursive path construction and lookup,
  consumer-local tree evaluator, and public effective-material shortcut.
- `Assembly::instance_by_address` exposes the existing address index to callers
  that need a runtime handle without scanning instances.
- Basilica deletes public exact-path and role-selection helpers; callers use
  exact locators and typed metadata or part queries.
- Exact addresses are canonical structured values in memory. Text beginning
  with `/` is parsing and serialization, not the storage representation.
- `PartId` and `InstanceId` remain runtime accelerators. A resolved handle is
  revision-scoped and never becomes durable identity.
- More capable live or mutation infrastructure moves into Addressable only when
  another concrete consumer deletion proves the shared seam.

This is semantic subtraction, not a net source-code reduction. The branch adds
the reusable tree projection, read-side material explanation, executable tour,
migration coverage, and docs while deleting narrower path and selection
machinery. At adoption time it is roughly 1,400 net lines: about 870 lines of
projection, vocabulary, and explanation; 280 lines of executable tour; and the
remaining documentation, lockfile, and consumer migration. `assembly.rs`
itself removes 46 more lines than it adds. Example exporters still remove the
canonical leading slash when preserving their established OBJ and glTF names;
that presentation conversion is not part of address identity.

## Migration note

- Replace `InstancePath` with `InstanceAddress`; obtain it from
  `Instance::address` or parse a canonical exact address such as `/root/child`.
- Replace `Assembly::path_of` and `Assembly::resolve_path` with
  `Assembly::into_addressable`, `AddressableAssembly::locator`, and Addressable
  resolution or query methods.
- Use `Assembly::instance_by_address` when an indexed `InstanceId` lookup is all
  that is required.
- Replace `RenderItem::path` with `RenderItem::address`.
- Replace `Assembly::resolved_material` reads with a typed `MaterialSlot`
  endpoint and `AddressableAssembly::read_material`. Construction code that
  already owns an instance may inspect its authored binding and part default.
- Instance keys `.`, `..`, strings containing `/`, NUL, or the empty string are
  now rejected because each key is one Addressable name segment.
- glTF extras now use `instanceAddress`, and the Cambium assembly bridge uses
  `instance_address`; both carry canonical leading-slash text.
- `AddressableAssembly::into_inner` now returns `(Revision<AssemblySpace>,
  Assembly)`. Restore that pair with `AddressableAssembly::resume` rather than
  rebinding the same space id with `new`.
- The provisional `BindMaterial`, `EditCapability`, `TransactionReport`, and
  `TransactionConflict` APIs from development revisions are not part of this
  slice. Existing authoring methods remain available through
  `AddressableAssembly::commit`; a guarded editing API is deferred.
