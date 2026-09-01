// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use setout::{Count, Offset, Rational};

use super::*;
use crate::{GenerationError, InvocationKey, ItemKey, ItemOverride};

fn invocation() -> InvocationKey {
    InvocationKey::new("fixture/arcade-bays").unwrap()
}

#[test]
fn bay_edges_and_centers_keep_non_integral_spacing_exact() {
    // Three bays across ten iotas require thirds at their edges and sixths at
    // their centers. The last edge must still land exactly on the authored
    // endpoint without accumulating a rounded pitch.
    let key = invocation();
    let fragment = distribute_linear_bays(&LinearBayDistribution {
        invocation: &key,
        start: Offset::from_iota(2),
        end: Offset::from_iota(12),
        bays: Count::new(3),
        overrides: &[],
    })
    .unwrap();

    assert_eq!(fragment.items()[0].start(), Rational::new(2, 1).unwrap());
    assert_eq!(fragment.items()[0].center(), Rational::new(11, 3).unwrap());
    assert_eq!(fragment.items()[0].end(), Rational::new(16, 3).unwrap());
    assert_eq!(fragment.items()[2].center(), Rational::new(31, 3).unwrap());
    assert_eq!(fragment.items()[2].end(), Rational::new(12, 1).unwrap());
}

#[test]
fn bay_labels_and_omissions_are_semantic() {
    // An omission targets a stable one-based bay label, while a label outside
    // the current run stays visible as unresolved intent rather than being
    // rebound to a current ordinal.
    let key = invocation();
    let omitted = ItemOverride::omit("bay/000002").unwrap();
    let missing = ItemOverride::omit("bay/000009").unwrap();
    let fragment = distribute_linear_bays(&LinearBayDistribution {
        invocation: &key,
        start: Offset::ZERO,
        end: Offset::from_iota(12),
        bays: Count::new(3),
        overrides: &[missing.clone(), omitted],
    })
    .unwrap();

    let labels: Vec<_> = fragment
        .items()
        .iter()
        .map(|bay| bay.label().as_str())
        .collect();
    assert_eq!(labels, ["bay/000001", "bay/000003"]);
    assert_eq!(fragment.orphaned_overrides(), &[missing]);
}

#[test]
fn count_growth_reports_rank_identity_and_exact_payload_changes() {
    // Retained ranks keep their keys while uniform redistribution moves every
    // old bay extent and center; only the new final rank is added.
    let key = invocation();
    let distribute = |bays| {
        distribute_linear_bays(&LinearBayDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::from_iota(12),
            bays: Count::new(bays),
            overrides: &[],
        })
        .unwrap()
    };
    let three = distribute(3);
    let four = distribute(4);
    let delta = three.delta_to(&four).unwrap();

    assert_eq!(delta.retained().len(), 3);
    assert_eq!(delta.changed().len(), 3);
    assert_eq!(
        delta.added(),
        &[ItemKey::new("fixture/arcade-bays/bay/000004").unwrap()]
    );
    assert!(delta.removed().is_empty());
}

#[test]
fn invalid_bay_shapes_fail_before_expansion() {
    // Empty topology, coincident edges, and work beyond the shared linear
    // budget are rejected rather than yielding plausible partial fragments.
    let key = invocation();
    assert_eq!(
        distribute_linear_bays(&LinearBayDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::from_iota(1),
            bays: Count::new(0),
            overrides: &[],
        }),
        Err(GenerationError::NoBays)
    );
    assert_eq!(
        distribute_linear_bays(&LinearBayDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::ZERO,
            bays: Count::new(1),
            overrides: &[],
        }),
        Err(GenerationError::CoincidentAnchors)
    );
    assert!(matches!(
        distribute_linear_bays(&LinearBayDistribution {
            invocation: &key,
            start: Offset::ZERO,
            end: Offset::from_iota(1),
            bays: Count::new(u64::from(crate::MAX_LINEAR_STATIONS)),
            overrides: &[],
        }),
        Err(GenerationError::TooManyStations { .. })
    ));
}

#[test]
fn warm_reexpansion_matches_the_fresh_bay_oracle() {
    // The convenience wrapper may retain the prior fragment only; after an
    // edit its fragment must equal a fresh pure expansion byte-for-byte.
    let key = invocation();
    let initial = LinearBayDistribution {
        invocation: &key,
        start: Offset::ZERO,
        end: Offset::from_iota(10),
        bays: Count::new(3),
        overrides: &[],
    };
    let edited = LinearBayDistribution {
        invocation: &key,
        start: Offset::from_iota(-2),
        end: Offset::from_iota(12),
        bays: Count::new(4),
        overrides: &[],
    };
    let mut warm = LinearBayGenerator::new(&initial).unwrap();
    let delta = warm.reexpand(&edited).unwrap();
    let fresh = distribute_linear_bays(&edited).unwrap();

    assert_eq!(warm.fragment(), &fresh);
    assert_eq!(delta.added().len(), 1);
    assert_eq!(delta.changed().len(), 3);
}

#[test]
fn small_exact_domain_matches_an_independent_midpoint_oracle() {
    // Exercise ascending and descending extents, negative coordinates, and
    // every small bay count against the affine midpoint equation rather than
    // the generator's own helper or floating-point lowering.
    let key = invocation();
    for start in -4_i64..=4 {
        for end in -4_i64..=4 {
            if start == end {
                continue;
            }
            for bays in 1_u32..=9 {
                let fragment = distribute_linear_bays(&LinearBayDistribution {
                    invocation: &key,
                    start: Offset::from_iota(start),
                    end: Offset::from_iota(end),
                    bays: Count::new(u64::from(bays)),
                    overrides: &[],
                })
                .unwrap();
                assert_eq!(
                    fragment.items().first().unwrap().start(),
                    Rational::new(i128::from(start), 1).unwrap()
                );
                assert_eq!(
                    fragment.items().last().unwrap().end(),
                    Rational::new(i128::from(end), 1).unwrap()
                );
                for adjacent in fragment.items().windows(2) {
                    assert_eq!(adjacent[0].end(), adjacent[1].start());
                }
                for bay in fragment.items() {
                    let midpoint_ordinal = i128::from(bay.ordinal()) * 2 + 1;
                    let denominator = u128::from(bays) * 2;
                    let numerator = i128::from(start) * i128::from(bays) * 2
                        + (i128::from(end) - i128::from(start)) * midpoint_ordinal;
                    assert_eq!(bay.center(), Rational::new(numerator, denominator).unwrap());
                }
            }
        }
    }
}

#[test]
fn exact_bay_arithmetic_covers_the_full_offset_domain() {
    // The widest signed extent and largest accepted bay count must remain in
    // checked i128 arithmetic, including the doubled midpoint denominator.
    let key = invocation();
    let bays = crate::MAX_LINEAR_STATIONS - 1;
    let fragment = distribute_linear_bays(&LinearBayDistribution {
        invocation: &key,
        start: Offset::from_iota(i64::MIN),
        end: Offset::from_iota(i64::MAX),
        bays: Count::new(u64::from(bays)),
        overrides: &[],
    })
    .unwrap();

    assert_eq!(
        fragment.items().first().unwrap().start(),
        Rational::new(i128::from(i64::MIN), 1).unwrap()
    );
    assert_eq!(
        fragment.items().last().unwrap().end(),
        Rational::new(i128::from(i64::MAX), 1).unwrap()
    );
}
