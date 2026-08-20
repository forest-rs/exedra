# constructive_gallery

The reference gallery builds anonymous shapes through
`exedra_constructive`'s *public surface only*, as an external geometry frontend
would. If a scenario needs a private API or a workaround, the integration
surface is wrong and must be fixed in the library rather than in the example.

Scenarios: a rectangular prism, a concave L-prism, a rounded profile with
true arcs, a holed ring profile, a partial revolve, and a transformed CSG
difference evaluated through the mesh Boolean pipeline. Configurations refused
by that pipeline retain a structured `eval.csg.unsupported` diagnostic and an
envelope-only result.

Run the binary for a one-line-per-scenario summary with deterministic
signatures:

```sh
cargo run -p constructive_gallery
```

## License

Apache-2.0 OR MIT
