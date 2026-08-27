---
id: ea-q7tb
status: closed
deps: []
links: []
created: 2026-08-26T16:55:16Z
type: feature
priority: 1
assignee: Bruce Mitchener
---
# Expose Exedra assemblies through Addressable

Make Addressable the native identity, resolution, and query API for
`exedra_assembly` while preserving stable part and occurrence identity,
revision-scoped handles, and material explanations.

## Design

Store `AbsoluteAddress<AssemblySpace>` directly on each instance and index it
inside `Assembly`. Implement the small domain projection required by the
consumer-derived `addressable_tree` runtime, and keep Exedra's read-side
material explanation policy inside `exedra_assembly`. Remove the separate
integration crate and the custom path resolution/query machinery.
Pin Addressable through the exact HTTPS Git revision that contains the shared
tree runtime.

## Acceptance Criteria

The native API resolves exact, relative, and pinned instance locators; executes
explicit child/descendant queries; consumes revision-scoped `InstanceId`
handles; explains instance-binding versus part-default material values;
documents the public workflow; and is exercised by the Basilica and top-level
tour. `InstancePath`, recursive path lookup, duplicate tree execution, and
Basilica-specific public selection helpers are removed.


## Notes

**2026-08-26T17:12:49Z**

The initial separate adapter proved the semantic workflow but failed the
subtraction test: it left Exedra's narrower path and selection APIs intact.
Consumer evidence pulled the reusable rooted-tree evaluator into Addressable.
Exedra now stores structured addresses, projects its own nodes and predicates,
and delegates locator/query semantics directly to `addressable_tree`.
Material explanations remain domain-owned, while guarded material transactions
are deferred until a concrete editing consumer can shape the shared validation
seam. The same revision gateway covers later structural, metadata, material,
and part-content authoring. The public migration is recorded in assembly ADR
0002.
