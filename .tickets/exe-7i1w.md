---
id: exe-7i1w
status: closed
deps: []
links: []
created: 2026-09-02T15:14:47Z
type: bug
priority: 1
assignee: Bruce Mitchener
---
# Support curved-on-curved mesh intersections

Intersection of two equal tessellated unit spheres translated to x = -0.8 and x = 0.8 is refused because split chains terminate inside curved faces.

## Design

Fix the mesh Boolean splitter or intersection graph invariant that produces dangling face cuts. Preserve typed refusal for genuinely unsupported topology and validate the fix against analytic and isosurface membership oracles.

## Acceptance Criteria

The two-sphere intersection returns one deterministic closed deep-valid body with outward winding, correct bounds and lens volume, no pipeline deferral, and no regression in union or difference. Nearby offsets and tessellation densities are covered.


## Notes

**2026-09-02T16:27:12Z**

Implemented deterministic seam identity recovery from exact `MeshAnchor` incidence. The graph keeps provenance and exact stored-position matches as its first choices, then reconciles edge/edge with reciprocal edge/face observations without a geometric epsilon. Late class propagation is limited to identical stored positions so valid branches at multi-cut vertices remain distinct; parallel ambiguity remains a typed refusal. Updated ADR 0010. Coverage includes the fixed curved lens, offsets and tessellation densities across both triangulation modes, determinism and operand order, Boolean volume identities, constructive evaluation, and a three-witness analytic/field oracle sweep. Validation: `typos`; `cargo fmt --all -- --check`; `taplo fmt --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `cargo doc --workspace --all-features --no-deps`; 200 oracle cases with zero validation, seam, mesh-membership, or field-membership disagreements.

**2026-09-04T03:30:00Z**

CI's `libm` coordinates exposed an observation-order case not produced by the host math backend: an edge/edge endpoint matched one existing class by stored position and therefore returned before its reciprocal incidence could join an adjacent representable class. Matching now considers the same observation's incidence proofs, while an anchor-specificity guard keeps an established edge/edge branch from being swallowed by a differently rounded edge/face match. A focused three-observation regression traps the ordering and the existing multi-cutter sweep traps the guard. Curved fixtures select the same explicit math backend as primitive generation under unified features, and the faceted-volume assertion admits the input's `f32` rounding band. Validation: `typos`; `cargo fmt --all -- --check`; `taplo fmt --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `cargo test --exclude cambium_web_bridge --workspace --locked --all-features --target wasm32-wasip1 --no-fail-fast`; `cargo doc --workspace --all-features --no-deps`.
