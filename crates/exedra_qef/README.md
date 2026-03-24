# exedra_qef

Small, deterministic QEF solving for Exedra.

Current scope:

- accumulate plane constraints from position + normal pairs,
- solve a bounded 3x3 QEF without external linear algebra dependencies,
- support explicit low-rank anchoring so callers can bias null-space dimensions
  toward a mass point instead of always using the bounds center,
- classify the local feature as smooth, edge, or corner from solver rank,
- expose residual error for extraction-time quality checks.

This crate intentionally owns only the fitting step. It does not own Hermite
sampling, octree traversal, or mesh extraction.
