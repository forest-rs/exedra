# ADR-0003: OUTSIDE Face Representation Is a Sentinel

**Status:** Accepted  
**Date:** 2026-03-03

## Context

Exedra's boundary model uses explicit boundary half-edges and a reserved OUTSIDE
face region. Before implementing `Mesh` storage/traversal, we need to decide
whether OUTSIDE is:

1. A real `Face` arena entry, or
2. A sentinel `FaceId` value not stored in the face arena.

This decision affects traversal, validation, and rendering/extraction behavior.

## Decision

Use a **sentinel `FaceId::OUTSIDE`** that is **not** stored as a `Face` arena
entry.

`FaceId::OUTSIDE` is a reserved compile-time ID (`index = u32::MAX`,
`generation = NonZeroU32::MIN`) and is handled explicitly in boundary-aware
code paths.

## Rationale

- Keeps face arena iteration purely over authored/internal faces.
- Avoids introducing a pseudo-face record with ambiguous `degree` semantics for
  disconnected boundary loops.
- Aligns with current stable-ID implementation (`exe-dc9l`) and keeps the
  representation explicit.
- Preserves determinism: OUTSIDE never participates in normal face ordering.

## Consequences

- Boundary-aware traversal/validation must branch on the OUTSIDE sentinel where
  needed.
- APIs that enumerate "all faces" should define whether they exclude OUTSIDE
  (default behavior for arena iteration).
- `exe-cbv1` and validation/extraction tickets must follow this representation.
