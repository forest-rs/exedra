# Retained plane operations

Status: accepted

`PlaneCut` retains an explicitly named half-space of every completely evaluated
child body, using `split_body`. `ExtrudeToPlane` uses `extrude_to_plane`. These
nodes introduce no new geometry algorithm or broader solid-validity claim.

Both operate in node-local coordinates. A cut evaluates child transforms first;
an extrusion places its profile relative to its target plane. Ancestor transforms
apply to the completed local body. `EvalPolicy::section` specifies local-unit
accuracy and per-body work budgets. Reflections preserve outward winding through
the existing checked placement adapter. Close contacts, unsupported topology,
and work limits remain typed failures. A disjoint retained half is empty.

Child diagnostics are replayed before cut cache lookup. An incomplete child
cannot become a successful partial cut. Nonempty singleton results use the
existing body cache; multi-body and empty cuts reuse child evaluations without
pretending the body cache can store a collection. Inherited materials remain
outside geometry cache keys; explicit cap overrides and face provenance survive.

JSON and canonical text retain the authored operations. Schema 32 invalidates
prior evaluation fingerprints. Existing node constructors are unchanged; callers
should initialize `EvalPolicy` from its default before overriding section limits.
