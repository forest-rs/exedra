# ADR-0005: Eager Edit Scopes Use Optional Change Sinks

**Status:** Accepted  
**Date:** 2026-03-08

## Context

Exedra previously exposed `Mesh::begin()` and `EditSession::commit()`.
That naming overstated the semantics:

- edits already applied eagerly to the mesh during the session,
- dropping the session did not roll anything back,
- every session paid the cost of recording change summaries even when the
  caller did not need a `ChangeSet`.

This made the public model less honest than the implementation and forced
mandatory bookkeeping into preview and throwaway edit paths.

## Decision

Adopt eager edit-scope terminology and make change recording explicit.

- `Mesh::edit()` starts an eager edit scope with no change recording.
- `Mesh::edit_with(sink)` starts an eager edit scope with explicit recording.
- `EditSession::finish()` closes the scope, increments mesh revision, and
  returns the sink output.
- `ChangeSetBuilder` is the standard sink for callers that need a deterministic
  Exedra `ChangeSet`.
- `DiscardChanges` is the default sink used by `Mesh::edit()`.

`EditSession` remains eager: dropping a session never rolls back mesh changes.

## Rationale

- The API now matches the actual semantics.
- Preview and one-off edit paths can avoid unnecessary `Vec` pushes and dirty
  bookkeeping.
- Change recording becomes an explicit choice instead of an implicit tax.
- Cambium can request a `ChangeSet` only in the runner path that actually uses
  it.

## Consequences

- Public Exedra code should talk about edit scopes and finishing, not
  transactions and commits.
- Callers that need a `ChangeSet` must opt in with `ChangeSetBuilder`.
- Internal mutation code stays generic over `ChangeSink`, so kernel ops work
  with either recorded or unrecorded edit scopes.
