---
id: exe-al9q
title: Txn face-creation kernel for edit operators
status: open
deps: []
links: []
created: 2026-03-04T07:43:37Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Txn face-creation kernel for edit operators

Add a transaction-level kernel to create interior faces from existing topology endpoints so higher-level edit operators (extrude/inset) can build side walls and frames without bypassing Txn bookkeeping.

## Design

Current Txn primitives (split_edge/split_face/delete_faces) cannot create arbitrary new interior face loops. Add a deterministic Txn API that can stitch a new face loop from canonical live corners/vertices, updates twin/next/face pointers, syncs attrs, and records dirty/created entities. Must preserve manifold invariants and fail precondition checks before mutation when possible.

## Acceptance Criteria

- New Txn face-creation API exists and is documented
- Supports building side-wall faces needed by extrude/inset
- Precondition validation covers stale/non-manifold/invalid loops
- Records created entities + dirty channels deterministically
- validate_fast and validate_deep pass on exercised paths
- Unit tests include success + failure cases
