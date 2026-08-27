# Addressable assembly tour

This example uses the Addressable API implemented directly by the real Basilica
`Assembly`. It queries the instance forest by stable part identity, proves exact
and relative resolution agree, pins the result, consumes an `InstanceId` only
through a revision-scoped handle, explains the dome's effective `surface`
material, and performs a guarded dry run followed by an atomic apply.

Run it with:

```sh
cargo run -p addressable_assembly
```

The library tests also cover root-child traversal, locator and pin round trips,
stale handle rejection, and all-or-nothing failure of a multi-target edit.
