## Scope

- Add a first-slice Cambium bridge operator for two explicit boundary loops.
- Keep the implementation on top of the existing patch/loop substrate.
- Add fluent support and focused demo-quality tests.

## Non-goals

- No loop resampling.
- No unequal-count bridging.
- No generalized loft/path sweep behavior.
- No Exedra public API expansion unless a real kernel gap appears.

## Steps

1. Define deterministic loop-input contract and typed operator surface.
2. Add shared loop-pair alignment/orientation helpers in `patch`.
3. Implement bridge face generation and output typing.
4. Add fluent API support and documentation.
5. Validate ring-to-ring and hole-to-hole style cases.

## Risks

- Loop orientation can silently flip winding if alignment is underspecified.
- Boundary selection identity must remain deterministic across identical mesh state.
- Sharpness/UV defaults on generated bridge faces should stay explicit rather than accidental.

## Validation

- `typos`
- `cargo fmt --all`
- `taplo fmt`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --no-deps`
