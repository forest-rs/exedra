# `exedra_measurements`

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

```rust
use exedra_measurements::{Angle, Length, Offset};

let width = Length::millimeters(120).expect("positive length");
let setback = Offset::millimeters(-25).expect("signed offset");
let bevel = Angle::degrees(45).expect("nonnegative angle");

assert_eq!(width.as_meters(), 0.12);
assert_eq!(setback.as_meters(), -0.025);
assert_eq!(bevel.as_degrees(), 45.0);
```

Use `Length` and `Angle` for magnitudes. Use `Offset` and `AngularOffset`
whenever zero or a negative value is meaningful. Constructors return `None`
when a value cannot be represented exactly or violates that distinction.

## License

Apache-2.0 OR MIT
