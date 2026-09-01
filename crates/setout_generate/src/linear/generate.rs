// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pure expansion and canonical fragment fingerprinting.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::vec::Vec;

use setout::{ArithmeticError, CanonicalEncoder, Fingerprint, Offset, Rational};

use crate::key::{ItemKey, ItemLabel};

use super::error::GenerationError;
use super::item_override::ItemOverride;
use super::types::{LinearDistribution, LinearFragment, LinearStation};
use super::{GENERATION_SCHEMA_VERSION, MAX_LINEAR_STATIONS};

/// Expands endpoint-inclusive stations using exact rational coordinates.
///
/// The first and final labels are `start` and `end`. Interior labels use their
/// one-based rank, for example `interior/000001`. Count changes therefore never
/// turn the stable endpoint identity into an interior item. Exact coordinates
/// are computed directly from the anchors; no rounded step is accumulated.
///
/// Overrides are normalized by label before fingerprinting. An override that
/// has no item in this expansion is retained in
/// [`LinearFragment::orphaned_overrides`] instead of being rebound by ordinal.
///
/// # Errors
///
/// Returns [`GenerationError`] for zero or excessive interval counts,
/// coincident anchors, duplicate override targets, or exact arithmetic failure.
pub fn distribute_linear(spec: &LinearDistribution<'_>) -> Result<LinearFragment, GenerationError> {
    let intervals = spec.intervals.get();
    if intervals == 0 {
        return Err(GenerationError::NoIntervals);
    }
    let station_count = intervals
        .checked_add(1)
        .ok_or(GenerationError::TooManyStations {
            requested: u64::MAX,
            limit: MAX_LINEAR_STATIONS,
        })?;
    if station_count > u64::from(MAX_LINEAR_STATIONS) {
        return Err(GenerationError::TooManyStations {
            requested: station_count,
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
    let intervals = u32::try_from(intervals).expect("bounded interval count fits u32");

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

    let known_labels: BTreeSet<ItemLabel> = (0..=intervals)
        .map(|ordinal| station_label(ordinal, intervals))
        .collect();
    let mut items = Vec::with_capacity(known_labels.len().saturating_sub(overrides.len()));
    for ordinal in 0..=intervals {
        let label = station_label(ordinal, intervals);
        if overrides.contains_key(&label) {
            continue;
        }
        items.push(LinearStation {
            key: ItemKey::within(spec.invocation, &label),
            label,
            ordinal,
            position: station_position(spec.start, spec.end, ordinal, intervals)?,
        });
    }
    let orphaned_overrides: Vec<_> = overrides
        .values()
        .filter(|item_override| !known_labels.contains(item_override.target()))
        .cloned()
        .collect();
    let items = items.into_boxed_slice();
    let orphaned_overrides = orphaned_overrides.into_boxed_slice();
    let fingerprint = fragment_fingerprint(
        spec,
        intervals,
        overrides.values(),
        &items,
        &orphaned_overrides,
    );

    Ok(LinearFragment {
        invocation: spec.invocation.clone(),
        start: spec.start,
        end: spec.end,
        intervals,
        items,
        orphaned_overrides,
        fingerprint,
    })
}

fn station_label(ordinal: u32, intervals: u32) -> ItemLabel {
    let value = if ordinal == 0 {
        "start".into()
    } else if ordinal == intervals {
        "end".into()
    } else {
        format!("interior/{ordinal:06}").into_boxed_str()
    };
    ItemLabel::new(value).expect("generator-owned station labels are valid")
}

pub(super) fn station_position(
    start: Offset,
    end: Offset,
    ordinal: u32,
    intervals: u32,
) -> Result<Rational, ArithmeticError> {
    // Form the affine numerator in i128 instead of rounding an iota-sized
    // step. The largest accepted product is comfortably inside i128 because
    // both anchors are i64 and intervals are bounded to u32.
    let denominator = i128::from(intervals);
    let start_numerator = i128::from(start.iota()) * denominator;
    let delta = i128::from(end.iota()) - i128::from(start.iota());
    let numerator = start_numerator + delta * i128::from(ordinal);
    Rational::new(numerator, intervals.into())
}

fn fragment_fingerprint<'a>(
    spec: &LinearDistribution<'_>,
    intervals: u32,
    overrides: impl Iterator<Item = &'a ItemOverride>,
    items: &[LinearStation],
    orphaned: &[ItemOverride],
) -> Fingerprint {
    let mut encoder = CanonicalEncoder::new("setout_generate/linear-fragment");
    encoder.u32(GENERATION_SCHEMA_VERSION);
    encoder.str(spec.invocation.as_str());
    encoder.i64(spec.start.iota());
    encoder.i64(spec.end.iota());
    encoder.u32(intervals);
    let overrides: Vec<_> = overrides.collect();
    encoder.u32(u32::try_from(overrides.len()).expect("override count is station-bounded"));
    for item_override in overrides {
        encoder.str(item_override.target().as_str());
        encoder.u8(0); // Version-one override action: omit.
    }
    encoder.u32(u32::try_from(items.len()).expect("item count is station-bounded"));
    for item in items {
        encoder.str(item.key().as_str());
        encoder.u32(item.ordinal());
        encoder.i128(item.position().numerator());
        encoder.u128(item.position().denominator());
    }
    encoder.u32(u32::try_from(orphaned.len()).expect("orphan count is station-bounded"));
    for item_override in orphaned {
        encoder.str(item_override.target().as_str());
    }
    encoder.finish()
}
