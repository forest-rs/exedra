# Measurements of evaluated planar boundaries

Status: accepted

Constructive owns area, perimeter, centroid, and local bounds for `PlanarPatch`
and `PlaneSection`. Both use one allocation-free, linear boundary traversal,
with compensated sums and per-loop translation before area/moment arithmetic.
Holes subtract area and first moments and add perimeter. Disconnected section
regions contribute to a single area-weighted centroid. Empty sections report
zero area/perimeter and absent centroid/bounds.

Measurements use the owner's orthonormal XY coordinates and body units (squared
for area). They describe stored polygonal boundaries, not reconstructed analytic
curves; section coordinates precede f32 realization of split caps. The owner
retains the frame, provenance, and existing accuracy evidence. A centroid may
lie outside material. Measurements are not a geometry validity certificate.

Patch boundaries are immutable and already checked. Publicly mutable sections
must retain simple nonoverlapping regions and correctly nested holes. Measurement
checks winding, basic degeneracy, and finite arithmetic, but does not repeat the
quadratic crossing/nesting validation. Generated inputs remain bounded by their
extraction policies; results count visited edges. No new policy, cache, or
snapshot wrapper is needed for this read-only query.

## Migration

Additive `measure()` methods and `measure` result/error types; no caller changes,
recipe serialization changes, or evaluation-schema changes are required.
