# constructive_gallery

The spearhead gallery: six anonymous, spec-agnostic shapes built through
`exedra_constructive`'s *public surface only*, exactly the way an external
spec compiler would. It stands in for those (out-of-tree) frontends: if a
scenario here needs a private API or a workaround, the integration surface is
wrong and must be fixed in the library, never here.

Scenarios: a rectangular prism, a concave L-prism, a rounded profile with
true arcs, a holed ring profile, a partial revolve, and a transformed CSG
difference (which reports the structured `eval.csg.unsupported` diagnostic
until the mesh boolean pipeline lands).

Run the binary for a one-line-per-scenario summary with deterministic
signatures:

```sh
cargo run -p constructive_gallery
```

## License

Apache-2.0 OR MIT
