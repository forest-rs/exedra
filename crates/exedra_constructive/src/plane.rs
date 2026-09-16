// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared, consistently ordered plane/edge intersection arithmetic.

/// Intersects a straddling edge in ascending vertex-ID order. Returns the
/// point and interpolation parameter measured from the caller's first end.
pub(crate) fn intersect_edge(
    a: (u32, [f64; 3], f64),
    b: (u32, [f64; 3], f64),
) -> Option<([f64; 3], f64)> {
    let (low, high) = if a.0 < b.0 { (a, b) } else { (b, a) };
    let denominator = low.2 - high.2;
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let t = low.2 / denominator;
    let point = core::array::from_fn(|i| low.1[i] + t * (high.1[i] - low.1[i]));
    Some((point, if a.0 < b.0 { t } else { 1.0 - t }))
}
