// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pure linear-bay expansion and canonical fingerprinting.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::vec::Vec;

use setout::{ArithmeticError, CanonicalEncoder, Fingerprint, Offset, Rational};

use crate::key::{ItemKey, ItemLabel};

use super::super::error::GenerationError;
use super::super::generate::station_position;
use super::super::item_override::ItemOverride;
use super::super::{GENERATION_SCHEMA_VERSION, MAX_LINEAR_STATIONS};
use super::types::{LinearBay, LinearBayDistribution, LinearBayFragment};

/// Expands equal edge-to-edge bays using exact rational coordinates.
///
/// Bay labels use their one-based rank, for example `bay/000001`. Exact edges
/// and centers are calculated directly from the outer anchors; no rounded
/// pitch is accumulated. An omitted label remains absent while an unavailable
/// target is retained in [`LinearBayFragment::orphaned_overrides`].
///
/// # Errors
///
/// Returns [`GenerationError`] for zero or excessive bay counts, coincident
/// anchors, duplicate override targets, or exact arithmetic failure.
pub fn distribute_linear_bays(
    spec: &LinearBayDistribution<'_>,
) -> Result<LinearBayFragment, GenerationError> {
    let bays = spec.bays.get();
    if bays == 0 {
        return Err(GenerationError::NoBays);
    }
    let boundary_count = bays
        .checked_add(1)
        .ok_or(GenerationError::TooManyStations {
            requested: u64::MAX,
            limit: MAX_LINEAR_STATIONS,
        })?;
    if boundary_count > u64::from(MAX_LINEAR_STATIONS) {
        return Err(GenerationError::TooManyStations {
            requested: boundary_count,
            limit: MAX_LINEAR_STATIONS,
        });
    }
    if spec.start == spec.end {
        return Err(GenerationError::CoincidentAnchors);
    }
    if spec.overrides.len() > MAX_LINEAR_STATIONS as usize {
        return Err(GenerationError::TooManyOverrides {
            requested: spec.overrides.len(),
            limit: MAX_LINEAR_STATIONS,
        });
    }
    let bays = u32::try_from(bays).expect("bounded bay count fits u32");

    let mut overrides = BTreeMap::new();
    for item_override in spec.overrides {
        if overrides
            .insert(item_override.target().clone(), item_override.clone())
            .is_some()
        {
            return Err(GenerationError::DuplicateOverride(
                item_override.target().clone(),
            ));
        }
    }

    let known_labels: BTreeSet<_> = (0..bays).map(bay_label).collect();
    let mut items = Vec::with_capacity(known_labels.len().saturating_sub(overrides.len()));
    for ordinal in 0..bays {
        let label = bay_label(ordinal);
        if overrides.contains_key(&label) {
            continue;
        }
        items.push(LinearBay {
            key: ItemKey::within(spec.invocation, &label),
            label,
            ordinal,
            start: station_position(spec.start, spec.end, ordinal, bays)?,
            end: station_position(spec.start, spec.end, ordinal + 1, bays)?,
            center: bay_center(spec.start, spec.end, ordinal, bays)?,
        });
    }
    let orphaned_overrides: Vec<_> = overrides
        .values()
        .filter(|item_override| !known_labels.contains(item_override.target()))
        .cloned()
        .collect();
    let items = items.into_boxed_slice();
    let orphaned_overrides = orphaned_overrides.into_boxed_slice();
    let fingerprint =
        fragment_fingerprint(spec, bays, overrides.values(), &items, &orphaned_overrides);

    Ok(LinearBayFragment {
        invocation: spec.invocation.clone(),
        start: spec.start,
        end: spec.end,
        bays,
        items,
        orphaned_overrides,
        fingerprint,
    })
}

fn bay_label(ordinal: u32) -> ItemLabel {
    ItemLabel::new(format!("bay/{:06}", ordinal + 1)).expect("generator-owned bay labels are valid")
}

fn bay_center(
    start: Offset,
    end: Offset,
    ordinal: u32,
    bays: u32,
) -> Result<Rational, ArithmeticError> {
    // A bay midpoint lies at (2 * ordinal + 1) / (2 * bays). Forming the
    // affine numerator directly retains half-iota centers and avoids adding
    // two rationals with a wider intermediate denominator.
    let denominator = i128::from(bays) * 2;
    let start_numerator = i128::from(start.iota()) * denominator;
    let delta = i128::from(end.iota()) - i128::from(start.iota());
    let midpoint_ordinal = i128::from(ordinal) * 2 + 1;
    let numerator = start_numerator + delta * midpoint_ordinal;
    Rational::new(
        numerator,
        u128::try_from(denominator).expect("positive u32 bay count doubled fits u128"),
    )
}

fn fragment_fingerprint<'a>(
    spec: &LinearBayDistribution<'_>,
    bays: u32,
    overrides: impl Iterator<Item = &'a ItemOverride>,
    items: &[LinearBay],
    orphaned: &[ItemOverride],
) -> Fingerprint {
    let mut encoder = CanonicalEncoder::new("setout_generate/linear-bay-fragment");
    encoder.u32(GENERATION_SCHEMA_VERSION);
    encoder.str(spec.invocation.as_str());
    encoder.i64(spec.start.iota());
    encoder.i64(spec.end.iota());
    encoder.u32(bays);
    let overrides: Vec<_> = overrides.collect();
    encoder.u32(u32::try_from(overrides.len()).expect("override count is bay-bounded"));
    for item_override in overrides {
        encoder.str(item_override.target().as_str());
        encoder.u8(0); // Version-one override action: omit.
    }
    encoder.u32(u32::try_from(items.len()).expect("item count is bay-bounded"));
    for item in items {
        encoder.str(item.key().as_str());
        encoder.u32(item.ordinal());
        for coordinate in [item.start(), item.end(), item.center()] {
            encoder.i128(coordinate.numerator());
            encoder.u128(coordinate.denominator());
        }
    }
    encoder.u32(u32::try_from(orphaned.len()).expect("orphan count is bay-bounded"));
    for item_override in orphaned {
        encoder.str(item_override.target().as_str());
    }
    encoder.finish()
}
