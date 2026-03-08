// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

# ADR-0001: Primitive feature edges are authored sharp by default

## Status

Accepted

## Context

Primitive meshes are used directly in demos, tests, and modeling flows. Relying
on angle-based normal derivation alone produces the wrong default shading
contract for semantically obvious feature boundaries:

- boxes should shade with hard outer edges,
- capped cylinders should shade with smooth sides and hard cap rims,
- capped cones should shade with smooth sides and a hard bottom rim.

An angle-only fallback is not stable enough as the primary contract because the
expected shading behavior is semantic, not merely geometric.

## Decision

Primitive constructors author explicit edge sharpness where the feature boundary
is semantically obvious:

- `box_primitive`: edges between different side regions are sharp,
- `cylinder`: top and bottom cap rim edges are sharp when caps exist,
- `cone`: bottom rim edges are sharp when a cap exists.

Side seams remain smooth unless another contract marks them sharp explicitly.

## Consequences

- Default render extraction produces the expected modeling-style shading without
  depending on `auto_sharp_angle_degrees`.
- Primitive shading behavior is more explicit and stable across geometry
  variations.
- Consumers that want a different shading contract can still clear or override
  authored sharpness after construction.
