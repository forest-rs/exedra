# Addressable assembly tour

This example uses the Addressable API implemented directly by the real Basilica
`Assembly`. It queries the instance forest by stable part identity, proves exact
and relative resolution agree, pins the result, consumes an `InstanceId` only
through a revision-scoped handle, explains the dome's effective `surface`
material, and retains the provenance behind that effective value.

Run it with:

```sh
cargo run -p addressable_assembly
```

The library tests also cover root-child traversal, locator and pin round trips,
and stale handle and pin rejection after committed authoring.
