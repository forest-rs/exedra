# setout_joiner

`setout_joiner` is the narrow adapter between exact setting-out and Joiner's
construction graph. It resolves exact points and dimensions once, constructs
deterministic analytic extents with `exedra_math`, retains claim/support links,
and maps quantity deltas to Joiner's geometry, contact, and load-path channels.

Positive dimensions arrive as `exedra_measurements::Length`; point coordinates
arrive as signed `Offset`; subdivided generated stations arrive as rational
iota counts. This crate lowers them to meters at the construction boundary,
keeping the shared `joto_constants` scale and conversion order out of setout
propagation, architecture modules, and construction rules.

Neither side depends on the other through this crate: `setout` remains consumer
neutral, and `joiner` remains independently usable.
