# constructive_wind_tunnel

Wind-tunnel scenarios for the constructive geometry head, following the
workspace convention: argv-selected profiles, one `key=value` summary line
per run, and a signature-determinism assertion *before* any timing — the
wind tunnel is a determinism regression oracle first and a perf harness
second.

**CT-1** compiles and evaluates a batch of parameterized rounded-profile
extrusion recipes (arcs exercise the libm discretization path), asserts
bit-identical trimesh signatures across two full passes, exercises
source-map forward/reverse lookups at scale, and reports timing plus
introspection counters.

**CT-2** compares a cold grouped-recipe rebuild with a one-node edit through
the evaluation cache, while pinning the warm result against a full rebuild.

**CT-3** mirrors both gallery drill paths: the public constructive CSG card and
the direct 16-sided mesh Boolean used by the rounded-drill export. It isolates
constructive evaluation, direct Boolean, sharp-edge rounding, and render
extraction; deep-validates and signature-checks every result before timing;
then reports best and average phase times with Boolean/rounding work counters.

**CT-4** compares constructive stretch's algebraic box and extrusion paths
against its closed imported-mesh fallback. It validates and signature-checks
each output before reporting the paths separately, so tessellation-policy or
topology regressions cannot hide inside an aggregate number. The stress
profile uses a five-digit-vertex closed prism to expose work that scales with
the whole source mesh per output corner.

```sh
cargo run --release -p constructive_wind_tunnel -- --quick
cargo run --release -p constructive_wind_tunnel -- --ct1-stress
cargo run --release -p constructive_wind_tunnel -- --gallery
cargo run --release -p constructive_wind_tunnel -- --gallery-stress
cargo run --release -p constructive_wind_tunnel -- --gallery-sample
cargo run --release -p constructive_wind_tunnel -- --stretch
cargo run --release -p constructive_wind_tunnel -- --stretch-stress
```

`--gallery-sample` repeats the unchanged CT-3 workload long enough for an
external sampling profiler to attach; it is not a distinct benchmark case.

## License

Apache-2.0 OR MIT
