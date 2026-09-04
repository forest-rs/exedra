# Brief: Staged booleans with artifacts (debuggability is the product)

## Decision
Boolean operations are implemented as an explicit staged pipeline (broad phase → narrow phase → graph → split → classify → stitch). Failures return structured **artifacts**, not just an error string.

## Why
Booleans fail in the real world—coplanar ambiguity, numerical instability, non-manifold inputs. If failures are opaque, you can't improve them. Artifacts make failures:

- reproducible
- inspectable (segments, suspect regions, stage stats)
- actionable (which stage failed, why)

Staging also provides performance clarity (wind tunnels can measure stage breakdowns).

## Alternatives considered
- **Monolithic boolean function**: simpler interface, but opaque and hard to debug or optimize.
- **Return only a boolean “success”**: guarantees frustration.

## Implications
- Exedra defines a failure taxonomy (kinds) and artifact structures with deterministic ordering.
- Scratch buffers are reused across stages to avoid allocations.
- Intermediate artifact sizes must be bounded/streamable for large meshes.

## Non-goals / deferrals
- Handling every degeneracy in v0.9/v1.0; early versions can reject some cases with clear diagnostics.
