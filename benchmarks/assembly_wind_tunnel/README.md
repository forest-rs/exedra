# assembly_wind_tunnel

Wind-tunnel scenarios for assembly flattening at placement scale, following
the workspace convention: argv-selected profiles, one `key=value` summary
line per run, and a determinism assertion before any timing.

**AS-1** places one compiled part (a subdivided icosphere standing in for a
vegetation asset) thousands of times under one frame, once as individual
instances and once as a placement set, and times assembly construction and
flattening with the default transformed-box bounds and with exact bounds.
Both representations must agree on placed triangles and bounds.

```sh
cargo run --release -p assembly_wind_tunnel -- --quick
cargo run --release -p assembly_wind_tunnel -- --stress
```
