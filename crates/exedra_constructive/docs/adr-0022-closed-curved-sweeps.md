# ADR-0022: Authored closed planar curve sweeps

Status: accepted

## Decision

Extend the existing `Path3::Curves` with a profile datum (`section_origin`),
`PathClosure`, and `PathJoin`. Profiles, curve segments and sweep nodes remain
ordinary construction inputs. This continues [ADR-0020](adr-0020-closed-planar-sweeps.md)
from closed polylines to lines, arcs and cubics, including authored sharp corners.
There is no separate surround operation or closed spatial frame-closure solver.

Closure is authored. A curved path explicitly terminates at its exact `start`;
there is no inferred closing segment or fuzzy endpoint repair. Full-turn arcs
already reproduce their exact initial point/tangent. Other arc chains whose
computed endpoints do not close exactly are refused; callers can choose a seam
on an explicit line/cubic endpoint. Closed paths require `CapMode::None`.
Validate authored cubic controls/endpoints and circular axes against the declared
plane at f64 rounding scale, before sampling. Checking sampled stations alone
would permit an off-plane curve to hide between them.

`PathJoin::Smooth` retains the existing analytic tangent continuity requirement.
`Miter { limit }` permits authored discontinuities, using the same bounded
bisector-plane cuts as controlled polylines, including the closing corner.
Sampling stations have incoming and outgoing analytic tangents. No fictitious
zero-length span represents a corner. `PathSampling` retains closure, join policy,
and explicit corner records separately from the chord/tangent bounds on spans.
Only authored corners contribute ring creases; curve subdivision does not.

Open spans retain double-reflection frame transport with shortest rotation at
sharp joins. Closed planar frames reconstruct the authored initial roll against
the plane normal independently at each station, so a final twist cannot hide
accumulated transport error. The plane normal's sign does not change the authored
roll. The duplicate closing station is used for checks; emission shares the first
ring. Apply the profile datum through the same frame, including miter stretch.

Existing span/f32 winding checks and work limits apply to every closing band.
They do not certify distant self-intersections or unsampled swept interiors.
A wide section may require tighter tangent sampling even when its centerline
chord tolerance is satisfied. Original sampling/corner evidence survives Boolean
composition as source evidence, not as validity checks on the resulting mesh.

## Migration

- Add `section_origin: [0.0; 2]`, `closure: PathClosure::Open`, and
  `joins: PathJoin::Smooth` to existing `Path3::Curves` constructions for the
  existing open smooth behavior.
- Add those three arguments after `section_x` to `tessellate_curved_sweep`.
  Add `closure` and `joins` before the policy in `discretize_path`.
- `PathClosure` is owned by `path` and remains reexported from `ir`.
  `PathStation` adds `incoming_tangent`; `PathSampling` adds `closure`, `joins`
  and `corners`. New sampling failures report invalid joins/planes, nonplanarity
  and nonclosing endpoints explicitly.
- Text and interchange write `curved_path_sweep`, preventing older readers from
  silently ignoring the added geometry semantics. Legacy `curved_sweep` payloads
  still read as open smooth paths with a zero datum.
- Evaluation schema 38 deliberately invalidates cached fingerprints and stamped
  text. Frozen wire-format inputs remain unchanged; fingerprint goldens update.

No dependency or unsafe change is needed.
