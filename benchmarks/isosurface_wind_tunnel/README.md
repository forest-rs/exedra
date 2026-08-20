# isosurface_wind_tunnel

Deterministic correctness and measurement witness for error-driven adaptive
dual contouring. The H1 scenario compares public adaptive extraction with an
independently implemented finest-grid dual-contour reference, then measures
both directions of a finite sampled surface deviation against exact visible
axis-aligned box-union patches.

The reported deviations are deterministic finite sampling oracles, not a
formal Hausdorff proof. Extraction signatures and correctness gates run before
timing. Timing is never included in artifact reports.

```sh
cargo test -p isosurface_wind_tunnel
cargo run --release -p isosurface_wind_tunnel -- --quick
cargo run --release -p isosurface_wind_tunnel -- --gate --write-artifacts
```

Artifacts are written below
`target/isosurface_wind_tunnel/{quick,gate}/`.

`--quick` checks deterministic extraction and topology at depth 5, but does
not report a reduction ratio because the authoritative private comparator pin
is depth 7. `--gate` first requires exact ordered signature, statistics,
regions, counters, and leaf-histogram parity with that private pin. Only then
does it run the bidirectional finite sampled-deviation oracle and report the
current reduction result. A failed 10x reduction is reported as data; this
executable does not tune thresholds or change production retention behavior.

`uniform.work.lattice_bytes` is the logical byte size of the independent
comparator's dense `(2^depth + 1)^3` `f32` scalar lattice. It excludes `Vec`
spare capacity, the output mesh, interval-tree bookkeeping, Hermite records,
and QEF scratch space, so it is not peak resident memory.

The gate artifacts are timing-free. Reproduce and compare their hashes with:

```sh
cargo run --release -p isosurface_wind_tunnel -- --gate --write-artifacts
shasum -a 256 \
  target/isosurface_wind_tunnel/gate/report.txt \
  target/isosurface_wind_tunnel/gate/uniform.obj \
  target/isosurface_wind_tunnel/gate/adaptive.obj \
  target/isosurface_wind_tunnel/gate/hard-box-cylinder.obj

# Run both commands again; all four hashes must be unchanged.
```

## License

Apache-2.0 OR MIT
