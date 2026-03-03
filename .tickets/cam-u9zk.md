---
id: cam-u9zk
title: Select faces by region (select.faces.by_region)
status: open
deps: [cam-ul4v, cam-ibof]
links: [cam-ul4v]
created: 2026-03-03T06:33:27Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Select faces by region (select.faces.by_region)

Implement select.faces.by_region: produce a canonical FaceSet of all faces matching a given region id. This is the counterpart to tag.face.region and is essential for composing operators in the pipeline (e.g. "extrude all REGION_FOOTPRINT faces", "UV project REGION_WALL_OUTER faces").

## Design

Reads the face-domain region id layer and collects all faces matching a target region id into a canonical Vec<FaceId> (sorted, deduplicated).

This may be a pure operator (no Txn needed — read-only) or a utility function. Either way it must:
- Iterate faces in deterministic arena order
- Skip OUTSIDE face
- Output a canonical selection

Stats: faces_processed (total faces checked), selections_canonicalized

This is the query half of the region tagging system. Together with tag.face.region, it enables the composable pipeline pattern from the basilica worked example.

## Acceptance Criteria

- Produces canonical FaceSet for a given region id
- Iterates in deterministic order
- Skips OUTSIDE face
- Works with the region layer from cam-ul4v
- Unit tests: tag some faces, select by region, verify canonical output


## Notes

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — select-by-region is how operators like shape.extrude.walls find their input faces (region.footprint).
