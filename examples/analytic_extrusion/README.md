# Analytic extrusion

One plate with a round hole, described by hand as an `exedra::profile::Profile`
and through constructive builders with `Profile2::to_line_arc`. Both routes give
the same profile, so the same exact `Extrusion` and a bit-identical realized
mesh. The demo:

- reads exact native measures without a mesh;
- realizes the solid, with the deviation measured on the emitted coordinates;
- compiles the mesh as baked geometry while keeping the realization report;
- refuses a tolerance the per-arc budget cannot meet, then raises the budget to
  the count the refusal names. The samples then meet the tolerance, but `f32`
  narrowing at this scale does not, and the report says so;
- names the segments a hole touches when it is moved through the outline.

```sh
cargo run -p analytic_extrusion
```
