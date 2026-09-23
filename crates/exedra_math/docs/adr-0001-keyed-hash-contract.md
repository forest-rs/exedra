# ADR-0001: Keyed hash contract

**Status:** Accepted
**Date:** 2026-09-23

## Context

Procedural generators built on Exedra (sylva's trees, dapple's material
fields) need randomness that is deterministic and *local*. If they draw from
one sequential random stream, adding or removing a single draw reshuffles
every later decision. Editing one branch angle would then move every leaf,
and incremental regeneration and art direction stop working.

Sylva and dapple each implemented the same counter-based scheme with identical
golden vectors, pending a shared home. Keeping two copies invites drift in a
contract whose whole value is that it never drifts.

## Decision

`exedra_math::keyed` owns version 1 of the keyed hash contract:

- `splitmix64_mix`: the SplitMix64 finalizer.
- `mix(h, k)`: `splitmix64_mix(h ^ k * 0x9E3779B97F4A7C15)`, in wrapping
  arithmetic.
- `hash(seed, keys)`: a left fold of `mix` over the keys, starting from
  `seed`.
- `unit_f32` / `unit_f64`: the top 24 / 53 bits scaled into `[0, 1)`. The
  conversions are exact.
- `tag(name)`: 64-bit FNV-1a over the name's UTF-8 bytes (offset basis
  `0xcbf29ce484222325`, prime `0x100000001b3`), for named purpose keys.
- `Key`: a builder with `with`, `bits`, the unit conversions, and
  `range_f32` / `range_f64`, which compute `lo + (hi - lo) * unit` in the
  target precision as subtract, multiply, add, never fused.

The module documentation states the full formulas and golden vectors, and
tests pin them.

1. **The outputs are frozen.** Everything listed above returns the same bits
   for fixed inputs on every platform, backend (`std` or `libm`), and release,
   across crate major versions. The integer functions use wrapping arithmetic
   and exact conversions; the ranges use single IEEE operations in a fixed
   order, so no float backend can change them.
2. **What may change, and what may not.** Adding functions, derives, trait
   impls, or documentation is allowed. Any change to an output bit is
   breaking: the mixer or its constants, the fold direction, which hash bits
   feed the floats, their scaling or rounding, the range arithmetic or its
   order, and the tag algorithm. A breaking change ships as a new
   `keyed::v2` module beside version 1, which stays unchanged. Consumers'
   generated content depends on these bits, so an in-place change would
   silently alter every seeded asset.
3. **Keys carry meaning, not the module.** Callers choose their key layout:
   stable element IDs, purpose tags, ordinals. The module does not interpret
   keys. Integer keys are derived by the caller, losslessly (for example
   `i64::cast_unsigned`), and names through `tag`. `mix(h, k)` is zero
   whenever `h == k * 0x9E3779B97F4A7C15`, including `mix(0, 0)`; leading
   with a nonzero purpose tag makes such collisions unlikely, not impossible.
4. **Helpers stay minimal.** The module adds only what Exedra itself needs:
   unit values, ranges, and tags. Consumer-specific helpers, such as signed
   unit values or unit-vector and disk sampling, live in the consumer as free
   functions or extension traits over `Key` until a second consumer needs the
   same one.

## Consequences

- Sylva and dapple can replace their copies with this module and keep their
  golden vectors unchanged, including sylva's tags.
- Exedra code that needs jitter or seeded variation has one deterministic
  source.
- The contract cannot improve in place. That is intentional.
