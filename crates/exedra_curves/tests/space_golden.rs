// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Space curves give the same bits as before NURBS became generic over
//! dimension.

use exedra_curves::{Nurbs, Tolerance};

/// A weighted cubic leaving the plane, with a double knot and a corner.
fn curve() -> ([[f64; 3]; 9], [f64; 13], [f64; 9]) {
    let points = [
        [0.0, 0.0, 0.25],
        [1.0, 2.0, -0.5],
        [2.5, 1.5, 1.0],
        [3.0, -1.0, 0.75],
        [4.5, 0.5, -1.25],
        [5.0, 2.5, 0.5],
        [6.5, 1.0, 2.0],
        [7.0, -0.5, 0.0],
        [8.0, 1.0, -0.75],
    ];
    let knots = [
        0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 3.5, 3.5, 3.5, 5.0, 5.0, 5.0,
    ];
    let weights = [1.0, 0.75, 2.0, 1.25, 0.5, 1.5, 3.0, 0.8, 1.0];
    (points, knots, weights)
}

/// FNV-1a over every value's bits, so signed zeros count.
fn digest(values: impl IntoIterator<Item = f64>) -> u64 {
    values
        .into_iter()
        .fold(0xCBF2_9CE4_8422_2325, |hash, value| {
            value.to_bits().to_le_bytes().iter().fold(hash, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
            })
        })
}

#[test]
fn space_curves_keep_their_bits() {
    let (points, knots, weights) = curve();
    let valid = Nurbs {
        order: 4,
        knots: &knots,
        points: &points,
        weights: Some(&weights),
    }
    .validate()
    .unwrap();
    let prepared = valid.prepare();
    let mut values = Vec::new();
    for k in 0..=100 {
        let t = 5.0 * f64::from(k) / 100.0;
        for e in [valid.evaluation(t), prepared.evaluation(t)] {
            values.extend(e.point);
            values.extend(e.first);
            values.extend(e.second);
        }
        values.extend(prepared.evaluate(t));
    }
    let samples = valid.sample(Some([0.5, 4.5]), 7).unwrap();
    values.extend(samples.params);
    values.extend(samples.points.into_iter().flatten());
    let (adaptive, _) = prepared.adaptive_sample(
        Tolerance {
            chord: 1e-3,
            angle: 0.2,
        },
        4000,
    );
    values.extend(adaptive.params);
    values.extend(adaptive.points.into_iter().flatten());
    let lengths = prepared.lengths(8);
    values.extend(lengths.iter().copied());
    let total = *lengths.last().unwrap();
    values.extend(
        (0..=10).map(|k| prepared.parameter_at_length(&lengths, 8, total * f64::from(k) / 10.0)),
    );
    // Recorded from the 3D-only implementation.
    assert_eq!(digest(values), 16_830_906_767_569_059_661, "bits changed");
}
