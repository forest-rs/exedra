# Brief: Math types in Exedra core (keep it minimal and deterministic)

## Decision
Exedra core stores geometry in plain arrays (`[f32; 3]` for positions, `[f32; 2]` for UVs) and provides a small internal math module for the necessary vector operations. Avoid depending on a large math crate in the kernel core.

## Why
Exedra is a kernel: dependency surface and determinism matter more than ergonomics.

- `[f32; N]` is ubiquitous and stable across crates and FFI.
- A tiny math module gives full control over tie-breaking, epsilons, and normalization behavior.
- Large math crates may introduce SIMD-vs-scalar differences and broaden dependencies.

## Alternatives considered
- **Use a full math crate (e.g., glam) in the core**: ergonomic and fast, but expands dependency surface and can complicate strict determinism expectations.
- **Custom newtypes everywhere**: adds boilerplate without much benefit over `[f32; N]` at this stage.

## Implications
- Exedra Ops and higher layers may use a math crate internally, but write back to Exedra using `[f32; N]`.
- Any transcendental usage (sin/cos/atan2) should live above the core or be explicitly policy-controlled.

## Non-goals / deferrals
- SIMD optimization in v0.1; focus on correctness, determinism, and cache-friendly storage first.
