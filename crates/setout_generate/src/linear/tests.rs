// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use setout::{Count, Offset, Rational};

use super::generate::station_position;
use super::*;
use crate::{InvocationKey, ItemLabel};

fn invocation() -> InvocationKey {
    InvocationKey::new("fixture/frames").unwrap()
}

#[test]
fn stations_keep_non_integral_spacing_exact() {
    // Ten iotas divided into three intervals cannot be represented as an
    // Offset step. Each station instead retains its reduced rational value
    // and the endpoint lands exactly, without accumulated rounding.
    let key = invocation();
    let fragment = distribute_linear(&LinearDistribution {
        invocation: &key,
        start: Offset::from_iota(2),
        end: Offset::from_iota(12),
        intervals: Count::new(3),
        overrides: &[],
    })
    .unwrap();

    let positions: alloc::vec::Vec<_> = fragment
        .items()
        .iter()
        .map(LinearStation::position)
        .collect();
    assert_eq!(positions[0], Rational::new(2, 1).unwrap());
    assert_eq!(positions[1], Rational::new(16, 3).unwrap());
    assert_eq!(positions[2], Rational::new(26, 3).unwrap());
    assert_eq!(positions[3], Rational::new(12, 1).unwrap());
}

#[test]
fn overrides_are_semantic_and_unknown_targets_remain_visible() {
    // Reordering overrides cannot change output identity or fingerprints;
    // a missing target stays an orphan instead of suppressing a station at
    // the same numeric position.
    let key = invocation();
    let interior = ItemOverride::omit("interior/000002").unwrap();
    let missing = ItemOverride::omit("interior/000009").unwrap();
    let first = distribute_linear(&LinearDistribution {
        invocation: &key,
        start: Offset::ZERO,
        end: Offset::from_iota(40),
        intervals: Count::new(4),
        overrides: &[interior.clone(), missing.clone()],
    })
    .unwrap();
    let second = distribute_linear(&LinearDistribution {
        invocation: &key,
        start: Offset::ZERO,
        end: Offset::from_iota(40),
        intervals: Count::new(4),
        overrides: &[missing.clone(), interior],
    })
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(first.orphaned_overrides(), &[missing]);
    assert!(
        !first
            .items()
            .iter()
            .any(|item| item.label().as_str() == "interior/000002")
    );
}

#[test]
fn invalid_shapes_fail_before_expansion() {
    // Work bounds, degenerate geometry, and ambiguous duplicate edits are
    // rejected explicitly rather than producing plausible partial output.
    let key = invocation();
    let duplicate = ItemOverride::omit("start").unwrap();
    assert_eq!(
        distribute_linear(&LinearDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::from_iota(1),
            intervals: Count::new(0),
            overrides: &[],
        }),
        Err(GenerationError::NoIntervals)
    );
    assert!(matches!(
        distribute_linear(&LinearDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::from_iota(1),
            intervals: Count::new(u64::from(MAX_LINEAR_STATIONS)),
            overrides: &[],
        }),
        Err(GenerationError::TooManyStations { .. })
    ));
    assert_eq!(
        distribute_linear(&LinearDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::ZERO,
            intervals: Count::new(1),
            overrides: &[],
        }),
        Err(GenerationError::CoincidentAnchors)
    );
    assert_eq!(
        distribute_linear(&LinearDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::from_iota(1),
            intervals: Count::new(1),
            overrides: &[duplicate.clone(), duplicate],
        }),
        Err(GenerationError::DuplicateOverride(
            ItemLabel::new("start").unwrap()
        ))
    );
}

#[test]
fn warm_reexpansion_is_the_same_fragment_as_fresh_generation() {
    // The stateful wrapper retains only the old immutable result. Its new
    // fragment must therefore equal the pure fresh oracle byte-for-byte.
    let key = invocation();
    let initial = LinearDistribution {
        invocation: &key,
        start: Offset::ZERO,
        end: Offset::from_iota(40),
        intervals: Count::new(4),
        overrides: &[],
    };
    let edited = LinearDistribution {
        invocation: &key,
        start: Offset::ZERO,
        end: Offset::from_iota(60),
        intervals: Count::new(5),
        overrides: &[],
    };
    let mut warm = LinearGenerator::new(&initial).unwrap();
    let delta = warm.reexpand(&edited).unwrap();
    let fresh = distribute_linear(&edited).unwrap();

    assert_eq!(warm.fragment(), &fresh);
    assert!(!delta.added().is_empty());
    assert!(!delta.changed().is_empty());
}

#[test]
fn small_exact_domain_matches_the_affine_rational_oracle() {
    // Exercise ascending and descending intervals, negative coordinates,
    // and every small interval count against the equation independently
    // of the generator's item storage and fingerprinting.
    let key = invocation();
    for start in -4_i64..=4 {
        for end in -4_i64..=4 {
            if start == end {
                continue;
            }
            for intervals in 1_u32..=9 {
                let fragment = distribute_linear(&LinearDistribution {
                    invocation: &key,
                    start: Offset::from_iota(start),
                    end: Offset::from_iota(end),
                    intervals: Count::new(u64::from(intervals)),
                    overrides: &[],
                })
                .unwrap();
                assert_eq!(fragment.items().len(), intervals as usize + 1);
                for station in fragment.items() {
                    let ordinal = i128::from(station.ordinal());
                    let numerator = i128::from(start) * i128::from(intervals)
                        + (i128::from(end) - i128::from(start)) * ordinal;
                    assert_eq!(
                        station.position(),
                        Rational::new(numerator, intervals.into()).unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn exact_position_arithmetic_covers_the_full_offset_domain() {
    // Interpolation between the widest signed endpoints must stay in i128
    // and land on both exact anchors without routing through overflowing
    // i64 subtraction.
    let intervals = MAX_LINEAR_STATIONS - 1;
    assert_eq!(
        station_position(
            Offset::from_iota(i64::MIN),
            Offset::from_iota(i64::MAX),
            0,
            intervals,
        )
        .unwrap(),
        Rational::new(i128::from(i64::MIN), 1).unwrap()
    );
    assert_eq!(
        station_position(
            Offset::from_iota(i64::MIN),
            Offset::from_iota(i64::MAX),
            intervals,
            intervals,
        )
        .unwrap(),
        Rational::new(i128::from(i64::MAX), 1).unwrap()
    );
}

#[test]
fn complete_omission_is_valid_but_override_work_is_bounded() {
    // A deliberately empty fragment is meaningful. The override list is
    // still work, however, and cannot bypass the same bounded-expansion
    // contract merely by naming absent targets.
    let key = invocation();
    let overrides = [
        ItemOverride::omit("start").unwrap(),
        ItemOverride::omit("interior/000001").unwrap(),
        ItemOverride::omit("end").unwrap(),
    ];
    let empty = distribute_linear(&LinearDistribution {
        invocation: &key,
        start: Offset::ZERO,
        end: Offset::from_iota(2),
        intervals: Count::new(2),
        overrides: &overrides,
    })
    .unwrap();
    assert!(empty.items().is_empty());
    assert!(empty.orphaned_overrides().is_empty());

    let excessive = alloc::vec![
        ItemOverride::omit("missing").unwrap();
        MAX_LINEAR_STATIONS as usize + 1
    ];
    assert!(matches!(
        distribute_linear(&LinearDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::from_iota(1),
            intervals: Count::new(1),
            overrides: &excessive,
        }),
        Err(GenerationError::TooManyOverrides { .. })
    ));
}
