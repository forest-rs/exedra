# setout_joiner

`setout_joiner` is the narrow adapter between exact setting-out and Joiner's
construction graph. It resolves exact points and dimensions once, constructs
deterministic analytic extents with `exedra_math`, retains claim/support links,
and maps quantity deltas to Joiner's geometry, contact, and load-path channels.

Neither side depends on the other through this crate: `setout` remains consumer
neutral, and `joiner` remains independently usable.
