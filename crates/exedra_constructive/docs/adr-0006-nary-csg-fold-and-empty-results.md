# ADR-0006: N-ary CSG folds and exact empty results

## Status

Accepted (ec-55ov).

## Context

`CsgOp` promises union and intersection across every operand, while difference
means the first operand minus the union of the rest. Evaluation previously
unioned operands 2 through n for every operation before applying the requested
operation to operand 1. For intersection this evaluated
`A intersection (B union C ...)`, so three or more operands could emit geometry
outside later operands and still report `Exact`.

The mesh Boolean layer already treats an empty intersection as a successful
zero-face mesh. Constructive evaluation must preserve that distinction from a
pipeline refusal so an empty intermediate can participate in an outer CSG node,
be cached, and reach assembly and export without being replaced by an operand
or mislabeled as envelope-only.

## Decision

- Union and intersection fold from left to right using their own operation at
  every step. Binary behavior is the same fold with one step.
- Difference continues to union operands 2 through n, then subtracts that
  single tail from operand 1.
- Each intermediate mesh carries an ID-keyed face-to-operand table. Boolean
  face provenance composes through that table after every step, so the final
  `Feature::BooleanFace` names the originating CSG operand even when an earlier
  intermediate supplied the face. IDs, rather than face counts, support meshes
  whose arenas contain tombstones.
- A successful empty Boolean remains one `Fidelity::Exact` tessellated body
  with zero faces and vertices. It is cacheable and remains a real operand for
  nested CSG. A Boolean error still produces the existing explicit
  `EnvelopeOnly` report and diagnostics.
- Assembly retains the empty compiled body and instance identity. glTF export
  retains the instance node, transform, and metadata without attaching a mesh;
  when no geometry remains, it omits the optional mesh, accessor, buffer-view,
  and buffer arrays and emits no GLB binary chunk.

Because an unchanged recipe can now produce different geometry and face
provenance, `EVAL_SCHEMA_VERSION` advances from 10 to 11. Recipe fingerprints,
evaluation caches, assembly compilation caches, and schema-stamped text output
therefore invalidate together.

## Consequences

- Intersection is associative at the evaluator boundary in the intended set
  semantics. Operand permutations preserve bounds and volume for supported
  inputs; mesh ordering remains an implementation detail.
- Empty common volume is observable as successful exact geometry throughout
  evaluation, nesting, caching, assembly compilation, and export.
- Intermediate folds still use the Boolean pipeline's typed refusal contract.
  A configuration the pipeline cannot decide remains envelope-only rather than
  being treated as empty or partially accepted.
