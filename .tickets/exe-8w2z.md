---
id: exe-8w2z
status: open
deps: [exe-dey4]
links: []
created: 2026-03-03T07:06:40Z
type: delete_faces kernel primitive
priority: P1
assignee: Bruce Mitchener
---
# Untitled

Add delete_faces(faces: &FaceSet, policy: DeletePolicy) -> ChangeSet as a kernel edit primitive. Needed for boolean cleanup, operator workflows, and general mesh editing. Policy controls what happens to newly exposed boundary edges and isolated vertices. Must go through the transaction system and produce a proper ChangeSet with dirty bits.

## Design

delete_faces removes the specified faces and their half-edges. Policy enum controls: (a) whether to delete isolated vertices left behind, (b) whether to leave boundary edges or also remove them. Deletion marks arena slots as dead (generation bump). The resulting ChangeSet records all removed elements. Must handle cascading: removing a face may leave dangling half-edges that need cleanup. Boundary model: removed face half-edges that had a twin on a surviving face get their twin pointed to OUTSIDE.

## Acceptance Criteria

Can delete a single face from a box, leaving 5-face open mesh. Can delete multiple faces. Isolated vertices optionally cleaned up. Resulting mesh passes validate_fast(). ChangeSet accurately records all deletions. Works through transaction system.

