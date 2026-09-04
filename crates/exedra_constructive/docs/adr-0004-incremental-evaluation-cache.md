# ADR-0004: Incremental evaluation cache

## Status

Accepted (M5, cam-ip50 / ec-7538).

## Context

The whole fingerprint architecture (ADR-0001, `EVAL_SCHEMA_VERSION`,
Merkle node fingerprints) exists so that re-evaluating an edited recipe
can reuse everything the edit did not touch. M5 lands that reuse:
`evaluate_with_cache` consults a caller-owned `EvalCache` at every
body-producing node and must be **bit-identical** to the pure `evaluate`
— same bodies, same source maps, same report, differing only in the
work counters that honestly describe what each run did.

## Decision

### Key: `(node fingerprint, world placement bits, policy fingerprint)`

- The **node fingerprint** is the recipe's Merkle content hash: it covers
  the node's parameters, its profiles/imports, and its whole subtree.
- The **world placement** (12 exact `f64` bit patterns) participates
  because tessellation composes the world placement into f64 vertex
  construction *before* the single documented f32 narrowing. Caching
  local-space bodies and re-transforming on hit would narrow twice and
  break bit-identity — the one contract this cache is not allowed to
  bend. Bit-exact placement comparison is conservative: at worst it
  misses (recomputes); it can never wrongly hit.
- The **policy fingerprint** folds every `EvalPolicy` field's exact bits
  with `EVAL_SCHEMA_VERSION`, so discretization changes and schema bumps
  re-key the world instead of trusting stale bodies.

Invalidation is therefore not a mechanism but an identity: an edit
changes fingerprints on exactly the path from the edited node to the
root, and those keys simply never hit again. There is no dirty
propagation step to get wrong and no way to observe a stale entry.

### What is cached

Body-producing nodes: `Extrude`, `Revolve`, `Loft`, `Sweep`,
`GridSurface`, `MeshImport`, and the **successful result of a `Csg`
node** (the expensive boolean pipeline). `Stretch` caches exact rewrites and
single-body unmapped-mesh results; a mesh stretch carrying corner UVs is not
cached yet because UV-extension counters/diagnostics are part of the report
while the cache value currently stores only the body. Cached values are
`Rc<TessellatedBody>`; `PlacedBody::body` became `Rc<TessellatedBody>` so
hits are zero-copy (a deliberate public-shape break: long-term core
shape over caller compatibility; consumers read through `Deref`).

Not cached, deliberately:

- **CSG failures** (envelope fallbacks). Their diagnostics must replay
  honestly on every evaluation, and caching refusals would hide the
  pipeline's own improvement between runs. Failure paths re-run.
- **Instance instantiation** (the per-instance rigid transform of a
  definition body). The definition's bodies hit the cache at identity
  world; the O(vertices) transform per instance is cheap relative to
  tessellation. A follow-up may key instantiated bodies if profiling
  warrants it.
- **Intermediate n-ary CSG union folds.** Only the final boolean result
  is keyed; a recipe with many-operand CSG nodes re-folds tails on a
  changed sibling. Nested `Csg` nodes each get their own entry, which
  covers deep trees.

On a `Csg` hit the operand subtrees are still walked (their bodies hit
the leaf cache), so the report — fidelity entries, policy attributions,
diagnostics — is identical with and without the cache. This also means
a run can hit entries inserted earlier in the *same* evaluation: a node
referenced both directly and as a CSG operand or instanced definition
tessellates once (tested).

### Eviction and measurement

Deterministic LRU: entries carry `(generation, sequence)` stamps
(generation advances once per evaluation, sequence per touch); the
least recently touched entry is evicted first past the explicit entry
capacity, via a `BTreeMap` recency index — no hash-order iteration is
observable. Counters expose hits, misses, insertions, evictions, live
entries, and approximate retained bytes (documented as an estimate,
not an allocator measurement).

### Why the `invalidation` crate is not used inside `EvalCache`

The `invalidation` crate models *channel-based dirty marking* for
identities that are not content-addressed (mesh element ids, assembly
part slots — see `exedra_assembly::PartCompiler` and Exedra Ops' dirty
channels, which use it correctly). Recipe evaluation identity *is*
content-addressed: the Merkle fingerprint already encodes exactly the
ancestor-path invalidation a dirty channel would approximate, with
proof instead of bookkeeping. Wiring a dirty channel here would
duplicate the fingerprints' guarantee and add a second source of truth
that could disagree with the first.

### Threading

`Rc` keeps the cache and evaluation output single-threaded, matching
the existing instance cache. The threadability model for the whole
operator stack is cam-ezlm's decision; switching to `Arc` is mechanical
when that lands.

## Consequences

- A single-parameter edit re-tessellates exactly the changed node (and
  any `Csg` ancestors), asserted by counter tests and the CT-2 wind
  tunnel; output is bit-identical to a full rebuild (signature oracle).
- `EvalCounters` gained `cache_hits`/`cache_misses`; `tessellations`
  now counts only real tessellation work, so goldens (which pin only
  `bodies` and `envelope_only`) are unaffected.
- Frontends own cache lifetime and capacity explicitly; nothing is
  global and nothing is implicit.
