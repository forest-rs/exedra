# ADR-0021: Bounded local trimming beside fitted offset cubics

Status: accepted

## Decision

Profile offset owns local corner resolution and its derivation evidence. It does
not repair arbitrary offset topology or certify Kurbo's fitted offset error.
Line/line, line/arc and arc/arc joins keep their existing analytic construction.
When a corner overlaps and either neighbor is fitted, intersect the finite runs
and require exactly one isolated transverse intersection. Trim at the recovered
parameters, dropping consumed fitted pieces and splitting retained Beziers with
Kurbo. Preserve source tags and sampling policy. Reject consumed runs, competing
roots, tangency, coincidence and unresolved fitted-piece endpoint intersections.
The existing sampled global checks still reject collapse, crossing and contact.

Polynomial intersections use outward-rounded interval arithmetic and Krawczyk
inclusion/subdivision. Circle intersections use the implicit circle equation,
then filter against the authored arc; uncertain endpoint membership is refused.
The inclusion criterion follows [equation 10 and theorem 6, section 2.3 of the
interval-method reference](https://ww2.ii.uj.edu.pl/~wilczak/papers/logmap/logmap.pdf).
Each interval box/contraction is charged before work. A finite default budget
also applies to the legacy offset entry point. No new dependency is needed.

`OffsetPolicy` adds `trim_tolerance` in recipe units and `max_trim_steps` for the
whole operation. `OffsetResult::trims` records the source corner, local fitted
piece indices, parameters and their enclosures, common point, positional enclosure and endpoint
adjustment. Retained cuts must have strictly separated parameter enclosures;
overlap is unresolved, not evidence that a thin piece survives. Endpoints and adjacent cubic handles move together to share a join.
`OffsetMethod::Trimmed` distinguishes an analytic line/arc with a numerical trim;
a fitted run remains `Fitted`. This evidence describes the derivation, not an
exact offset certificate or continuous topology guarantee. Evidence is not
encoded into the resulting ordinary profile; there is no recipe schema change.

## Migration

Add the new policy fields to complete struct literals, or use a default tail.
Scale trim tolerance alongside other dimensional tolerances. Clearance slack
must cover `2 * check_tolerance + fit_tolerance + trim_tolerance`.
Account for `OffsetBudget::TrimSteps`, `OffsetWork::trim_steps`,
`OffsetMethod::Trimmed`, and `OffsetResult::trims` when exhaustively constructing
or matching evidence. Replace handling of `OffsetCornerUnsupported` with
`OffsetTrimUnresolved` and `OffsetTrimAmbiguous`. No root and consumed retained
runs produce `OffsetLoopDegenerate`. Legacy trim accuracy is
`abs(distance) * 1e-6`, bounded below by the smallest positive normal f64.
