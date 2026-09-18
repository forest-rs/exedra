# Construction failure context

Assembly owns part keys; constructive owns recipe sources, loft sections and
geometric refusal evidence. Errors own their resolved labels so inspection does
not require keeping the assembly or recipe alive. Numeric IDs remain alongside
labels, which are opaque author references and need not be unique.

Retained loft sections are explicit records with an optional source reference.
Validation and lowering treat those references like node sources. Labels
participate in content identity, resolved by string rather than table index.
Unlabeled loft fingerprints and successful geometry remain unchanged. JSON v1
keeps its section pairs and adds an optional parallel `section_sources` array;
when nonempty it must have exactly one entry per section. The text format adds
an optional `section_source` suffix to each section. Both forms round-trip labels.

Smooth-loft refusal captures the cubic trajectory and the control edge whose
advance along the band secant is nonpositive. This is evidence that the existing
sufficient forward-motion check failed, not proof of a surface self-intersection.
Witness coordinates are the coordinates supplied to tessellation; retained
evaluation uses the current construction space (including applicable transforms).
The outgoing sampled edge at the trajectory point identifies each section's
authored segment/tag. No second discretization or guessed correspondence is used.

## Migration

Replace `(placement, profile)` entries in `NodeKind::Loft::sections` with
`LoftSection::new(placement, profile)`. Add `.with_source(source)` for authored
section labels. The immediate tessellation API still accepts placed profile pairs.

`EvalError` adds owned source and optional section context. Assembly compilation
errors add the part key and box the evaluation payload to keep result sizes bounded. `LoftError::Foldover` adds a boxed witness and no longer
implements `Eq` because the evidence contains floating-point coordinates. Match
additional fields with `..` when only the error category matters.

No geometry algorithm or interpolation policy changes. Shared patch boundaries,
construction UV charts, renderer batching and domain-specific construction rules
remain separate work.
