// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lowering adapter from exact generated bays to profile coordinates.

use setout_generate::LinearBayFragment;
use setout_joiner::lower_rational_iotas;

/// Lowers exact bay centers into the local meter coordinates of one profile.
///
/// Basilica creates each bay fragment in its consuming wall segment's local
/// frame. This is the sole floating-point boundary: profile construction sees
/// the center coordinates but cannot recalculate count, pitch, or margins.
pub(super) fn local_bay_centers(bays: &LinearBayFragment) -> impl Iterator<Item = f64> + '_ {
    bays.items()
        .iter()
        .map(|bay| lower_rational_iotas(bay.center()))
}
