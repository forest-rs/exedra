---
id: cam-uox0
status: closed
deps: []
links: []
created: 2026-03-08T13:16:43Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Fix face-edit winding consistency for inset/extrude outputs

Quad/inset normals flip relative to the source face, and closed-form face-edit output winding needs regression coverage. Verify frame/inner/cap face orientation is consistent with the source region and fix the helper/orientation logic accordingly.

## Design

Add regression tests that compare face normals before and after inset/extrude on simple planar inputs and inspect generated face winding on closed primitives. Fix the frame/inner/cap loop construction to preserve source orientation independent of boundary-reuse direction.

## Acceptance Criteria

1. Insetting a quad preserves the source face normal direction on the generated inner face. 2. Extruding/insetting selected faces do not flip cap/inner face orientation on planar inputs. 3. Relevant regressions exist in cambium tests. 4. Full workspace quality gates pass.

