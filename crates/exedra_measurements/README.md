# exedra_measurements

`exedra_measurements` provides small, exact physical measurement values for
the Exedra workspace. A [`Length`](https://docs.rs/exedra_measurements/latest/exedra_measurements/struct.Length.html)
is a strictly positive size; an
[`Offset`](https://docs.rs/exedra_measurements/latest/exedra_measurements/struct.Offset.html)
is a signed displacement and may be zero. `Angle` is a nonnegative angular
magnitude and `AngularOffset` is its signed counterpart.

Linear values store joto iotas (one ninth of a nanometer); angular values store
microarcseconds so degrees, arcminutes, and arcseconds remain exact. Values do
not implicitly normalize or wrap, and remain exact until a geometry boundary
explicitly lowers them to meters, degrees, or radians. The crate is `no_std`
and depends only on `joto_constants`; parsing, formatting, dimensional analysis,
and geometry are deliberately outside its boundary.
