---
id: exe-xs3x
status: closed
deps: []
links: []
created: 2026-03-07T18:30:51Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Replace begin/commit with edit/finish and optional change sink

Replace eager edit-session lifecycle naming with Mesh::edit()/EditSession::finish(), make change recording optional via an explicit sink/builder, and update session internals/call sites accordingly.

## Design

EditSession remains the eager mutation host. Mesh::edit() creates a session without change recording. Mesh::edit_with(&mut sink) creates a session that records changes through a sink interface. finish() closes the scope; ChangeSetBuilder becomes one sink implementation that produces ChangeSet. No transition compatibility layer; remove begin()/commit() directly.

## Acceptance Criteria

Mesh::begin and EditSession::commit are removed. Mesh::edit and Mesh::edit_with exist. EditSession::finish replaces commit. ChangeSet is produced through ChangeSetBuilder rather than mandatory session-owned vectors. Exedra/Cambium/tests/docs all build and use the new API.

